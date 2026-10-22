//! What a phone message shows as a desktop notification. The shell posts it
//! (`HostBridge::notify_message`); this decides the text, the identifier
//! and how many show at once.
//!
//! Per device, the first two messages of a burst notify normally. Later ones
//! replace the second notification, quietly (no banner), with the newest
//! message and how many more there are. A burst ends after `BURST_GAP`
//! without a message from that device.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::model::MessageOut;
use super::util::preview;

/// A quiet minute ends a burst.
pub const BURST_GAP: Duration = Duration::from_secs(60);
/// The part of a message a notification shows.
const PREVIEW_CHARS: usize = 80;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeKind {
    /// Offers 复制 (Copy the full text).
    Text,
    /// Offers 打开 (Open; risky file types are only shown in the folder).
    File,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageNotice {
    /// Posting again under the same identifier replaces that notification.
    pub identifier: String,
    pub message_id: i64,
    pub device_id: String,
    pub title: String,
    /// Shown above the body, e.g. "还有 3 条新消息" (3 more new messages).
    pub subtitle: Option<String>,
    pub body: String,
    pub kind: NoticeKind,
    /// Update the list without a banner or sound.
    pub quiet: bool,
}

/// Notifications to take back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Withdraw {
    Ids(Vec<String>),
    /// Everything about one conversation.
    Device(String),
    All,
}

struct Burst {
    count: u32,
    last: Instant,
    /// The message the second, replaceable notification shows.
    shown_last: Option<i64>,
}

#[derive(Default)]
pub struct Notifier {
    bursts: Mutex<HashMap<String, Burst>>,
    /// The shell skips quiet updates (macOS 11), so the second notification
    /// keeps showing its message.
    no_quiet_updates: bool,
}

fn first_identifier(message_id: i64) -> String {
    format!("msg.{message_id}")
}

fn latest_identifier(device_id: &str) -> String {
    format!("dev.{device_id}")
}

impl Notifier {
    /// For a shell that cannot update a notification quietly (`HostBridge::quiet_updates`).
    pub fn without_quiet_updates() -> Self {
        Notifier { no_quiet_updates: true, ..Default::default() }
    }

    /// The notice for a new message from `device_name`'s phone.
    pub fn notice(&self, device_name: &str, msg: &MessageOut, hide_text: bool, now: Instant) -> MessageNotice {
        let mut bursts = self.bursts.lock().unwrap();
        let burst = bursts.entry(msg.device_id.clone()).or_insert(Burst { count: 0, last: now, shown_last: None });
        if now.duration_since(burst.last) > BURST_GAP {
            // The second notification still shows its message until a new one replaces it.
            *burst = Burst { count: 0, last: now, shown_last: burst.shown_last };
        }
        burst.count += 1;
        burst.last = now;
        let (kind, body) = match (&msg.text, &msg.file) {
            (_, Some(f)) => (NoticeKind::File, if hide_text { "发来一个文件".to_string() } else { format!("[文件] {}", f.name) }),
            (Some(t), _) => (NoticeKind::Text, if hide_text { "发来一条消息".to_string() } else { preview(t, PREVIEW_CHARS) }),
            _ => (NoticeKind::Text, String::new()),
        };
        let (identifier, subtitle, quiet) = match burst.count {
            1 => (first_identifier(msg.id), None, false),
            2 => (latest_identifier(&msg.device_id), None, false),
            n => (latest_identifier(&msg.device_id), Some(format!("还有 {} 条新消息", n - 2)), true),
        };
        if burst.count == 2 || (burst.count > 2 && !self.no_quiet_updates) {
            burst.shown_last = Some(msg.id);
        }
        MessageNotice { identifier, message_id: msg.id, device_id: msg.device_id.clone(), title: device_name.to_string(), subtitle, body, kind, quiet }
    }

    /// The notifications that show the deleted `message_ids` of one device.
    pub fn for_deleted(&self, device_id: &str, message_ids: &[i64]) -> Vec<String> {
        let mut ids: Vec<String> = message_ids.iter().map(|&id| first_identifier(id)).collect();
        let bursts = self.bursts.lock().unwrap();
        if bursts.get(device_id).and_then(|b| b.shown_last).is_some_and(|shown| message_ids.contains(&shown)) {
            ids.push(latest_identifier(device_id));
        }
        ids
    }

    /// A conversation was read or removed: its next message starts a new burst.
    pub fn forget(&self, device_id: &str) {
        self.bursts.lock().unwrap().remove(device_id);
    }

