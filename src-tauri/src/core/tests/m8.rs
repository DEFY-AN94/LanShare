//! M8: notifications you can act on, unread counts, and quick copy between
//! the computer and the phone. The shell side is a `RecordingBridge`, so
//! everything but the native APIs is tested without a window.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use super::*;
use crate::core::bridge::{Clip, HostBridge, MessageNotice, Withdraw};
use crate::core::quick;

/// Records what the server asks of the desktop shell.
#[derive(Default)]
struct RecordingBridge {
    notices: Mutex<Vec<MessageNotice>>,
    plain: Mutex<Vec<(String, String)>>,
    withdrawn: Mutex<Vec<Withdraw>>,
    unread: Mutex<Vec<u32>>,
    shown: Mutex<Vec<String>>,
    clipboard: Mutex<Option<Clip>>,
    copied: Mutex<Vec<String>>,
    opened: Mutex<Vec<(&'static str, PathBuf)>>,
    focused: AtomicBool,
    shortcut: Mutex<Option<String>>,
    refuse_shortcuts: AtomicBool,
    login_item: Mutex<Option<&'static str>>,
}

impl HostBridge for RecordingBridge {
    fn notify(&self, title: &str, body: &str) {
        self.plain.lock().unwrap().push((title.into(), body.into()));
    }
    fn notify_message(&self, n: &MessageNotice) {
        self.notices.lock().unwrap().push(n.clone());
    }
    fn withdraw_notifications(&self, w: &Withdraw) {
        self.withdrawn.lock().unwrap().push(w.clone());
    }
    fn window_focused(&self) -> bool {
        self.focused.load(Ordering::SeqCst)
    }
    fn set_unread(&self, total: u32) {
        self.unread.lock().unwrap().push(total);
    }
    fn show_conversation(&self, device_id: &str) {
        self.shown.lock().unwrap().push(device_id.into());
    }
    fn read_clipboard(&self) -> Clip {
        self.clipboard.lock().unwrap().clone().unwrap_or(Clip::Empty)
    }
    fn write_clipboard_text(&self, text: &str) -> Result<(), String> {
        self.copied.lock().unwrap().push(text.into());
        Ok(())
    }
    fn set_shortcut(&self, accelerator: Option<&str>) -> Result<(), String> {
        if self.refuse_shortcuts.load(Ordering::SeqCst) {
            return Err("另一个 App 已经在用".into());
        }
        *self.shortcut.lock().unwrap() = accelerator.map(str::to_string);
        Ok(())
    }
    fn open_path(&self, path: &std::path::Path) -> Result<(), String> {
        self.opened.lock().unwrap().push(("open", path.to_path_buf()));
        Ok(())
    }
    fn reveal_path(&self, path: &std::path::Path) -> Result<(), String> {
        self.opened.lock().unwrap().push(("reveal", path.to_path_buf()));
        Ok(())
    }
    fn open_url(&self, _url: &str) -> Result<(), String> {
        Ok(())
    }
    fn pick_folder(&self) -> Option<PathBuf> {
        None
    }
    fn login_item(&self) -> &'static str {
        self.login_item.lock().unwrap().unwrap_or("off")
    }
    fn set_login_item(&self, on: bool) -> Result<&'static str, String> {
        let state = if on { "on" } else { "off" };
        *self.login_item.lock().unwrap() = Some(state);
        Ok(state)
    }
}

/// The names of the events `rx` has received so far.
fn heard(rx: &mut tokio::sync::broadcast::Receiver<crate::core::events::ServerEvent>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        out.push((ev.name.to_string(), ev.data.to_string()));
    }
    out
}

fn recorded() -> (Env, Arc<RecordingBridge>) {
    recorded_in(tempfile::tempdir().unwrap())
}

fn recorded_in(dir: tempfile::TempDir) -> (Env, Arc<RecordingBridge>) {
    let bridge = Arc::new(RecordingBridge::default());
    (env_bridged(dir, None, bridge.clone()), bridge)
}

async fn phone_says(e: &Env, cookie: &str, text: &str) -> i64 {
    let r = send(&e.lan, dev_req(Method::POST, "/api/messages", Some(cookie), Body::from(json!({ "text": text }).to_string()), true)).await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.json());
    r.json()["id"].as_i64().unwrap()
}

