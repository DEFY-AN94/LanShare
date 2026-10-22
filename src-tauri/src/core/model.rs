//! JSON shapes sent to the web UI (mirrored in src/lib/types.ts).
//! Timestamps are milliseconds since the Unix epoch.

use serde::Serialize;

#[derive(Serialize, Clone, Debug)]
pub struct FileRef {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub mime: String,
    pub exists: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct MessageOut {
    pub id: i64,
    pub device_id: String,
    pub from_host: bool,
    pub client_id: Option<String>,
    pub kind: String,
    pub text: Option<String>,
    pub file: Option<FileRef>,
    pub created_at: i64,
}

#[derive(Serialize)]
pub struct MessagePage {
    pub messages: Vec<MessageOut>,
    pub has_more: bool,
}

#[derive(Serialize)]
pub struct Me {
    pub kind: &'static str,
    pub device_id: Option<String>,
    pub device_name: Option<String>,
    pub host_name: String,
    pub can_write: bool,
    pub paused: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: i64,
}

#[derive(Serialize)]
pub struct DirListing {
    pub path: String,
    pub entries: Vec<FileEntry>,
    pub can_write: bool,
}

#[derive(Serialize, Clone)]
pub struct DeviceOut {
    pub id: String,
    pub name: String,
    pub can_write: bool,
    pub created_at: i64,
    pub last_seen_at: Option<i64>,
    pub online: bool,
    pub last_message: Option<MessageOut>,
    /// Messages from this device the computer has not read yet.
    pub unread: u32,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Address {
    pub ip: String,
    pub iface: String,
}

#[derive(Serialize)]
pub struct PairingUrl {
    pub ip: String,
    pub iface: String,
    pub url: String,
    pub qr_svg: String,
}

#[derive(Serialize)]
pub struct PairingSession {
    pub code: String,
    pub expires_at: i64,
    pub urls: Vec<PairingUrl>,
    /// SHA-256 of the local CA, to compare with the profile on the phone.
    pub ca_fingerprint: String,
    pub network_warning: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct PairRequestOut {
    pub id: String,
    pub name: String,
    pub confirm_code: String,
    pub ip: String,
    pub created_at: i64,
}

#[derive(Serialize)]
pub struct HostOverview {
    pub host_name: String,
    pub port: u16,
    pub addresses: Vec<Address>,
    pub shared_root: String,
    /// Why the shared folder cannot be used right now ("disk" or "folder"), if it cannot.
    pub shared_root_unavailable: Option<crate::core::paths::Unavailable>,
    /// The folder chosen in Settings while it is unavailable and `shared_root` stands in for it.
    pub preferred_root: Option<String>,
    pub ca_fingerprint: String,
    /// The CA's name as iPhone settings list it.
    pub ca_name: String,
    pub paused: bool,
    pub version: String,
    pub network_warning: Option<String>,
    /// Automatic chat cleanup: messages older than this many days are deleted; 0 = off.
    pub chat_retention_days: u32,
    /// Debug-level logging is on (it resets when LanShare restarts).
    pub verbose_logging: bool,
    /// "macos", "windows" or "linux", for platform-specific wording.
    pub platform: &'static str,
    /// Whether the system shows LanShare's notifications ("allowed",
    /// "denied", "notDetermined" or "unknown").
    pub notifications: &'static str,
    pub notify_hide_text: bool,
    /// The global shortcut that sends the clipboard, if one is set.
    pub clipboard_shortcut: Option<String>,
    /// Why that shortcut could not be registered when LanShare started.
    pub clipboard_shortcut_error: Option<String>,
    /// The device the clipboard goes to; `None` is the most recent conversation.
    pub clipboard_target: Option<String>,
}
