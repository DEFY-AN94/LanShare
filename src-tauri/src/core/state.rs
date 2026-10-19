//! Shared server state.

use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use serde_json::json;
use tokio::sync::broadcast;

use super::auth::Session;
use super::bridge::HostBridge;
use super::config::Config;
use super::db::{Db, MessageRow};
use super::events::{Audience, ServerEvent};
use super::model::{Address, FileRef, MessageOut};
use super::pairing::Pairing;
use super::paths::Sandbox;
use super::tls::Tls;
use super::util::now_ms;

pub const INBOX_DIR: &str = "收件箱";
pub const SENT_DIR: &str = "已发送";

/// 收件箱 / 已发送 and everything under them: chat attachments of every
/// conversation, which only the host may browse.
pub fn is_chat_path(rel: &str) -> bool {
    matches!(super::paths::segments(rel).ok().and_then(|s| s.first().copied()), Some(INBOX_DIR | SENT_DIR))
}

pub struct AppState {
    pub db: Db,
    pub data_dir: PathBuf,
    pub home_dir: PathBuf,
    pub config: Mutex<Config>,
    sandbox: RwLock<Sandbox>,
    pub events: broadcast::Sender<ServerEvent>,
    pub pairing: Mutex<Pairing>,
    presence: Mutex<HashMap<String, usize>>,
    paused: AtomicBool,
    /// Secret that signs the desktop window in on the loopback listener.
    pub local_token: String,
    pub local_port: u16,
    pub lan_port: u16,
    pub tls: Tls,
    pub bridge: Arc<dyn HostBridge>,
    pub host_name: String,
    addresses: RwLock<Vec<Address>>,
    /// Shown in the host UI when the OS is likely to block phones (Windows "Public" network).
    network_warning: RwLock<Option<String>>,
    /// Serializes "pick a free name + rename" so two uploads never clobber each other.
    pub upload_lock: tokio::sync::Mutex<()>,
    /// Devices removed while this process runs; their in-flight transfers stop.
    revoked: Mutex<HashSet<String>>,
    /// Watches the shared folder so changes made in Finder reach the clients.
    pub watcher: Mutex<Option<notify::RecommendedWatcher>>,
    /// Chat folders of shared folders used before (attachments remember
    /// their root): they stay host-only even inside a new shared folder.
    old_chat_dirs: RwLock<Vec<PathBuf>>,
    /// Cutoff (ms) of the last automatic cleanup or clear. Each new event
    /// stream starts with it, so clients that were offline drop those messages too.
    pruned_before: AtomicI64,
}

fn chat_dirs_of(root: &Path) -> [PathBuf; 2] {
    [root.join(INBOX_DIR), root.join(SENT_DIR)]
}

pub type Shared = Arc<AppState>;

pub struct StateParts {
    pub db: Db,
    pub data_dir: PathBuf,
    pub home_dir: PathBuf,
    pub config: Config,
    pub sandbox: Sandbox,
    pub local_token: String,
    pub local_port: u16,
    pub lan_port: u16,
    pub tls: Tls,
    pub bridge: Arc<dyn HostBridge>,
    pub host_name: String,
    pub addresses: Vec<Address>,
}

impl AppState {
    pub fn new(p: StateParts) -> Shared {
        let (events, _) = broadcast::channel(512);
        let paused = p.config.paused;
        let old_chat_dirs = p.db.attachment_roots().unwrap_or_default().iter().flat_map(|r| chat_dirs_of(Path::new(r))).collect();
        Arc::new(AppState {
            db: p.db,
            data_dir: p.data_dir,
            home_dir: p.home_dir,
            config: Mutex::new(p.config),
            sandbox: RwLock::new(p.sandbox),
            events,
            pairing: Mutex::new(Pairing::default()),
            presence: Mutex::new(HashMap::new()),
            paused: AtomicBool::new(paused),
            local_token: p.local_token,
            local_port: p.local_port,
            lan_port: p.lan_port,
            tls: p.tls,
            bridge: p.bridge,
            host_name: p.host_name,
            addresses: RwLock::new(p.addresses),
            network_warning: RwLock::new(None),
            upload_lock: tokio::sync::Mutex::new(()),
            revoked: Mutex::new(HashSet::new()),
            watcher: Mutex::new(None),
            old_chat_dirs: RwLock::new(old_chat_dirs),
            pruned_before: AtomicI64::new(0),
        })
    }

