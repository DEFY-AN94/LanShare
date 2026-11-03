//! Persistent settings stored as `config.json` in the app data directory.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Below 49152 so it stays clear of the dynamic/ephemeral range that
/// Windows (Hyper-V/WSL) carves exclusion blocks out of.
pub const DEFAULT_PORT: u16 = 39210;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    pub shared_root: PathBuf,
    #[serde(default = "default_port")]
    pub port: u16,
    /// Kept across restarts, so a pause set on an untrusted network is not
    /// quietly undone by relaunching (or, later, by launch at login).
    #[serde(default)]
    pub paused: bool,
    /// Chat messages older than this many days are deleted; 0 keeps them.
    #[serde(default)]
    pub chat_retention_days: u32,
    /// Launch at login was suggested once (after the first pairing) and is not suggested again.
    #[serde(default)]
    pub login_item_suggested: bool,
    /// Cutoff (ms) of the last automatic cleanup: phones that were away, also
    /// across a restart, drop what they still show from before it.
    #[serde(default)]
    pub pruned_before: i64,
    /// Highest message id the last "clear all chats" removed, for the same
    /// purpose. Ids, not times, so a clock that was off cannot hide later messages.
    #[serde(default)]
    pub cleared_through: i64,
    /// Notifications name the device but leave out the message text.
    #[serde(default)]
    pub notify_hide_text: bool,
    /// Global shortcut that sends the clipboard (a Tauri accelerator such as
    /// "Control+Alt+Super+KeyV"); none by default.
    #[serde(default)]
    pub clipboard_shortcut: Option<String>,
    /// The device the clipboard goes to; `None` is the most recent conversation.
    #[serde(default)]
    pub clipboard_target: Option<String>,
    /// Launch at login was last turned on in Settings (see `AppState::login_item_chosen`).
    #[serde(default)]
    pub login_item: bool,
    /// The install the last start ran from (see `AppState::note_install`).
    #[serde(default)]
    pub last_install: Option<String>,
}

/// The choices Settings offers for automatic chat cleanup.
pub const RETENTION_CHOICES: [u32; 3] = [0, 30, 90];

fn default_port() -> u16 {
    DEFAULT_PORT
}

impl Config {
    /// The defaults, sharing `shared_root`.
    pub fn with_root(shared_root: PathBuf) -> Config {
        Config {
            shared_root,
            port: DEFAULT_PORT,
            paused: false,
            chat_retention_days: 0,
            login_item_suggested: false,
            pruned_before: 0,
            cleared_through: 0,
            notify_hide_text: false,
            clipboard_shortcut: None,
            clipboard_target: None,
            login_item: false,
            last_install: None,
        }
    }

    fn file(data_dir: &Path) -> PathBuf {
        data_dir.join("config.json")
    }

    /// Loads `config.json`, falling back to `~/LanShare` on port 39210.
    /// The flag is true when a `config.json` is there but cannot be read: it
    /// is kept as `config.json.bad`, and the defaults are saved with sharing
    /// paused and message text left out of notifications, so a pause or
    /// hidden text that was lost does not quietly turn itself off. Chat
    /// cleanup stays off: deleting messages on a guess would be worse.
    pub fn load_or_default(data_dir: &Path, home: &Path) -> (Config, bool) {
        let mut config = Config::with_root(home.join("LanShare"));
        let file = Self::file(data_dir);
        // Not serde's message: it can quote a value from the file.
        let (why, bytes) = match std::fs::read(&file) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (config, false),
            Err(e) => (e.to_string(), None),
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(saved) => return (saved, false),
                Err(e) => (format!("{:?} error at line {} column {}", e.classify(), e.line(), e.column()), Some(bytes)),
            },
        };
        let kept = if keep_copy(&file, &data_dir.join(BAD_FILE), bytes.as_deref()) { "kept it as config.json.bad" } else { "could not keep a copy" };
        tracing::warn!("config.json could not be read ({why}); {kept}; starting with the default settings, sharing paused");
        config.paused = true;
        config.notify_hide_text = true;
        if let Err(e) = config.save(data_dir) {
            tracing::warn!("could not save config: {e}");
        }
        (config, true)
    }

    pub fn save(&self, data_dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(data_dir)?;
        let tmp = data_dir.join("config.json.tmp");
        {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&serde_json::to_vec_pretty(self)?)?;
            // On disk before it replaces config.json, so a power cut leaves the old file or the new one.
            f.sync_all()?;
        }
        std::fs::rename(tmp, Self::file(data_dir))
    }
}

