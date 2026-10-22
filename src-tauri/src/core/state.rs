//! Shared server state.

use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, UNIX_EPOCH};

use serde_json::json;
use tokio::sync::broadcast;

use super::auth::Session;
use super::awake::KeepAwake;
use super::bridge::{HostBridge, Withdraw};
use super::config::Config;
use super::db::{Db, MessageRow};
use super::disk;
use super::events::{Audience, ServerEvent};
use super::model::{Address, FileRef, MessageOut};
use super::notices::Notifier;
use super::pairing::Pairing;
use super::paths::{Sandbox, Unavailable, unsafe_root_reason};
use super::tls::Tls;
use super::util::now_ms;

pub const INBOX_DIR: &str = "收件箱";
pub const SENT_DIR: &str = "已发送";

/// 收件箱 / 已发送 and everything under them: chat attachments of every
/// conversation, which only the host may browse.
pub fn is_chat_path(rel: &str) -> bool {
    matches!(super::paths::segments(rel).ok().and_then(|s| s.first().copied()), Some(INBOX_DIR | SENT_DIR))
}

/// When a file was created, in ms, as kept with chat attachments.
pub fn created_ms(meta: &std::fs::Metadata) -> Option<i64> {
    meta.created().ok()?.duration_since(UNIX_EPOCH).ok().map(|d| d.as_millis() as i64)
}

/// FAT keeps file times to 2 s.
const SLACK: i64 = 2_000;
/// FAT keeps local time, so a daylight-saving or time-zone change moves every
/// time on it, by whole quarter hours and at most from UTC-12 to UTC+14.
const QUARTER_HOUR: i64 = 15 * 60_000;
const MAX_SHIFT: i64 = 26 * 3_600_000;

/// Whether the file at `path` is still the one a message saved: created at
/// the moment recorded with it (`created`), or, for messages saved before
/// LanShare recorded that, no later than the message itself (`message_at`).
/// Editing the file keeps its creation time, and so does saving over it
/// (macOS and Windows carry it over); a different file that took the name
/// later was created later. Where the system keeps no creation time, it
/// counts as the same file.
fn same_file(path: &Path, created: Option<i64>, message_at: i64) -> bool {
    let Ok(meta) = std::fs::metadata(path) else { return false };
    let Some(have) = created_ms(&meta) else { return true };
    let fat = || disk::disk_info(path).is_ok_and(|d| d.fat);
    match created {
        Some(want) => same_moment(have, want, fat),
        None => have <= message_at + SLACK || (have <= message_at + MAX_SHIFT + SLACK && fat()),
    }
}

/// Within 2 s, or, on FAT (`fat`), off by whole quarter hours (see `QUARTER_HOUR`).
fn same_moment(have: i64, want: i64, fat: impl FnOnce() -> bool) -> bool {
    let diff = (have - want).abs();
    if diff <= SLACK {
        return true;
    }
    let off = diff % QUARTER_HOUR;
    diff <= MAX_SHIFT + SLACK && (off <= SLACK || QUARTER_HOUR - off <= SLACK) && fat()
}

pub struct AppState {
    pub db: Db,
    pub data_dir: PathBuf,
    pub home_dir: PathBuf,
    pub config: Mutex<Config>,
    sandbox: RwLock<Sandbox>,
    /// What the last check of the shared folder found (see `Sandbox::check_root`).
    root_status: Mutex<Result<(), Unavailable>>,
    /// The folder chosen in Settings, while it was unavailable at startup and
    /// the default folder stands in for it; used again once it is back.
    preferred_root: Mutex<Option<PathBuf>>,
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
    /// Cutoff (ms) of the last automatic cleanup and the highest id the last
    /// clear removed (both kept in the config). Each new event stream starts
    /// with them, so clients that were offline drop those messages too;
    /// single deletes come from the database.
    pruned_before: AtomicI64,
    cleared_through: AtomicI64,
    /// Bytes promised to uploads in progress, so parallel uploads cannot
    /// together break the free-space reserve.
    upload_reserved: Mutex<u64>,
    /// Finished Files uploads by (who, upload ID): a retry whose first
    /// attempt already landed gets that file back instead of a "(1)" copy.
    recent_uploads: Mutex<HashMap<(String, String), (PathBuf, Instant)>>,
    /// Keeps the computer awake while a transfer runs.
    pub awake: KeepAwake,
    /// Replaces the free-space reserve (tests on full disks); `None` in the app.
    pub upload_reserve: Option<u64>,
    /// How many notifications each phone has raised in its current burst.
    pub notifier: Notifier,
    /// The conversation open in the window while the window is in front:
    /// its messages are read as they arrive.
    viewing: Mutex<Option<String>>,
    /// Why the saved global shortcut could not be registered at startup.
    shortcut_error: Mutex<Option<String>>,
    /// Since when Settings is recording a new shortcut: pressing the current
    /// one then sends nothing.
    shortcut_recording: Mutex<Option<Instant>>,
}

