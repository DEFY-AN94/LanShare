//! Chat folders stay the computer's alone: those of every shared folder used
//! before, for good, and only as plain folders.

use super::*;

/// A phone sends a file while `archives/old` is the shared folder; then the
/// original folder, which holds that one, is shared again. Returns the
/// earlier folder as it was recorded.
async fn earlier_chat_file(e: &Env, cookie: &str) -> (std::path::PathBuf, i64) {
    let shared = e.state.sandbox().root().to_path_buf();
    e.state.set_sandbox(Sandbox::new(&shared.join("archives").join("old")).unwrap());
    let earlier = e.state.sandbox().root().to_path_buf();
    let r = send(&e.lan, dev_req(Method::POST, "/api/messages/upload?name=private.txt", Some(cookie), Body::from("private attachment"), false)).await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.json());
    // In the table itself, not just named by the message: it outlives the message.
    let recorded: bool = rusqlite::Connection::open(e.state.data_dir.join("lanshare.db"))
        .unwrap()
        .query_row("SELECT COUNT(*) FROM chat_roots WHERE root = ?1", [earlier.to_string_lossy()], |r| r.get::<_, i64>(0))
        .unwrap()
        == 1;
    assert!(recorded, "recorded with its first chat file");
    e.state.set_sandbox(Sandbox::existing(&shared).unwrap());
    (earlier, r.json()["id"].as_i64().unwrap())
}

fn json_req(cookie: &str, route: &str, body: Value) -> Request<Body> {
    dev_req(Method::POST, route, Some(cookie), Body::from(body.to_string()), true)
}

#[tokio::test]
async fn phones_cannot_move_or_delete_folders_holding_earlier_chat_files() {
    let e = env();
    let cookie = pair(&e).await;
    let (earlier, _) = earlier_chat_file(&e, &cookie).await;
    for path in ["archives", "archives/old"] {
        for (route, body) in [("/api/files/rename", json!({ "path": path, "name": "moved" })), ("/api/files/delete", json!({ "path": path }))] {
            let r = send(&e.lan, json_req(&cookie, route, body)).await;
            assert_eq!(r.status, StatusCode::FORBIDDEN, "{route} {path}");
            assert!(r.json()["error"].as_str().unwrap().contains("只能在电脑上移动或删除"), "{route} {path}");
        }
    }
    assert_eq!(std::fs::read_to_string(earlier.join("收件箱/private.txt")).unwrap(), "private attachment");
    // The folders can still be browsed, and other folders renamed and deleted.
    let list = send(&e.lan, dev_req(Method::GET, "/api/files?path=archives%2Fold", Some(&cookie), Body::empty(), false)).await;
    assert_eq!(list.status, StatusCode::OK);
    assert!(!names(&list.json()).contains(&"收件箱".to_string()));
    std::fs::create_dir_all(e.state.sandbox().root().join("archives/other")).unwrap();
    let r = send(&e.lan, json_req(&cookie, "/api/files/rename", json!({ "path": "archives/other", "name": "renamed" }))).await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.json());
    // The computer decides for itself.
    let r = send(&e.local, host_req(Method::POST, "/api/files/rename", Some(json!({ "path": "archives", "name": "moved-by-the-computer" })))).await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.json());
}

#[tokio::test]
async fn earlier_chat_folders_stay_closed_after_every_cleanup_and_restart() {
    for cleanup in ["delete the message", "clear all chats", "automatic cleanup", "remove the device"] {
        let e = env();
        let cookie = pair(&e).await;
        let device = device_id(&e, &cookie).await;
        let (earlier, id) = earlier_chat_file(&e, &cookie).await;
        match cleanup {
            "delete the message" => assert_eq!(send(&e.local, host_req(Method::DELETE, &format!("/api/messages/{id}"), None)).await.status, StatusCode::NO_CONTENT),
            "clear all chats" => assert_eq!(send(&e.local, host_req(Method::POST, "/api/host/clear-messages", Some(json!({})))).await.status, StatusCode::NO_CONTENT),
            "automatic cleanup" => assert_eq!(e.state.db.delete_messages_before(i64::MAX).unwrap(), 1),
            _ => assert_eq!(send(&e.local, host_req(Method::DELETE, &format!("/api/host/devices/{device}"), None)).await.status, StatusCode::NO_CONTENT),
        }
        let Env { _dir: dir, .. } = e;
        let e = env_in(dir);
        let cookie = pair(&e).await;
        let file = "/api/files/download?path=archives%2Fold%2F%E6%94%B6%E4%BB%B6%E7%AE%B1%2Fprivate.txt"; // archives/old/收件箱/private.txt
        assert_eq!(send(&e.lan, dev_req(Method::GET, file, Some(&cookie), Body::empty(), false)).await.status, StatusCode::FORBIDDEN, "{cleanup}");
        let list = send(&e.lan, dev_req(Method::GET, "/api/files?path=archives%2Fold", Some(&cookie), Body::empty(), false)).await.json();
        assert!(!names(&list).contains(&"收件箱".to_string()), "{cleanup}");
        let r = send(&e.lan, json_req(&cookie, "/api/files/rename", json!({ "path": "archives", "name": "moved" }))).await;
        assert_eq!(r.status, StatusCode::FORBIDDEN, "{cleanup}");
        assert!(earlier.join("收件箱/private.txt").exists(), "{cleanup}");
    }
}

