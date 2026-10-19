//! What the server needs from the desktop shell. The Tauri app implements
//! this with real notifications and dialogs; the headless runner uses
//! plain OS commands so the server can be tested without a window.

use std::path::{Path, PathBuf};
use std::process::Command;

pub trait HostBridge: Send + Sync + 'static {
    /// Shows a system notification (the shell decides whether to suppress it
    /// while the main window is focused).
    fn notify(&self, title: &str, body: &str);
    fn open_path(&self, path: &Path) -> Result<(), String>;
    fn reveal_path(&self, path: &Path) -> Result<(), String>;
    fn open_url(&self, url: &str) -> Result<(), String>;
    /// Blocking native folder picker; called from a blocking thread.
    fn pick_folder(&self) -> Option<PathBuf>;
    /// Brings the main window to the front (e.g. for a pairing request).
    fn show_window(&self) {}
}

/// Used by `examples/headless.rs` and tests.
pub struct HeadlessBridge;

fn run(cmd: &mut Command) -> Result<(), String> {
    cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
}

impl HostBridge for HeadlessBridge {
    fn notify(&self, title: &str, body: &str) {
        // The body is message text or a file name: log its length only.
        tracing::info!("[notification] {title} ({} characters)", body.chars().count());
    }

    fn open_path(&self, path: &Path) -> Result<(), String> {
        if cfg!(target_os = "macos") {
            run(Command::new("open").arg(path))
        } else if cfg!(windows) {
            run(Command::new("explorer").arg(path))
        } else {
            run(Command::new("xdg-open").arg(path))
        }
    }

    fn reveal_path(&self, path: &Path) -> Result<(), String> {
        if cfg!(target_os = "macos") {
            run(Command::new("open").arg("-R").arg(path))
        } else if cfg!(windows) {
            run(Command::new("explorer").arg(format!("/select,{}", path.display())))
        } else {
            run(Command::new("xdg-open").arg(path.parent().unwrap_or(path)))
        }
    }

    fn open_url(&self, url: &str) -> Result<(), String> {
        if cfg!(target_os = "macos") {
            run(Command::new("open").arg(url))
        } else if cfg!(windows) {
            run(Command::new("explorer").arg(url))
        } else {
            run(Command::new("xdg-open").arg(url))
        }
    }

    fn pick_folder(&self) -> Option<PathBuf> {
        None
    }
}