/// A recording Settings did not end (the page went away) stops counting after this.
const SHORTCUT_RECORDING_MAX: Duration = Duration::from_secs(120);

/// How many deleted message ids a new event stream starts with.
const DELETED_REPLAYED: usize = 1000;
/// How long finished upload ids are remembered.
const UPLOAD_ID_TTL: Duration = Duration::from_secs(24 * 3600);
/// How long deletions are remembered (see `Db::was_deleted`). Longer than a
/// phone sends a queued text by itself (OUTBOX_AUTO_SEND_MS in src/chat/store.ts).
const DELETIONS_KEPT_MS: i64 = 180 * 86_400_000;

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
    pub preferred_root: Option<PathBuf>,
    pub local_token: String,
    pub local_port: u16,
    pub lan_port: u16,
    pub tls: Tls,
    pub bridge: Arc<dyn HostBridge>,
    pub host_name: String,
    pub addresses: Vec<Address>,
    pub upload_reserve: Option<u64>,
}

impl AppState {
    pub fn new(p: StateParts) -> Shared {
        let (events, _) = broadcast::channel(512);
        let paused = p.config.paused;
        // A cutoff from a clock that ran ahead must not hide newer messages.
        let pruned_before = p.config.pruned_before.min(now_ms());
        let cleared_through = p.config.cleared_through;
        let old_chat_dirs = p.db.attachment_roots().unwrap_or_default().iter().flat_map(|r| chat_dirs_of(Path::new(r))).collect();
        let notifier = if p.bridge.quiet_updates() { Notifier::default() } else { Notifier::without_quiet_updates() };
        Arc::new(AppState {
            db: p.db,
            data_dir: p.data_dir,
            home_dir: p.home_dir,
            config: Mutex::new(p.config),
            sandbox: RwLock::new(p.sandbox),
            root_status: Mutex::new(Ok(())),
            preferred_root: Mutex::new(p.preferred_root),
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
            pruned_before: AtomicI64::new(pruned_before),
            cleared_through: AtomicI64::new(cleared_through),
            upload_reserved: Mutex::new(0),
            recent_uploads: Mutex::new(HashMap::new()),
            awake: KeepAwake::default(),
            upload_reserve: p.upload_reserve,
            notifier,
            viewing: Mutex::new(None),
            shortcut_error: Mutex::new(None),
            shortcut_recording: Mutex::new(None),
        })
    }

    /// Ids of recently deleted messages in conversations `session` can see.
    pub fn deleted_visible_to(&self, session: &Session) -> Vec<i64> {
        self.db.deleted_ids(session.device_id(), DELETED_REPLAYED).unwrap_or_else(|e| {
            tracing::warn!("could not read recent deletions: {e}");
            Vec::new()
        })
    }

    /// Reserves `bytes` for an upload if `fits(bytes already reserved)` allows it.
    pub fn try_reserve_upload(&self, bytes: u64, fits: impl FnOnce(u64) -> bool) -> bool {
        let mut reserved = self.upload_reserved.lock().unwrap();
        if !fits(*reserved) {
            return false;
        }
        *reserved += bytes;
        true
    }

    pub fn release_upload(&self, bytes: u64) {
        let mut reserved = self.upload_reserved.lock().unwrap();
        *reserved = reserved.saturating_sub(bytes);
    }

    pub fn reserved_uploads(&self) -> u64 {
        *self.upload_reserved.lock().unwrap()
    }