    /// Whether `abs` (a resolved path) is in a chat folder of the current
    /// shared folder or of one used before.
    pub fn is_chat_location(&self, abs: &Path) -> bool {
        self.sandbox().relative(abs).is_some_and(|rel| is_chat_path(&rel)) || self.old_chat_dirs.read().unwrap().iter().any(|d| abs.starts_with(d))
    }

    /// Marks a removed device so transfers it already started are cut off.
    pub fn revoke(&self, device_id: &str) {
        self.revoked.lock().unwrap().insert(device_id.to_string());
    }

    /// Checked for every chunk of a running upload or download.
    pub fn transfer_allowed(&self, session: &Session) -> bool {
        match session {
            Session::Host => true,
            Session::Device { id, .. } => !self.is_paused() && !self.revoked.lock().unwrap().contains(id),
        }
    }

    /// Finds a chat attachment under the folder it was saved in, which may
    /// differ from the current shared folder.
    pub fn attachment_path(&self, root: Option<&str>, rel: &str) -> Option<PathBuf> {
        let current = self.sandbox();
        let sandbox = match root {
            Some(r) if Path::new(r) != current.root() => Sandbox::existing(Path::new(r)).ok()?.with_deny(std::slice::from_ref(&self.data_dir)),
            _ => current,
        };
        sandbox.resolve_file(rel).ok()
    }

    pub fn sandbox(&self) -> Sandbox {
        self.sandbox.read().unwrap().clone()
    }

    pub fn set_sandbox(&self, sb: Sandbox) {
        let previous = std::mem::replace(&mut *self.sandbox.write().unwrap(), sb);
        self.old_chat_dirs.write().unwrap().extend(chat_dirs_of(previous.root()));
    }

    pub fn emit(&self, audience: Audience, name: &'static str, data: &impl serde::Serialize) {
        // No receivers is fine (nobody connected).
        let _ = self.events.send(ServerEvent::new(audience, name, data));
    }

    /// Changes inside the chat folders go to the host only, so phones cannot
    /// tell when another conversation received a file.
    pub fn emit_files_changed(&self, dir: &str) {
        self.emit_files_event(dir, is_chat_path(dir));
    }

    pub fn emit_files_event(&self, dir: &str, host_only: bool) {
        let audience = if host_only { Audience::Host } else { Audience::Everyone };
        self.emit(audience, "files", &json!({ "dir": dir }));
    }

    pub fn pruned_before(&self) -> i64 {
        self.pruned_before.load(Ordering::SeqCst)
    }

