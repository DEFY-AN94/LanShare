//! What the server needs from the desktop shell. The Tauri app implements
//! this with real notifications and dialogs; the headless runner uses
//! plain OS commands so the server can be tested without a window.

use std::path::{Path, PathBuf};
use std::process::Command;

pub use super::notices::{MessageNotice, NoticeKind, Withdraw};

/// What the clipboard holds when the user sends it to a phone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Clip {
    Empty,
    Text(String),
    /// A picture (a screenshot), as PNG bytes.
    Png(Vec<u8>),
    /// Files copied in Finder / File Explorer, or a picture an app such as
    /// QQ or WeChat copies as its file. Only pictures are sent (`core::quick`).
    /// `image` is the picture itself as PNG when the clipboard holds it too
    /// (not a file icon), for a file LanShare may not read: macOS keeps other
    /// apps' folders private, and QQ copies pictures from its own.
    Files { paths: Vec<PathBuf>, image: Option<Vec<u8>> },
    /// Something that cannot be sent, with the reason to show.
    Unsupported(String),
}

pub trait HostBridge: Send + Sync + 'static {
    /// Shows a system notification, such as a pairing request or an error.
    fn notify(&self, title: &str, body: &str);
    /// Shows a phone's message; its buttons and a click come back through
    /// `core::quick::on_notification`. Shells without them show a plain one.
    fn notify_message(&self, n: &MessageNotice) {
        if !n.quiet {
            self.notify(&n.title, &n.body);
        }
    }
    /// Whether `notify_message` can update a notification without a banner
    /// (`MessageNotice::quiet`); without it those updates are skipped.
    fn quiet_updates(&self) -> bool {
        true
    }
    /// Takes notifications back (a message deleted, a conversation read).
    fn withdraw_notifications(&self, _w: &Withdraw) {}
    /// Whether the main window is in front, so a banner would only repeat it.
    fn window_focused(&self) -> bool {
        false
    }
    /// Unread messages from all phones, for the Dock / menu bar / tray.
    fn set_unread(&self, _total: u32) {}
    /// Shows the window with this device's conversation open.
    fn show_conversation(&self, _device_id: &str) {}
    /// Notifications as the system allows them: "allowed", "denied",
    /// "notDetermined" or "unknown". Blocking.
    fn notifications_state(&self) -> &'static str {
        "unknown"
    }
    /// Reads the clipboard; only called when the user asks to send it. Blocking.
    fn read_clipboard(&self) -> Clip {
        Clip::Unsupported("这里不能读取剪贴板".into())
    }
    /// Puts text on the clipboard. Blocking.
    fn write_clipboard_text(&self, _text: &str) -> Result<(), String> {
        Err("这里不能写入剪贴板".into())
    }
    /// Registers the global shortcut that sends the clipboard (replacing the
    /// previous one), or removes it with `None`. On an error the previous one
    /// stays registered. Blocking.
    fn set_shortcut(&self, _accelerator: Option<&str>) -> Result<(), String> {
        Err("这个版本不支持全局快捷键".into())
    }
    fn open_path(&self, path: &Path) -> Result<(), String>;
    fn reveal_path(&self, path: &Path) -> Result<(), String>;
    fn open_url(&self, url: &str) -> Result<(), String>;
    /// Blocking native folder picker; called from a blocking thread.
    fn pick_folder(&self) -> Option<PathBuf>;
    /// Brings the main window to the front (e.g. for a pairing request).
    fn show_window(&self) {}
    /// Launch at login as the Settings switch shows it: "unsupported",
    /// "notInstalled", "off", "on" or "needsApproval". Blocking.
    fn login_item(&self) -> &'static str {
        "unsupported"
    }
    /// Turns launch at login on or off; returns the state afterwards. Blocking.
    fn set_login_item(&self, _on: bool) -> Result<&'static str, String> {
        Ok("unsupported")
    }
    /// Opens the system page where launch at login can be allowed again.
    fn open_login_item_settings(&self) {}
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