/// Chat files saved by an older version, whose messages are gone, leave only
/// their folders: a shared folder that has them is recorded at startup. A
/// folder left during a run stays protected for the rest of it.
#[tokio::test]
async fn shared_folders_with_chat_folders_are_recorded_at_startup() {
    let e = env();
    let root = e.state.sandbox().root().to_path_buf();
    assert!(e.state.db.chat_roots().unwrap().is_empty(), "nothing sent yet");
    std::fs::create_dir(root.join("已发送")).unwrap();
    let Env { _dir: dir, .. } = e;
    let e = env_in(dir);
    assert_eq!(e.state.db.chat_roots().unwrap(), [root.to_string_lossy().into_owned()]);

    e.state.set_sandbox(Sandbox::new(&root.join("other")).unwrap());
    let other = e.state.sandbox().root().to_path_buf();
    e.state.set_sandbox(Sandbox::existing(&root).unwrap());
    assert!(e.state.is_chat_location(&other.join("收件箱").join("x.txt")));
}

/// Earlier versions followed a chat folder that was a link and saved
/// attachments in the folder it pointed to: that folder stays the computer's.
#[tokio::test]
async fn attachments_an_earlier_version_saved_through_a_link_stay_private() {
    let e = env();
    let cookie = pair(&e).await;
    let device = device_id(&e, &cookie).await;
    let root = e.state.sandbox().root().to_path_buf();
    std::fs::create_dir(root.join("Phone uploads")).unwrap();
    std::fs::write(root.join("Phone uploads").join("old.jpg"), b"jpeg").unwrap();
    e.state
        .db
        .insert_message(&NewMessage {
            device_id: &device,
            from_host: false,
            client_id: None,
            kind: "file",
            text: None,
            file_path: Some("Phone uploads/old.jpg"),
            file_name: Some("old.jpg"),
            file_size: Some(4),
            mime: None,
            created_at: 1,
            file_root: Some(root.to_str().unwrap()),
            file_created: None,
        })
        .unwrap();
    let Env { _dir: dir, .. } = e;
    let e = env_in(dir);
    let other = pair_named(&e, "Test iPad B").await;
    let get = |uri: &str| dev_req(Method::GET, uri, Some(&other), Body::empty(), false);
    assert_eq!(send(&e.lan, get("/api/files/download?path=Phone%20uploads%2Fold.jpg")).await.status, StatusCode::FORBIDDEN);
    assert!(!names(&send(&e.lan, get("/api/files")).await.json()).contains(&"Phone uploads".to_string()));
}

/// Long Windows paths come verbatim (`\\?\C:\...`); recorded folders do not.
#[cfg(windows)]
#[tokio::test]
async fn verbatim_windows_paths_are_recognised() {
    let e = env();
    let cookie = pair(&e).await;
    let (earlier, _) = earlier_chat_file(&e, &cookie).await;
    let verbatim = |p: &std::path::Path| std::path::PathBuf::from(format!(r"\\?\{}", p.display()));
    assert!(e.state.is_chat_location(&verbatim(&earlier.join("收件箱").join("private.txt"))));
    assert!(e.state.holds_chat_folder(&verbatim(earlier.parent().unwrap())));
}

#[cfg(unix)]
#[tokio::test]
async fn a_linked_chat_folder_is_refused_not_followed() {
    let e = env();
    let cookie = pair(&e).await;
    let root = e.state.sandbox().root().to_path_buf();
    std::fs::create_dir(root.join("ordinary-looking")).unwrap();
    std::os::unix::fs::symlink("ordinary-looking", root.join("收件箱")).unwrap();
    let upload = || dev_req(Method::POST, "/api/messages/upload?name=secret.txt", Some(&cookie), Body::from("secret"), false);
    let check = || dev_req(Method::GET, "/api/upload-check?size=6", Some(&cookie), Body::empty(), false);
    // Before sending: the phone hears why.
    let r = send(&e.lan, check()).await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    assert!(r.json()["error"].as_str().unwrap().contains("不是普通文件夹"), "{:?}", r.json());
    let r = send(&e.lan, upload()).await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    assert!(r.json()["error"].as_str().unwrap().contains("不是普通文件夹"), "{:?}", r.json());
    assert_eq!(std::fs::read_dir(root.join("ordinary-looking")).unwrap().count(), 0, "nothing landed in the folder phones can browse");
    // A link to nothing, or to a file, gets the same answer.
    std::fs::remove_file(root.join("收件箱")).unwrap();
    std::os::unix::fs::symlink("gone", root.join("收件箱")).unwrap();
    assert!(send(&e.lan, upload()).await.json()["error"].as_str().unwrap().contains("不是普通文件夹"));
    // Once the computer puts a plain folder back, the phone can send again.
    std::fs::remove_file(root.join("收件箱")).unwrap();
    assert_eq!(send(&e.lan, check()).await.status, StatusCode::NO_CONTENT);
    assert_eq!(send(&e.lan, upload()).await.status, StatusCode::OK);
    assert!(root.join("收件箱/secret.txt").is_file());
}
