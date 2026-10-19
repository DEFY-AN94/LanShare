//! Logging for the desktop app and the headless runner: stdout, plus a
//! size-capped file in `<data dir>/logs` once the data directory is known.
//!
//! Privacy rule for every log line in LanShare: never message text, file or
//! folder names, or paths (paths carry names). Say what happened and why,
//! not where. `core::tests::logs_never_contain_message_text_or_file_names`
//! checks the main request paths with marker strings.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Registry, fmt, reload};

pub const FILE_NAME: &str = "lanshare.log";
/// The current file is rotated at this size. With `KEEP` older files the
/// logs stay under about 10 MB.
const MAX_BYTES: u64 = 2 * 1024 * 1024;
const KEEP: usize = 4;

struct LogFile {
    dir: PathBuf,
    file: File,
    size: u64,
}

impl LogFile {
    fn open(dir: &Path) -> io::Result<LogFile> {
        std::fs::create_dir_all(dir)?;
        let file = OpenOptions::new().create(true).append(true).open(dir.join(FILE_NAME))?;
        let size = file.metadata()?.len();
        Ok(LogFile { dir: dir.to_path_buf(), file, size })
    }
}

fn rotated(dir: &Path, n: usize) -> PathBuf {
    dir.join(format!("lanshare.{n}.log"))
}

static FILE: Mutex<Option<LogFile>> = Mutex::new(None);
static FILTER: OnceLock<reload::Handle<EnvFilter, Registry>> = OnceLock::new();
/// Verbose (debug-level) logging is on by default only in debug builds, and
/// the switch in Settings resets when LanShare restarts.
static VERBOSE: AtomicBool = AtomicBool::new(cfg!(debug_assertions));

fn append(slot: &mut Option<LogFile>, buf: &[u8]) -> io::Result<()> {
    let Some(log) = slot.as_mut() else { return Ok(()) };
    if log.size > 0 && log.size + buf.len() as u64 > MAX_BYTES {
        let dir = log.dir.clone();
        // Close the file first: Windows cannot rename a file that is open.
        *slot = None;
        // Move the current file aside first, so that if another program holds
        // it, nothing older is shifted or lost.
        let staging = dir.join("lanshare.rotating.log");
        let _ = std::fs::remove_file(&staging);
        if std::fs::rename(dir.join(FILE_NAME), &staging).is_ok() {
            let _ = std::fs::remove_file(rotated(&dir, KEEP));
            for n in (1..KEEP).rev() {
                let _ = std::fs::rename(rotated(&dir, n), rotated(&dir, n + 1));
            }
            let _ = std::fs::rename(&staging, rotated(&dir, 1));
            *slot = Some(LogFile::open(&dir)?);
        } else {
            // Keep writing to the same file and try again after another MAX_BYTES.
            let mut same = LogFile::open(&dir)?;
            same.size = 0;
            *slot = Some(same);
        }
    }
    if let Some(log) = slot.as_mut() {
        log.file.write_all(buf)?;
        log.size += buf.len() as u64;
    }
    Ok(())
}

/// Writes to the log file if one is open; logging never fails the caller.
struct FileWriter;

impl Write for FileWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut slot = FILE.lock().unwrap_or_else(|p| p.into_inner());
        let _ = append(&mut slot, buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn filter(verbose: bool) -> EnvFilter {
    // RUST_LOG still wins during development.
    // Other crates are not held to LanShare's privacy rule (notify, for
    // example, logs the paths it watches), so they get warnings only, and
    // notify nothing at all.
    let directives = if verbose { "info,lanshare_lib=debug,notify=off" } else { "warn,lanshare_lib=info,notify=off" };
    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(directives))
}

/// Installs the global subscriber. Call once, as early as possible.
pub fn init() {
    let (filter, handle) = reload::Layer::new(filter(verbose()));
    let installed = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer())
        .with(fmt::layer().with_ansi(false).with_writer(|| FileWriter))
        .try_init()
        .is_ok();
    if installed {
        let _ = FILTER.set(handle);
    }
}

/// Starts writing `<data_dir>/logs/lanshare.log`.
pub fn start_file(data_dir: &Path) {
    match LogFile::open(&data_dir.join("logs")) {
        Ok(f) => *FILE.lock().unwrap_or_else(|p| p.into_inner()) = Some(f),
        Err(e) => tracing::warn!("cannot write the log file: {e}"),
    }
}

/// The folder the log file is in, once `start_file` succeeded.
pub fn dir() -> Option<PathBuf> {
    FILE.lock().unwrap_or_else(|p| p.into_inner()).as_ref().map(|f| f.dir.clone())
}

pub fn verbose() -> bool {
    VERBOSE.load(Ordering::Relaxed)
}

pub fn set_verbose(on: bool) {
    VERBOSE.store(on, Ordering::Relaxed);
    if let Some(handle) = FILTER.get() {
        let _ = handle.reload(filter(on));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotates_and_keeps_a_few_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut slot = Some(LogFile::open(dir.path()).unwrap());
        let line = vec![b'x'; 512 * 1024];
        for _ in 0..30 {
            append(&mut slot, &line).unwrap();
        }
        let mut names: Vec<String> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["lanshare.1.log", "lanshare.2.log", "lanshare.3.log", "lanshare.4.log", "lanshare.log"]);
        for name in names {
            assert!(std::fs::metadata(dir.path().join(name)).unwrap().len() <= MAX_BYTES);
        }
    }

    /// If the current file cannot be moved aside, older files stay as they are.
    #[test]
    fn a_failed_rotation_keeps_older_logs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(rotated(dir.path(), 1), "older").unwrap();
        // A non-empty folder under the staging name makes the move fail.
        std::fs::create_dir(dir.path().join("lanshare.rotating.log")).unwrap();
        std::fs::write(dir.path().join("lanshare.rotating.log/x"), "").unwrap();
        let mut slot = Some(LogFile::open(dir.path()).unwrap());
        let line = vec![b'x'; 512 * 1024];
        for _ in 0..12 {
            append(&mut slot, &line).unwrap();
        }
        assert_eq!(std::fs::read_to_string(rotated(dir.path(), 1)).unwrap(), "older");
        assert!(!rotated(dir.path(), 2).exists());
        assert_eq!(std::fs::metadata(dir.path().join(FILE_NAME)).unwrap().len(), 12 * 512 * 1024);
    }
}
