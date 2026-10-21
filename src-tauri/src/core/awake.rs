//! Keeps the computer from idle-sleeping while a phone transfer runs, so an
//! upload or download does not die because the Mac or PC went to sleep.

use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How long the computer stays awake after the last transfer ends, so a
/// burst of small downloads (chat thumbnails) does not toggle it each time.
const LINGER: Duration = if cfg!(test) { Duration::from_millis(50) } else { Duration::from_secs(30) };

#[derive(Default)]
struct Inner {
    active: usize,
    generation: u64,
    platform: Option<Platform>,
}

#[derive(Clone, Default)]
pub struct KeepAwake(Arc<Mutex<Inner>>);

/// Held for the length of one transfer.
pub struct AwakeGuard(KeepAwake);

impl KeepAwake {
    pub fn hold(&self) -> AwakeGuard {
        let mut inner = self.0.lock().unwrap_or_else(|p| p.into_inner());
        inner.active += 1;
        inner.generation += 1;
        if inner.platform.is_none() {
            inner.platform = Platform::start();
        }
        AwakeGuard(self.clone())
    }

    pub fn is_awake(&self) -> bool {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).platform.is_some()
    }

    fn release(&self) {
        let generation = {
            let mut inner = self.0.lock().unwrap_or_else(|p| p.into_inner());
            inner.active = inner.active.saturating_sub(1);
            if inner.active > 0 {
                return;
            }
            inner.generation
        };
        let me = self.clone();
        match tokio::runtime::Handle::try_current() {
            Ok(rt) => {
                rt.spawn(async move {
                    tokio::time::sleep(LINGER).await;
                    me.stop_if_idle(generation);
                });
            }
            Err(_) => me.stop_if_idle(generation),
        }
    }

    /// Stops only if nothing started since this release.
    fn stop_if_idle(&self, generation: u64) {
        let mut inner = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if inner.active == 0 && inner.generation == generation {
            inner.platform = None;
        }
    }
}

impl Drop for AwakeGuard {
    fn drop(&mut self) {
        self.0.release();
    }
}

/// macOS: `caffeinate -i` prevents idle sleep; `-w` ends it if LanShare quits.
#[cfg(target_os = "macos")]
struct Platform(std::process::Child);

#[cfg(target_os = "macos")]
impl Platform {
    fn start() -> Option<Platform> {
        use std::process::{Command, Stdio};
        let pid = std::process::id().to_string();
        match Command::new("/usr/bin/caffeinate").args(["-i", "-w", &pid]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
            Ok(child) => Some(Platform(child)),
            Err(e) => {
                tracing::warn!("cannot keep the Mac awake during a transfer: {e}");
                None
            }
        }
    }
}

#[cfg(target_os = "macos")]
impl Drop for Platform {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Windows: the request belongs to the thread that made it, so one thread
/// holds it until told to stop.
#[cfg(windows)]
struct Platform(std::sync::mpsc::Sender<()>);

#[cfg(windows)]
impl Platform {
    fn start() -> Option<Platform> {
        use windows_sys::Win32::System::Power::{ES_CONTINUOUS, ES_SYSTEM_REQUIRED, SetThreadExecutionState};
        let (stop, wait) = std::sync::mpsc::channel::<()>();
        let spawned = std::thread::Builder::new().name("lanshare-awake".into()).spawn(move || {
            // SAFETY: plain flag arguments.
            unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) };
            let _ = wait.recv();
            // SAFETY: plain flag arguments.
            unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
        });
        match spawned {
            Ok(_) => Some(Platform(stop)),
            Err(e) => {
                tracing::warn!("cannot keep the PC awake during a transfer: {e}");
                None
            }
        }
    }
}

#[cfg(windows)]
impl Drop for Platform {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
struct Platform;

#[cfg(not(any(target_os = "macos", windows)))]
impl Platform {
    fn start() -> Option<Platform> {
        Some(Platform)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stays_awake_until_the_last_transfer_has_ended() {
        let awake = KeepAwake::default();
        assert!(!awake.is_awake());
        let a = awake.hold();
        let b = awake.hold();
        assert!(awake.is_awake());
        drop(a);
        tokio::time::sleep(LINGER * 3).await;
        assert!(awake.is_awake(), "one transfer is still running");
        drop(b);
        assert!(awake.is_awake(), "it lingers after the last transfer");
        let c = awake.hold();
        tokio::time::sleep(LINGER * 3).await;
        assert!(awake.is_awake(), "a transfer that started during the linger keeps it on");
        drop(c);
        tokio::time::sleep(LINGER * 3).await;
        assert!(!awake.is_awake());
    }
}