async fn phone_sends_file(e: &Env, cookie: &str, name: &str, bytes: &'static [u8]) -> i64 {
    let uri = format!("/api/messages/upload?name={name}&size={}", bytes.len());
    let r = send(&e.lan, dev_req(Method::POST, &uri, Some(cookie), Body::from(bytes), false)).await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.json());
    r.json()["id"].as_i64().unwrap()
}

async fn unread_of(e: &Env, device: &str) -> u64 {
    let list = send(&e.local, host_req(Method::GET, "/api/host/devices", None)).await.json();
    list.as_array().unwrap().iter().find(|d| d["id"] == device).map(|d| d["unread"].as_u64().unwrap()).unwrap()
}

async fn viewing(e: &Env, device: Option<&str>) {
    let r = send(&e.local, host_req(Method::POST, "/api/host/viewing", Some(json!({ "device": device })))).await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn phone_messages_notify_in_bursts_and_count_as_unread() {
    let (e, bridge) = recorded();
    let cookie = pair(&e).await;
    let id = device_id(&e, &cookie).await;
    for text in ["one", "two", "three", "four"] {
        phone_says(&e, &cookie, text).await;
    }
    let notices = bridge.notices.lock().unwrap().clone();
    let shape: Vec<(bool, bool, Option<&str>)> = notices.iter().map(|n| (n.identifier.starts_with("msg."), n.quiet, n.subtitle.as_deref())).collect();
    assert_eq!(shape, [(true, false, None), (false, false, None), (false, true, Some("还有 1 条新消息")), (false, true, Some("还有 2 条新消息"))]);
    assert!(notices.iter().all(|n| n.title == "iPhone" && n.device_id == id));
    assert_eq!(notices[3].body, "four", "the summary shows the newest message");
    assert_eq!(unread_of(&e, &id).await, 4);
    assert_eq!(bridge.unread.lock().unwrap().last(), Some(&4));

    // Opening the conversation in the window reads it and takes its notifications back.
    viewing(&e, Some(&id)).await;
    assert_eq!(unread_of(&e, &id).await, 0);
    assert_eq!(bridge.withdrawn.lock().unwrap().last(), Some(&Withdraw::Device(id.clone())));
    assert_eq!(bridge.unread.lock().unwrap().last(), Some(&0));
    // While it is open and in front, new messages are read at once and raise no banner.
    bridge.focused.store(true, Ordering::SeqCst);
    let before = bridge.notices.lock().unwrap().len();
    phone_says(&e, &cookie, "seen").await;
    assert_eq!(unread_of(&e, &id).await, 0);
    assert_eq!(bridge.notices.lock().unwrap().len(), before);
    // Away from it, they count again, and a read conversation starts a new burst.
    bridge.focused.store(false, Ordering::SeqCst);
    viewing(&e, None).await;
    phone_says(&e, &cookie, "later").await;
    assert_eq!(unread_of(&e, &id).await, 1);
    assert!(bridge.notices.lock().unwrap().last().unwrap().identifier.starts_with("msg."));
    // The computer's own messages are never unread.
    let r = send(&e.local, host_req(Method::POST, "/api/messages", Some(json!({ "device": id, "text": "reply" })))).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(unread_of(&e, &id).await, 1);

    // The count survives a restart.
    let Env { _dir: dir, .. } = e;
    let (e, _) = recorded_in(dir);
    assert_eq!(unread_of(&e, &id).await, 1);
}

#[tokio::test]
async fn hidden_text_and_withdrawn_notifications() {
    let (e, bridge) = recorded();
    let cookie = pair(&e).await;
    let id = device_id(&e, &cookie).await;
    let r = send(&e.local, host_req(Method::POST, "/api/host/notifications", Some(json!({ "hide_text": true })))).await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    assert_eq!(bridge.withdrawn.lock().unwrap().last(), Some(&Withdraw::All), "texts already shown go too");
    let first = phone_says(&e, &cookie, "a password, HIDDEN-MARKER").await;
    phone_sends_file(&e, &cookie, "plan.pdf", b"%PDF").await;
    let notices = bridge.notices.lock().unwrap().clone();
    assert_eq!(notices[0].body, "发来一条消息");
    assert_eq!(notices[1].body, "发来一个文件");
    assert!(notices.iter().all(|n| !n.body.contains("HIDDEN-MARKER") && !n.body.contains("plan.pdf")));
    let overview = send(&e.local, host_req(Method::GET, "/api/host/overview", None)).await.json();
    assert_eq!(overview["notify_hide_text"], true);

    // A deleted message takes its notification back; so do a clear and a removed device.
    let r = send(&e.local, host_req(Method::DELETE, &format!("/api/messages/{first}"), None)).await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    assert_eq!(bridge.withdrawn.lock().unwrap().last(), Some(&Withdraw::Ids(vec![format!("msg.{first}")])));
    assert_eq!(unread_of(&e, &id).await, 1, "the deleted message is no longer unread");
    send(&e.local, host_req(Method::POST, "/api/host/clear-messages", Some(json!({})))).await;
    assert_eq!(bridge.withdrawn.lock().unwrap().last(), Some(&Withdraw::All));
    assert_eq!(unread_of(&e, &id).await, 0);
    send(&e.local, host_req(Method::DELETE, &format!("/api/host/devices/{id}"), None)).await;
    assert_eq!(bridge.withdrawn.lock().unwrap().last(), Some(&Withdraw::Device(id)));
}

#[tokio::test]
async fn the_clipboard_goes_to_the_most_recent_conversation() {
    let (e, bridge) = recorded();
    let state = &e.state;
    *bridge.clipboard.lock().unwrap() = Some(Clip::Text("https://example.com/a".into()));
    assert_eq!(quick::send_clipboard(state, None).await.unwrap_err(), "还没有配对的设备");

    let a = pair_named(&e, "A").await;
    let b = pair_named(&e, "B").await;
    let (a_id, b_id) = (device_id(&e, &a).await, device_id(&e, &b).await);
    phone_says(&e, &a, "from A").await;
    phone_says(&e, &b, "from B, last").await;
    let (name, msg) = quick::send_clipboard(state, None).await.unwrap();
    assert_eq!((name.as_str(), msg.device_id.as_str(), msg.from_host, msg.text.as_deref()), ("B", b_id.as_str(), true, Some("https://example.com/a")));
    let page = send(&e.lan, dev_req(Method::GET, "/api/messages", Some(&b), Body::empty(), false)).await.json();
    assert_eq!(page["messages"].as_array().unwrap().last().unwrap()["text"], "https://example.com/a", "it reaches the phone");
    assert!(bridge.notices.lock().unwrap().iter().all(|n| n.body != "https://example.com/a"), "the computer's own message raises no notification");

    // Settings can name a device; a device that is gone falls back to the most recent one.
    let r = send(&e.local, host_req(Method::POST, "/api/host/clipboard/target", Some(json!({ "device": a_id })))).await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    assert_eq!(quick::send_clipboard(state, None).await.unwrap().1.device_id, a_id);
    assert_eq!(quick::send_clipboard(state, Some(&b_id)).await.unwrap().1.device_id, b_id, "a device picked in the menu wins");
    send(&e.local, host_req(Method::DELETE, &format!("/api/host/devices/{a_id}"), None)).await;
    assert_eq!(quick::send_clipboard(state, None).await.unwrap().1.device_id, b_id);
    let bad = send(&e.local, host_req(Method::POST, "/api/host/clipboard/target", Some(json!({ "device": "nope" })))).await;
    assert_eq!(bad.status, StatusCode::NOT_FOUND);

    // A picture arrives as a PNG under 已发送.
    let png: Vec<u8> = [b"\x89PNG\r\n\x1a\n".as_slice(), b"pixels"].concat();
    *bridge.clipboard.lock().unwrap() = Some(Clip::Png(png.clone()));
    let mut rx = e.state.events.subscribe();
    let (_, pic) = quick::send_clipboard(state, None).await.unwrap();
    let file = pic.file.as_ref().unwrap();
    assert!(file.path.starts_with("已发送/截图 ") && file.path.ends_with(".png"), "{}", file.path);
    assert_eq!(std::fs::read(e.state.sandbox().root().join(&file.path)).unwrap(), png);
    let mut saw_msg = false;
    while let Ok(ev) = rx.try_recv() {
        saw_msg |= ev.name == "msg" && ev.data.contains(&b_id);
    }
    assert!(saw_msg, "the phone hears about it live");

    // Nothing usable on the clipboard: nothing is sent.
    let count = |e: &Env| e.state.db.messages(&b_id, None, None, 200).unwrap().0.len();
    let before = count(&e);
    *bridge.clipboard.lock().unwrap() = Some(Clip::Empty);
    assert_eq!(quick::send_clipboard(state, None).await.unwrap_err(), "剪贴板是空的，没有发送");
    *bridge.clipboard.lock().unwrap() = Some(Clip::Unsupported("剪贴板里是文件".into()));
    assert_eq!(quick::send_clipboard(state, None).await.unwrap_err(), "剪贴板里是文件");
    *bridge.clipboard.lock().unwrap() = Some(Clip::Text("   ".into()));
    assert!(quick::send_clipboard(state, None).await.is_err());
    assert_eq!(count(&e), before);
    quick::report(state, quick::send_clipboard(state, None).await);
    assert_eq!(bridge.plain.lock().unwrap().last().map(|(t, _)| t.as_str()), Some("LanShare"), "errors show as a notification");

    // With the window in front a notification would not show: the window says it.
    bridge.focused.store(true, Ordering::SeqCst);
    let notified = bridge.plain.lock().unwrap().len();
    let mut rx = e.state.events.subscribe();
    quick::send_clipboard_and_tell(state, None).await;
    *bridge.clipboard.lock().unwrap() = Some(Clip::Text("sent while in front".into()));
    quick::send_clipboard_and_tell(state, None).await;
    let notices: Vec<String> = heard(&mut rx).into_iter().filter(|(n, _)| n == "notice").map(|(_, d)| d).collect();
    assert_eq!(notices.len(), 2, "{notices:?}");
    assert!(notices[0].contains("\"error\":true") && notices[1].contains("已把剪贴板里的文字发送到「B」"), "{notices:?}");
    assert_eq!(bridge.plain.lock().unwrap().len(), notified);
}

#[tokio::test]
async fn pictures_copied_as_files_are_sent_other_files_are_not() {
    let (e, bridge) = recorded();
    let cookie = pair(&e).await;
    let id = device_id(&e, &cookie).await;
    let outside = tempfile::tempdir().unwrap();
    let jpeg = outside.path().join("9e9059c7.jpeg");
    let png = outside.path().join("Photo 1.PNG");
    let pdf = outside.path().join("plan.pdf");
    std::fs::write(&jpeg, b"\xFF\xD8\xFFjpeg").unwrap();
    std::fs::write(&png, b"\x89PNGpng").unwrap();
    std::fs::write(&pdf, b"%PDF").unwrap();
    let count = |e: &Env| e.state.db.messages(&id, None, None, 200).unwrap().0.len();

    // A picture copied in QQ, or pictures copied in Finder: the files themselves, by name.
    *bridge.clipboard.lock().unwrap() = Some(Clip::Files { paths: vec![jpeg.clone(), png.clone()], image: None });
    let (_, last) = quick::send_clipboard(&e.state, None).await.unwrap();
    assert_eq!(count(&e), 2);
    assert_eq!(last.file.as_ref().unwrap().name, "Photo 1.PNG");
    let sent = e.state.sandbox().root().join(&last.file.as_ref().unwrap().path);
    assert_eq!(std::fs::read(sent).unwrap(), b"\x89PNGpng");

    // Any other file, alone or among pictures, is not sent.
    for files in [vec![pdf.clone()], vec![jpeg.clone(), pdf.clone()], vec![]] {
        *bridge.clipboard.lock().unwrap() = Some(Clip::Files { paths: files, image: Some(b"\x89PNGx".to_vec()) });
        assert!(quick::send_clipboard(&e.state, None).await.unwrap_err().contains("剪贴板里是文件"));
    }
    let gone = outside.path().join("9e9059c7-gone.jpeg");
    *bridge.clipboard.lock().unwrap() = Some(Clip::Files { paths: vec![gone.clone()], image: None });
    assert_eq!(quick::send_clipboard(&e.state, None).await.unwrap_err(), "复制的图片已经不在了");
    assert_eq!(count(&e), 2);

    // QQ's own folder is closed to other apps: the picture the clipboard also holds goes instead.
    *bridge.clipboard.lock().unwrap() = Some(Clip::Files { paths: vec![gone], image: Some(b"\x89PNGqq".to_vec()) });
    let (_, sent) = quick::send_clipboard(&e.state, None).await.unwrap();
    assert_eq!(sent.file.as_ref().unwrap().name, "9e9059c7-gone.png");
    assert_eq!(std::fs::read(e.state.sandbox().root().join(&sent.file.as_ref().unwrap().path)).unwrap(), b"\x89PNGqq");
}

#[tokio::test]
async fn notification_buttons_copy_open_and_show() {
    let (e, bridge) = recorded();
    let state = &e.state;
    let cookie = pair(&e).await;
    let id = device_id(&e, &cookie).await;
    let long = "长".repeat(300);
    let text_id = phone_says(&e, &cookie, &long).await;
    assert_eq!(bridge.notices.lock().unwrap()[0].body.chars().count(), 81, "the notification shows a preview");

    quick::on_notification(state, quick::ACTION_COPY, Some(text_id), Some(&id)).await;
    assert_eq!(bridge.copied.lock().unwrap().last().map(|t| t.chars().count()), Some(300), "复制 copies the full text");

    // 复制最近收到的文字 skips the computer's own messages.
    send(&e.local, host_req(Method::POST, "/api/messages", Some(json!({ "device": id, "text": "from the computer" })))).await;
    assert_eq!(quick::copy_latest_received(state).unwrap(), "iPhone");
    assert_eq!(bridge.copied.lock().unwrap().last().unwrap(), &long);

    // 打开: a document opens; something that would run or install is only shown in its folder.
    let pdf = phone_sends_file(&e, &cookie, "plan.pdf", b"%PDF").await;
    let exe = phone_sends_file(&e, &cookie, "setup.exe", b"MZ").await;
    let profile = phone_sends_file(&e, &cookie, "wifi.mobileconfig", b"<?xml").await;
    quick::on_notification(state, quick::ACTION_OPEN, Some(pdf), Some(&id)).await;
    quick::on_notification(state, quick::ACTION_OPEN, Some(exe), Some(&id)).await;
    quick::on_notification(state, quick::ACTION_OPEN, Some(profile), Some(&id)).await;
    let opened = bridge.opened.lock().unwrap().clone();
    assert_eq!(opened.iter().map(|(how, p)| (*how, p.file_name().unwrap().to_string_lossy().into_owned())).collect::<Vec<_>>(), [
        ("open", "plan.pdf".to_string()),
        ("reveal", "setup.exe".to_string()),
        ("reveal", "wifi.mobileconfig".to_string())
    ]);
    for msg in [exe, profile] {
        let r = send(&e.local, host_req(Method::POST, &format!("/api/host/messages/{msg}/open"), Some(json!({})))).await;
        assert_eq!(r.json()["action"], "revealed");
    }
    // 在文件夹中显示 finds the file by its message too.
    let r = send(&e.local, host_req(Method::POST, &format!("/api/host/messages/{pdf}/reveal"), Some(json!({})))).await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    let last = bridge.opened.lock().unwrap().last().cloned().unwrap();
    assert_eq!((last.0, last.1.file_name().unwrap().to_string_lossy().into_owned()), ("reveal", "plan.pdf".to_string()));
    let r = send(&e.local, host_req(Method::POST, &format!("/api/host/messages/{text_id}/reveal"), Some(json!({})))).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND, "a text has no file");

    // A click opens that conversation.
    quick::on_notification(state, quick::ACTION_SHOW, Some(text_id), None).await;
    assert_eq!(bridge.shown.lock().unwrap().last(), Some(&id));

    // A message deleted since: the button says so instead.
    send(&e.local, host_req(Method::DELETE, &format!("/api/messages/{text_id}"), None)).await;
    quick::on_notification(state, quick::ACTION_COPY, Some(text_id), Some(&id)).await;
    assert_eq!(bridge.plain.lock().unwrap().last().map(|(_, b)| b.as_str()), Some("这条消息已被删除"));
    quick::on_notification(state, quick::ACTION_COPY, Some(pdf), Some(&id)).await;
    assert_eq!(bridge.plain.lock().unwrap().last().map(|(_, b)| b.as_str()), Some("这条消息不是文字"));
}