/// Where an unreadable `config.json` is kept, next to it.
const BAD_FILE: &str = "config.json.bad";

/// Moves `file` to `bad`, replacing an older copy; if that fails (Windows
/// may refuse to move a file another program has open), writes the `bytes`
/// already read from it there instead.
fn keep_copy(file: &Path, bad: &Path, bytes: Option<&[u8]>) -> bool {
    std::fs::rename(file, bad).is_ok() || bytes.is_some_and(|b| std::fs::write(bad, b).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved(data: &Path) -> Config {
        serde_json::from_slice(&std::fs::read(data.join("config.json")).unwrap()).unwrap()
    }

    #[test]
    fn an_unreadable_file_is_kept_and_the_next_start_is_paused_too() {
        let cut_off = b"{\n  \"shared_root\": \"/srv/share\",\n  \"paused\": true,\n  \"chat_retention_days\": 30,\n  \"notify_hi";
        for original in [&cut_off[..], b""] {
            let dir = tempfile::tempdir().unwrap();
            let data = dir.path().join("data");
            std::fs::create_dir(&data).unwrap();
            std::fs::write(data.join("config.json"), original).unwrap();
            let (config, unreadable) = Config::load_or_default(&data, dir.path());
            assert!(unreadable && config.paused && config.notify_hide_text);
            assert_eq!((config.shared_root, config.chat_retention_days), (dir.path().join("LanShare"), 0));
            assert_eq!(std::fs::read(data.join(BAD_FILE)).unwrap(), original);
            let on_disk = saved(&data);
            assert!(on_disk.paused && on_disk.notify_hide_text);
            let (config, unreadable) = Config::load_or_default(&data, dir.path());
            assert!(!unreadable && config.paused && config.notify_hide_text);
            assert_eq!(std::fs::read(data.join(BAD_FILE)).unwrap(), original);
        }
    }

    /// A read that fails for another reason than a missing file (here
    /// config.json is a folder) counts the same.
    #[test]
    fn a_config_that_cannot_be_opened_is_kept_too() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        std::fs::create_dir_all(data.join("config.json")).unwrap();
        std::fs::write(data.join("config.json").join("x"), "inside").unwrap();
        let (config, unreadable) = Config::load_or_default(&data, dir.path());
        assert!(unreadable && config.paused && config.notify_hide_text);
        assert_eq!(std::fs::read_to_string(data.join(BAD_FILE).join("x")).unwrap(), "inside");
        let (config, unreadable) = Config::load_or_default(&data, dir.path());
        assert!(!unreadable && config.paused);
    }

    #[test]
    fn a_file_that_cannot_be_moved_is_copied() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join(BAD_FILE);
        // The move fails (here the file is gone): the bytes read before are written instead.
        assert!(keep_copy(&dir.path().join("config.json"), &bad, Some(b"{\"paused\": tr")));
        assert_eq!(std::fs::read(&bad).unwrap(), b"{\"paused\": tr");
        std::fs::remove_file(&bad).unwrap();
        assert!(!keep_copy(&dir.path().join("config.json"), &bad, None));
        assert!(!bad.exists());
    }

    /// When no copy can be kept the start is still paused, and what is in the way stays.
    #[test]
    fn without_a_copy_the_start_is_paused_all_the_same() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        std::fs::create_dir_all(data.join(BAD_FILE)).unwrap();
        std::fs::write(data.join(BAD_FILE).join("x"), "older").unwrap();
        std::fs::write(data.join("config.json"), b"{\"paused\": tr").unwrap();
        let (config, unreadable) = Config::load_or_default(&data, dir.path());
        assert!(unreadable && config.paused && saved(&data).paused);
        assert_eq!(std::fs::read_to_string(data.join(BAD_FILE).join("x")).unwrap(), "older");
    }

    #[test]
    fn a_first_start_is_not_paused() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let (config, unreadable) = Config::load_or_default(&data, dir.path());
        assert!(!unreadable && !config.paused && !config.notify_hide_text);
        assert!(!data.exists(), "nothing is written before the first save");
    }
}