    /// Records that messages created before `before` (ms) no longer exist.
    pub fn mark_pruned(&self, before: i64) {
        self.pruned_before.fetch_max(before, Ordering::SeqCst);
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::SeqCst);
        {
            let mut cfg = self.config.lock().unwrap();
            if cfg.paused != paused {
                cfg.paused = paused;
                if let Err(e) = cfg.save(&self.data_dir) {
                    tracing::warn!("could not save the pause setting: {e}");
                }
            }
        }
        self.emit(Audience::Everyone, "paused", &json!({ "paused": paused }));
    }

    pub fn chat_retention_days(&self) -> u32 {
        self.config.lock().unwrap().chat_retention_days
    }

    /// Changes the setting only once it is saved, so Settings and the
    /// cleanup task never disagree.
    pub fn set_chat_retention_days(&self, days: u32) -> std::io::Result<()> {
        let mut cfg = self.config.lock().unwrap();
        let mut next = cfg.clone();
        next.chat_retention_days = days;
        next.save(&self.data_dir)?;
        *cfg = next;
        Ok(())
    }

    /// Deletes chat messages older than the retention setting and tells open
    /// conversations to drop them. Returns how many were deleted.
    pub fn prune_old_messages(&self) -> anyhow::Result<usize> {
        let days = self.chat_retention_days();
        if days == 0 {
            return Ok(0);
        }
        let before = now_ms() - i64::from(days) * 86_400_000;
        let n = self.db.delete_messages_before(before)?;
        self.mark_pruned(before);
        if n > 0 {
            self.emit(Audience::Everyone, "pruned", &json!({ "before": before }));
            self.emit(Audience::Host, "devices", &json!({}));
        }
        Ok(n)
    }

    pub fn addresses(&self) -> Vec<Address> {
        self.addresses.read().unwrap().clone()
    }

    pub fn network_warning(&self) -> Option<String> {
        self.network_warning.read().unwrap().clone()
    }

    /// Returns true if the warning changed.
    pub fn set_network_warning(&self, w: Option<String>) -> bool {
        let mut cur = self.network_warning.write().unwrap();
        if *cur == w {
            return false;
        }
        *cur = w;
        true
    }

    pub fn address_ips(&self) -> Vec<IpAddr> {
        self.addresses().iter().filter_map(|a| a.ip.parse().ok()).collect()
    }

    /// Returns true if the list changed.
    pub fn set_addresses(&self, next: Vec<Address>) -> bool {
        let mut cur = self.addresses.write().unwrap();
        if *cur == next {
            return false;
        }
        *cur = next;
        true
    }

    // ---- presence (devices with an open event stream) ----

    pub fn presence_enter(&self, device_id: &str) {
        let first = {
            let mut p = self.presence.lock().unwrap();
            let n = p.entry(device_id.to_string()).or_insert(0);
            *n += 1;
            *n == 1
        };
        if first {
            self.emit(Audience::Host, "devices", &json!({}));
        }
    }

    pub fn presence_leave(&self, device_id: &str) {
        let last = {
            let mut p = self.presence.lock().unwrap();
            match p.get_mut(device_id) {
                Some(n) if *n > 1 => {
                    *n -= 1;
                    false
                }
                Some(_) => {
                    p.remove(device_id);
                    true
                }
                None => false,
            }
        };
        if last {
            self.emit(Audience::Host, "devices", &json!({}));
        }
    }

    pub fn is_online(&self, device_id: &str) -> bool {
        self.presence.lock().unwrap().contains_key(device_id)
    }

    // ---- conversions ----

    pub fn message_out(&self, m: MessageRow) -> MessageOut {
        let file = match (m.kind.as_str(), m.file_path) {
            ("file", Some(path)) => {
                let exists = self.attachment_path(m.file_root.as_deref(), &path).is_some();
                Some(FileRef {
                    name: m.file_name.unwrap_or_else(|| path.rsplit('/').next().unwrap_or("").to_string()),
                    path,
                    size: m.file_size.unwrap_or(0).max(0) as u64,
                    mime: m.mime.unwrap_or_else(|| "application/octet-stream".into()),
                    exists,
                })
            }
            _ => None,
        };
        MessageOut {
            id: m.id,
            device_id: m.device_id,
            from_host: m.from_host,
            client_id: m.client_id,
            kind: m.kind,
            text: m.text,
            file,
            created_at: m.created_at,
        }
    }
}

/// Keeps a device marked online while its SSE stream is open.
pub struct PresenceGuard {
    state: Shared,
    device_id: Option<String>,
}

impl PresenceGuard {
    pub fn new(state: Shared, device_id: Option<String>) -> Self {
        if let Some(id) = &device_id {
            state.presence_enter(id);
        }
        Self { state, device_id }
    }
}

impl Drop for PresenceGuard {
    fn drop(&mut self) {
        if let Some(id) = &self.device_id {
            self.state.presence_leave(id);
        }
    }
}