    pub fn forget_all(&self) {
        self.bursts.lock().unwrap().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::FileRef;

    fn text(id: i64, device: &str, body: &str) -> MessageOut {
        MessageOut {
            id,
            device_id: device.into(),
            from_host: false,
            client_id: None,
            kind: "text".into(),
            text: Some(body.into()),
            file: None,
            created_at: 0,
        }
    }

    #[test]
    fn two_per_burst_then_one_quiet_summary() {
        let n = Notifier::default();
        let t0 = Instant::now();
        let a = n.notice("iPhone", &text(1, "d1", "one"), false, t0);
        let b = n.notice("iPhone", &text(2, "d1", "two"), false, t0 + Duration::from_secs(1));
        let c = n.notice("iPhone", &text(3, "d1", "three"), false, t0 + Duration::from_secs(2));
        let d = n.notice("iPhone", &text(4, "d1", "four"), false, t0 + Duration::from_secs(3));
        assert_eq!((a.identifier.as_str(), a.quiet), ("msg.1", false));
        assert_eq!((b.identifier.as_str(), b.quiet), ("dev.d1", false));
        assert_eq!((c.identifier.as_str(), c.quiet, c.subtitle.as_deref()), ("dev.d1", true, Some("还有 1 条新消息")));
        assert_eq!((d.identifier.as_str(), d.body.as_str(), d.subtitle.as_deref()), ("dev.d1", "four", Some("还有 2 条新消息")));
        // Another device keeps its own count.
        assert_eq!(n.notice("iPad", &text(5, "d2", "x"), false, t0 + Duration::from_secs(3)).identifier, "msg.5");
        // After a quiet minute the next message starts a new burst.
        let e = n.notice("iPhone", &text(6, "d1", "later"), false, t0 + Duration::from_secs(3) + BURST_GAP + Duration::from_secs(1));
        assert_eq!((e.identifier.as_str(), e.quiet), ("msg.6", false));
    }

    #[test]
    fn hides_the_text_when_asked() {
        let n = Notifier::default();
        let long = "秘".repeat(200);
        let shown = n.notice("iPhone", &text(1, "d1", &long), false, Instant::now());
        assert_eq!(shown.body.chars().count(), PREVIEW_CHARS + 1, "80 characters and an ellipsis");
        assert_eq!(shown.kind, NoticeKind::Text);
        let hidden = n.notice("iPhone", &text(2, "d1", "a password"), true, Instant::now());
        assert_eq!(hidden.body, "发来一条消息");
        let mut file = text(3, "d1", "");
        file.kind = "file".into();
        file.text = None;
        file.file = Some(FileRef { path: "收件箱/a.pdf".into(), name: "a.pdf".into(), size: 1, mime: "application/pdf".into(), exists: true });
        assert_eq!(n.notice("iPhone", &file, false, Instant::now()).body, "[文件] a.pdf");
        let hidden_file = n.notice("iPhone", &file, true, Instant::now());
        assert_eq!((hidden_file.body.as_str(), hidden_file.kind), ("发来一个文件", NoticeKind::File));
    }

    #[test]
    fn deleted_messages_take_their_notifications_back() {
        let n = Notifier::default();
        let t0 = Instant::now();
        n.notice("iPhone", &text(1, "d1", "one"), false, t0);
        n.notice("iPhone", &text(2, "d1", "two"), false, t0);
        assert_eq!(n.for_deleted("d1", &[1]), ["msg.1"]);
        assert_eq!(n.for_deleted("d1", &[2]), ["msg.2", "dev.d1"], "the second notification shows message 2");
        n.forget("d1");
        assert_eq!(n.notice("iPhone", &text(3, "d1", "three"), false, t0).identifier, "msg.3", "a read conversation starts over");
    }

    #[test]
    fn without_quiet_updates_the_second_notification_keeps_its_message() {
        let n = Notifier::without_quiet_updates();
        let t0 = Instant::now();
        for id in 1..=3 {
            n.notice("iPhone", &text(id, "d1", "x"), false, t0);
        }
        assert_eq!(n.for_deleted("d1", &[2]), ["msg.2", "dev.d1"], "dev.d1 still shows message 2");
        assert_eq!(n.for_deleted("d1", &[3]), ["msg.3"]);
    }

    #[test]
    fn a_new_burst_still_knows_what_the_second_notification_shows() {
        let n = Notifier::default();
        let t0 = Instant::now();
        n.notice("iPhone", &text(1, "d1", "one"), false, t0);
        n.notice("iPhone", &text(2, "d1", "a password"), false, t0);
        let later = t0 + BURST_GAP + Duration::from_secs(1);
        assert_eq!(n.notice("iPhone", &text(3, "d1", "later"), false, later).identifier, "msg.3");
        assert_eq!(n.for_deleted("d1", &[2]), ["msg.2", "dev.d1"], "dev.d1 still shows message 2");
        n.notice("iPhone", &text(4, "d1", "and more"), false, later);
        assert_eq!(n.for_deleted("d1", &[2]), ["msg.2"], "message 4 replaced it");
    }
}