#[tokio::test]
async fn a_refused_shortcut_keeps_the_old_one() {
    let (e, bridge) = recorded();
    let set = |s: Option<&str>| host_req(Method::POST, "/api/host/clipboard/shortcut", Some(json!({ "shortcut": s })));
    assert_eq!(send(&e.local, set(Some("Control+Shift+KeyV"))).await.status, StatusCode::NO_CONTENT);
    assert_eq!(bridge.shortcut.lock().unwrap().as_deref(), Some("Control+Shift+KeyV"));
    // A slip such as ⌘V would take over pasting everywhere.
    let r = send(&e.local, set(Some("Super+KeyV"))).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert!(r.json()["error"].as_str().unwrap().contains("两个修饰键"));
    bridge.refuse_shortcuts.store(true, Ordering::SeqCst);
    let r = send(&e.local, set(Some("Control+Shift+KeyC"))).await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    assert!(r.json()["error"].as_str().unwrap().contains("另一个 App 已经在用"));
    let overview = send(&e.local, host_req(Method::GET, "/api/host/overview", None)).await.json();
    assert_eq!(overview["clipboard_shortcut"], "Control+Shift+KeyV", "the old one stays");
    bridge.refuse_shortcuts.store(false, Ordering::SeqCst);
    assert_eq!(send(&e.local, set(None)).await.status, StatusCode::NO_CONTENT);
    assert_eq!(bridge.shortcut.lock().unwrap().as_deref(), None);
    let overview = send(&e.local, host_req(Method::GET, "/api/host/overview", None)).await.json();
    assert_eq!(overview["clipboard_shortcut"], Value::Null);
}

