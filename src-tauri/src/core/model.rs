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
    pub ca_fingerprint: String,
    pub paused: bool,
    pub version: String,
}