    pub fn recent_upload(&self, who: &str, upload_id: &str) -> Option<PathBuf> {
        let mut map = self.recent_uploads.lock().unwrap();
        map.retain(|_, (_, at)| at.elapsed() < UPLOAD_ID_TTL);
        map.get(&(who.to_string(), upload_id.to_string())).map(|(p, _)| p.clone())
    }

    pub fn remember_upload(&self, who: &str, upload_id: &str, path: PathBuf) {
        let mut map = self.recent_uploads.lock().unwrap();
        if map.len() >= 2000 {
            map.retain(|_, (_, at)| at.elapsed() < UPLOAD_ID_TTL);
        }
        map.insert((who.to_string(), upload_id.to_string()), (path, Instant::now()));
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

    /// Finds a message's attachment under the folder it was saved in, which
    /// may differ from the current shared folder. A different file now at
    /// that path (the attachment was moved away and a newer one took its
    /// name, perhaps from another phone) does not count.
    pub fn attachment_path(&self, m: &MessageRow) -> Option<PathBuf> {
        let rel = m.file_path.as_deref()?;
        let current = self.sandbox();
        let sandbox = match m.file_root.as_deref() {
            Some(r) if Path::new(r) != current.root() => Sandbox::existing(Path::new(r)).ok()?.with_deny(std::slice::from_ref(&self.data_dir)),
            _ => current,
        };
        let path = sandbox.resolve_file(rel).ok()?;
        same_file(&path, m.file_created, m.created_at).then_some(path)
    }

    pub fn sandbox(&self) -> Sandbox {
        self.sandbox.read().unwrap().clone()
    }

    /// The shared folder, or why it cannot be used right now. Everything that
    /// reads or writes in it asks this first.
    pub fn usable_sandbox(&self) -> Result<Sandbox, Unavailable> {
        let sb = self.sandbox();
        if let Err(why) = sb.check_root() {
            self.note_root_status(Err(why));
            return Err(why);
        }
        Ok(sb)
    }

    /// Records what a check of the shared folder found; when that changed, it
    /// is logged and Settings is told. Returns true if it changed.
    pub fn note_root_status(&self, now: Result<(), Unavailable>) -> bool {
        if std::mem::replace(&mut *self.root_status.lock().unwrap(), now) == now {
            return false;
        }
        match now {
            Ok(()) => tracing::info!("the shared folder is available again"),
            Err(Unavailable::Disk) => tracing::warn!("the shared folder is unavailable: its disk is not connected"),
            Err(Unavailable::Folder) => tracing::warn!("the shared folder is missing: moved, renamed or deleted"),
        }
        self.emit(Audience::Host, "shared_root", &json!({}));
        true
    }

    pub fn preferred_root(&self) -> Option<PathBuf> {
        self.preferred_root.lock().unwrap().clone()
    }

    /// Goes back to the folder chosen in Settings once it is there again.
    /// Returns true if it did.
    pub fn reopen_preferred_root(&self) -> bool {
        let Some(path) = self.preferred_root() else { return false };
        if unsafe_root_reason(&path, &self.home_dir, &self.data_dir).is_some() {
            return false;
        }
        let Ok(sb) = Sandbox::existing(&path) else { return false };
        let sb = sb.with_deny(std::slice::from_ref(&self.data_dir));
        // Unless another folder was picked in Settings meanwhile.
        let mut preferred = self.preferred_root.lock().unwrap();
        if preferred.as_ref() != Some(&path) {
            return false;
        }
        *preferred = None;
        self.replace_sandbox(sb);
        true
    }

    /// A folder picked in Settings replaces any preferred one.
    pub fn set_sandbox(&self, sb: Sandbox) {
        let mut preferred = self.preferred_root.lock().unwrap();
        *preferred = None;
        self.replace_sandbox(sb);
    }

    fn replace_sandbox(&self, sb: Sandbox) {
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

    /// Never later than now, in case the clock ran ahead when it was set.
    pub fn pruned_before(&self) -> i64 {
        self.pruned_before.load(Ordering::SeqCst).min(now_ms())
    }

    pub fn cleared_through(&self) -> i64 {
        self.cleared_through.load(Ordering::SeqCst)
    }

    /// Records that messages created before `before` (ms) no longer exist.
    pub fn mark_pruned(&self, before: i64) {
        if self.pruned_before.fetch_max(before, Ordering::SeqCst) < before {
            self.save_cutoff(|cfg| cfg.pruned_before = cfg.pruned_before.max(before));
        }
    }

    /// Records that messages up to id `through` no longer exist.
    pub fn mark_cleared(&self, through: i64) {
        if self.cleared_through.fetch_max(through, Ordering::SeqCst) < through {
            self.save_cutoff(|cfg| cfg.cleared_through = cfg.cleared_through.max(through));
        }
    }

    fn save_cutoff(&self, set: impl FnOnce(&mut Config)) {
        let mut cfg = self.config.lock().unwrap();
        set(&mut cfg);
        if let Err(e) = cfg.save(&self.data_dir) {
            tracing::warn!("could not save the chat cleanup cutoff: {e}");
        }
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

    pub fn login_item_suggested(&self) -> bool {
        self.config.lock().unwrap().login_item_suggested
    }

    pub fn mark_login_item_suggested(&self) {
        let mut cfg = self.config.lock().unwrap();
        if cfg.login_item_suggested {
            return;
        }
        cfg.login_item_suggested = true;
        if let Err(e) = cfg.save(&self.data_dir) {
            tracing::warn!("could not save the launch-at-login suggestion: {e}");
        }
    }

    /// Saves a change to the settings, and keeps it only once it is saved.
    fn change_config(&self, change: impl FnOnce(&mut Config)) -> std::io::Result<()> {
        let mut cfg = self.config.lock().unwrap();
        let mut next = cfg.clone();
        change(&mut next);
        next.save(&self.data_dir)?;
        *cfg = next;
        Ok(())
    }

    pub fn notify_hide_text(&self) -> bool {
        self.config.lock().unwrap().notify_hide_text
    }

    pub fn set_notify_hide_text(&self, hide: bool) -> std::io::Result<()> {
        self.change_config(|c| c.notify_hide_text = hide)
    }

    pub fn clipboard_shortcut(&self) -> Option<String> {
        self.config.lock().unwrap().clipboard_shortcut.clone()
    }

    pub fn set_clipboard_shortcut(&self, shortcut: Option<String>) -> std::io::Result<()> {
        self.change_config(|c| c.clipboard_shortcut = shortcut)?;
        *self.shortcut_error.lock().unwrap() = None;
        Ok(())
    }

    pub fn clipboard_target(&self) -> Option<String> {
        self.config.lock().unwrap().clipboard_target.clone()
    }

    pub fn set_clipboard_target(&self, device_id: Option<String>) -> std::io::Result<()> {
        self.change_config(|c| c.clipboard_target = device_id)
    }

    pub fn shortcut_error(&self) -> Option<String> {
        self.shortcut_error.lock().unwrap().clone()
    }

    pub fn set_shortcut_error(&self, error: Option<String>) {
        *self.shortcut_error.lock().unwrap() = error;
    }

    pub fn shortcut_recording(&self) -> bool {
        self.shortcut_recording.lock().unwrap().is_some_and(|since| since.elapsed() < SHORTCUT_RECORDING_MAX)
    }

    pub fn set_shortcut_recording(&self, on: bool) {
        *self.shortcut_recording.lock().unwrap() = on.then(Instant::now);
    }

    /// Records `stamp`, which every install of the app changes (Windows: the
    /// time of its uninstaller); returns true when it differs from the last
    /// start's: the first start after an install or upgrade.
    pub fn note_install(&self, stamp: &str) -> bool {
        if self.config.lock().unwrap().last_install.as_deref() == Some(stamp) {
            return false;
        }
        if let Err(e) = self.change_config(|c| c.last_install = Some(stamp.to_string())) {
            tracing::warn!("could not save the install time: {e}");
        }
        true
    }

    /// Launch at login as last set in Settings. Windows puts it back after an
    /// upgrade: the old version's uninstaller removes it.
    pub fn login_item_chosen(&self) -> bool {
        self.config.lock().unwrap().login_item
    }

    pub fn set_login_item_chosen(&self, on: bool) {
        if self.login_item_chosen() != on
            && let Err(e) = self.change_config(|c| c.login_item = on)
        {
            tracing::warn!("could not save the launch-at-login choice: {e}");
        }
    }

    pub fn is_viewing(&self, device_id: &str) -> bool {
        self.viewing.lock().unwrap().as_deref() == Some(device_id)
    }

    /// The window shows `device_id`'s conversation in front (or nothing):
    /// what it shows counts as read, and its notifications are taken back.
    pub fn set_viewing(&self, device_id: Option<String>) {
        // Published before marking read: a message arriving meanwhile is either
        // covered by mark_read or finds its conversation in front (chat::after_insert).
        *self.viewing.lock().unwrap() = device_id.clone();
        if let Some(id) = &device_id {
            if let Err(e) = self.db.mark_read(id) {
                tracing::warn!("could not mark a conversation read: {e}");
            }
            self.notifier.forget(id);
            self.bridge.withdraw_notifications(&Withdraw::Device(id.clone()));
        }
        self.unread_changed();
    }

    /// Unread messages per device.
    pub fn unread_counts(&self) -> HashMap<String, u32> {
        self.db.unread_counts().map(|v| v.into_iter().collect()).unwrap_or_else(|e| {
            tracing::warn!("could not count unread messages: {e}");
            HashMap::new()
        })
    }

    /// Tells the window and the shell after anything that changes what is unread.
    pub fn unread_changed(&self) {
        let counts = self.unread_counts();
        let total = counts.values().sum::<u32>();
        self.emit(Audience::Host, "unread", &json!({ "counts": counts, "total": total }));
        self.bridge.set_unread(total);
    }

    /// A device and its conversation are gone: its notifications go too.
    pub fn device_removed(&self, device_id: &str) {
        self.notifier.forget(device_id);
        self.bridge.withdraw_notifications(&Withdraw::Device(device_id.to_string()));
        let mut viewing = self.viewing.lock().unwrap();
        if viewing.as_deref() == Some(device_id) {
            *viewing = None;
        }
        drop(viewing);
        self.unread_changed();
    }

    pub fn chat_retention_days(&self) -> u32 {
        self.config.lock().unwrap().chat_retention_days
    }

    /// Changes the setting only once it is saved, so Settings and the
    /// cleanup task never disagree.
    pub fn set_chat_retention_days(&self, days: u32) -> std::io::Result<()> {
        self.change_config(|c| c.chat_retention_days = days)
    }

    /// Deletes chat messages older than the retention setting and tells open
    /// conversations to drop them. Returns how many were deleted.
    pub fn prune_old_messages(&self) -> anyhow::Result<usize> {
        self.db.forget_deleted_before(now_ms() - DELETIONS_KEPT_MS)?;
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
            self.unread_changed();
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
        let exists = m.kind == "file" && self.attachment_path(&m).is_some();
        let file = match (m.kind.as_str(), m.file_path) {
            ("file", Some(path)) => Some(FileRef {
                name: m.file_name.unwrap_or_else(|| path.rsplit('/').next().unwrap_or("").to_string()),
                path,
                size: m.file_size.unwrap_or(0).max(0) as u64,
                mime: m.mime.unwrap_or_else(|| "application/octet-stream".into()),
                exists,
            }),
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

#[cfg(test)]
mod tests {
    use super::same_moment;

    #[test]
    fn creation_times_match_within_fat_rules() {
        let t = 1_800_000_000_000;
        assert!(same_moment(t + 1_500, t, || false), "FAT keeps 2 s");
        assert!(!same_moment(t + 3_000, t, || true));
        // After a daylight-saving or time-zone change, but only on FAT.
        assert!(same_moment(t + 3_600_000 + 1_000, t, || true));
        assert!(same_moment(t - 5 * 3_600_000 - 1_800_000, t, || true), "a half-hour zone");
        assert!(!same_moment(t + 3_600_000, t, || false));
        assert!(!same_moment(t + 3_600_000 + 60_000, t, || true), "not a whole quarter hour");
        assert!(same_moment(t - 15 * 3_600_000, t, || true), "UTC+8 to UTC-7");
        assert!(!same_moment(t + 27 * 3_600_000, t, || true), "more than any zone change");
    }
}