#[tokio::test]
async fn pressing_the_shortcut_while_settings_records_one_sends_nothing() {
    let (e, bridge) = recorded();
    let cookie = pair(&e).await;
    let id = device_id(&e, &cookie).await;
    *bridge.clipboard.lock().unwrap() = Some(Clip::Text("maybe a password".into()));
    let count = |e: &Env| e.state.db.messages(&id, None, None, 200).unwrap().0.len();
    let recording = |on: bool| host_req(Method::POST, "/api/host/clipboard/shortcut/recording", Some(json!({ "on": on })));
    assert_eq!(send(&e.local, recording(true)).await.status, StatusCode::NO_CONTENT);
    let mut rx = e.state.events.subscribe();
    quick::shortcut_pressed(&e.state).await;
    assert_eq!(count(&e), 0, "nothing was sent");
    assert!(heard(&mut rx).iter().any(|(n, _)| n == "shortcut_pressed"), "Settings hears it");
    send(&e.local, recording(false)).await;
    quick::shortcut_pressed(&e.state).await;
    assert_eq!(count(&e), 1);
}

#[tokio::test]
async fn launch_at_login_is_remembered_until_turned_off_elsewhere() {
    let (e, bridge) = recorded();
    let set = |on: bool| host_req(Method::POST, "/api/host/login-item", Some(json!({ "on": on })));
    assert_eq!(send(&e.local, set(true)).await.json()["state"], "on");
    assert!(e.state.login_item_chosen(), "an upgrade on Windows puts it back");
    assert_eq!(send(&e.local, host_req(Method::GET, "/api/host/login-item", None)).await.json()["state"], "on");
    assert!(e.state.login_item_chosen());
    // Turned off in System Settings or by another tool: not put back.
    *bridge.login_item.lock().unwrap() = Some("off");
    send(&e.local, host_req(Method::GET, "/api/host/login-item", None)).await;
    assert!(!e.state.login_item_chosen());
    send(&e.local, set(true)).await;
    send(&e.local, set(false)).await;
    assert!(!e.state.login_item_chosen());

    // Windows puts it back only on the first start after an install.
    assert!(e.state.note_install("1000"), "nothing ran before");
    assert!(!e.state.note_install("1000"), "the same install again");
    let Env { _dir: dir, .. } = e;
    let (e, _) = recorded_in(dir);
    assert!(!e.state.note_install("1000"), "remembered across a restart");
    assert!(e.state.note_install("2000"), "installed again");
}

/// The quick actions never put message text, clipboard content or file names in the log.
#[tokio::test]
async fn quick_actions_keep_content_out_of_the_log() {
    let logs = captured_logs();
    let (e, bridge) = recorded();
    let cookie = pair(&e).await;
    let id = device_id(&e, &cookie).await;
    let text_id = phone_says(&e, &cookie, "QUICKMARK text").await;
    phone_sends_file(&e, &cookie, "QUICKMARK-file.pdf", b"%PDF").await;
    *bridge.clipboard.lock().unwrap() = Some(Clip::Text("QUICKMARK clipboard".into()));
    quick::send_clipboard(&e.state, None).await.unwrap();
    quick::on_notification(&e.state, quick::ACTION_COPY, Some(text_id), Some(&id)).await;
    quick::copy_latest_received(&e.state).unwrap();
    let log = String::from_utf8_lossy(&logs.0.lock().unwrap()).into_owned();
    assert!(!log.contains("QUICKMARK"), "{log}");
}
