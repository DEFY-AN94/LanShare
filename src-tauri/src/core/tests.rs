//! In-process API tests: real routers, auth and handlers, no sockets.
//! They run on every platform (including Windows CI).

use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use axum::Extension;
use serde_json::{Value, json};
use tower::ServiceExt as _;

use super::auth::Session;
use super::bridge::HeadlessBridge;
use super::config::Config;
use super::db::{Db, NewMessage};
use super::paths::Sandbox;
use super::state::{AppState, Shared, StateParts};
use super::tls::{self, Tls};
use super::{model::Address, net, routes};

const LOCAL_PORT: u16 = 53299;
const TOKEN: &str = "test-local-token";

struct Env {
    _dir: tempfile::TempDir,
    state: Shared,
    lan: Router,
    local: Router,
}

fn env() -> Env {
    env_in(tempfile::tempdir().unwrap())
}

/// Builds the server state in `dir`, reading the `config.json` and database
/// an earlier state left there (a restart).
fn env_in(dir: tempfile::TempDir) -> Env {
    env_with(dir, None)
}

/// `preferred_root`: the folder chosen in Settings, unavailable at startup.
fn env_with(dir: tempfile::TempDir, preferred_root: Option<std::path::PathBuf>) -> Env {
    env_bridged(dir, preferred_root, Arc::new(HeadlessBridge))
}

/// The same with the shell side played by `bridge`.
fn env_bridged(dir: tempfile::TempDir, preferred_root: Option<std::path::PathBuf>, bridge: Arc<dyn super::bridge::HostBridge>) -> Env {
    let data_dir = dir.path().join("data");
    let sandbox = Sandbox::new(&dir.path().join("share")).unwrap();
    let tls = Tls::init(&dir.path().join("tls"), "test", None, &[]).unwrap();
    let mut config = Config::load_or_default(&data_dir, dir.path());
    config.shared_root = sandbox.root().to_path_buf();
    config.port = 53210;
    std::fs::create_dir_all(&data_dir).unwrap();
    let db = Db::open(&data_dir.join("lanshare.db")).unwrap();
    super::state::adopt_devices(&db, &tls).unwrap();
    let state = AppState::new(StateParts {
        db,
        data_dir,
        home_dir: dir.path().to_path_buf(),
        config,
        sandbox,
        preferred_root,
        local_token: TOKEN.into(),
        local_port: LOCAL_PORT,
        lan_port: 53210,
        tls,
        bridge,
        host_name: "test-host".into(),
        addresses: vec![Address { ip: "192.168.1.5".into(), iface: "en0".into() }],
        upload_reserve: None,
    });
    // Tests must not depend on this machine's free space: a roomy disk unless a test says otherwise.
    super::disk::fake_disk(state.sandbox().root(), super::disk::DiskInfo { available: 1 << 40, total: 2 << 40, fat: false });
    let peer = ConnectInfo(SocketAddr::from(([192, 168, 1, 9], 5555)));
    let lan = routes::lan_router(state.clone()).layer(Extension(peer));
    let local = routes::local_router(state.clone()).layer(Extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 6000)))));
    Env { _dir: dir, state, lan, local }
}

struct Res {
    status: StatusCode,
    headers: HeaderMap,
    body: Bytes,
}

impl Res {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
}

async fn send(app: &Router, req: Request<Body>) -> Res {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let body = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    Res { status, headers, body }
}

fn host_req(method: Method, uri: &str, body: Option<Value>) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, format!("127.0.0.1:{LOCAL_PORT}"))
        .header(header::COOKIE, format!("ls_local={TOKEN}"))
        .header("x-lanshare", "1");
    if body.is_some() {
        b = b.header(header::CONTENT_TYPE, "application/json");
    }
    b.body(body.map(|v| Body::from(v.to_string())).unwrap_or_default()).unwrap()
}

fn dev_req(method: Method, uri: &str, cookie: Option<&str>, body: Body, json_body: bool) -> Request<Body> {
    let mut b = Request::builder().method(method).uri(uri).header(header::HOST, "192.168.1.5:53210").header("x-lanshare", "1");
    // As a browser sends it for a body of known size (a streamed body has none).
    if let Some(n) = axum::body::HttpBody::size_hint(&body).exact().filter(|&n| n > 0) {
        b = b.header(header::CONTENT_LENGTH, n);
    }
    if let Some(c) = cookie {
        b = b.header(header::COOKIE, format!("ls_dev={c}"));
    }
    if json_body {
        b = b.header(header::CONTENT_TYPE, "application/json");
    }
    b.body(body).unwrap()
}

/// Pairs a device with the 6-digit code and returns its cookie value.
async fn pair(e: &Env) -> String {
    pair_named(e, "iPhone").await
}

async fn pair_named(e: &Env, name: &str) -> String {
    let session = send(&e.local, host_req(Method::POST, "/api/host/pairing", Some(json!({})))).await.json();
    let code = session["code"].as_str().unwrap().to_string();
    let r = send(&e.lan, dev_req(Method::POST, "/api/pair/request", None, Body::from(json!({"code": code, "name": name}).to_string()), true)).await;
    assert_eq!(r.status, StatusCode::OK);
    let id = r.json()["request_id"].as_str().unwrap().to_string();
    let ok = send(&e.local, host_req(Method::POST, &format!("/api/host/pairing/requests/{id}/approve"), Some(json!({})))).await;
    assert_eq!(ok.status, StatusCode::OK);
    let st = send(&e.lan, dev_req(Method::GET, &format!("/api/pair/status/{id}"), None, Body::empty(), false)).await;
    assert_eq!(st.json()["status"], "approved");
    let set_cookie = st.headers[header::SET_COOKIE].to_str().unwrap().to_string();
    assert!(set_cookie.contains("HttpOnly") && set_cookie.contains("Secure") && set_cookie.contains("SameSite=Strict"));
    set_cookie.trim_start_matches("ls_dev=").split(';').next().unwrap().to_string()
}

#[tokio::test]
async fn pairing_messages_and_auth() {
    let e = env();
    // Unpaired phone is refused, host on loopback is accepted.
    assert_eq!(send(&e.lan, dev_req(Method::GET, "/api/me", None, Body::empty(), false)).await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(send(&e.local, host_req(Method::GET, "/api/me", None)).await.json()["kind"], "host");

    let cookie = pair(&e).await;
    let me = send(&e.lan, dev_req(Method::GET, "/api/me", Some(&cookie), Body::empty(), false)).await.json();
    assert_eq!(me["kind"], "device");
    let device = me["device_id"].as_str().unwrap().to_string();

    // A paired device cannot use host endpoints or the local login.
    assert_eq!(send(&e.lan, dev_req(Method::GET, "/api/host/devices", Some(&cookie), Body::empty(), false)).await.status, StatusCode::FORBIDDEN);
    assert_eq!(
        send(&e.lan, dev_req(Method::GET, &format!("/__local_login?token={TOKEN}"), None, Body::empty(), false)).await.status,
        StatusCode::NOT_FOUND
    );
    // The local token is useless on the LAN listener.
    let mut forged = dev_req(Method::GET, "/api/me", None, Body::empty(), false);
    forged.headers_mut().insert(header::COOKIE, format!("ls_local={TOKEN}").parse().unwrap());
    assert_eq!(send(&e.lan, forged).await.status, StatusCode::UNAUTHORIZED);

    // Messages both ways, deduped by client_id.
    let msg = || Body::from(json!({"client_id": "c-1", "text": "你好"}).to_string());
    let a = send(&e.lan, dev_req(Method::POST, "/api/messages", Some(&cookie), msg(), true)).await.json();
    let b = send(&e.lan, dev_req(Method::POST, "/api/messages", Some(&cookie), msg(), true)).await.json();
    assert_eq!(a["id"], b["id"]);
    let reply = send(&e.local, host_req(Method::POST, "/api/messages", Some(json!({"device": device, "text": "收到"})))).await;
    assert_eq!(reply.json()["from_host"], true);
    let page = send(&e.local, host_req(Method::GET, &format!("/api/messages?device={device}"), None)).await.json();
    assert_eq!(page["messages"].as_array().unwrap().len(), 2);

    // CSRF: missing header and foreign origin are refused.
    let mut no_header = dev_req(Method::POST, "/api/messages", Some(&cookie), msg(), true);
    no_header.headers_mut().remove("x-lanshare");
    assert_eq!(send(&e.lan, no_header).await.status, StatusCode::FORBIDDEN);
    let mut foreign = dev_req(Method::POST, "/api/messages", Some(&cookie), msg(), true);
    foreign.headers_mut().insert(header::ORIGIN, "https://evil.example".parse().unwrap());
    assert_eq!(send(&e.lan, foreign).await.status, StatusCode::FORBIDDEN);

    // Loopback listener rejects foreign Host headers (DNS rebinding).
    let mut rebind = host_req(Method::GET, "/api/me", None);
    rebind.headers_mut().insert(header::HOST, "evil.example:53299".parse().unwrap());
    assert_eq!(send(&e.local, rebind).await.status, StatusCode::FORBIDDEN);

    // Pause blocks the phone but not the host; revoking locks it out.
    e.state.set_paused(true);
    assert_eq!(send(&e.lan, dev_req(Method::GET, "/api/me", Some(&cookie), Body::empty(), false)).await.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(send(&e.local, host_req(Method::GET, "/api/me", None)).await.status, StatusCode::OK);
    e.state.set_paused(false);
    assert_eq!(send(&e.local, host_req(Method::DELETE, &format!("/api/host/devices/{device}"), None)).await.status, StatusCode::NO_CONTENT);
    assert_eq!(send(&e.lan, dev_req(Method::GET, "/api/me", Some(&cookie), Body::empty(), false)).await.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn files_and_sandbox() {
    let e = env();
    let cookie = pair(&e).await;
    let up = |dir: &str, name: &str, data: &'static [u8]| {
        dev_req(Method::POST, &format!("/api/files/upload?dir={dir}&name={name}"), Some(&cookie), Body::from(data), false)
    };
    let r = send(&e.lan, up("", "a.txt", b"hello world")).await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    assert_eq!(send(&e.lan, up("", "a.txt", b"x")).await.json()["name"], "a (1).txt");
    // A path in the name is flattened to its last component.
    assert_eq!(send(&e.lan, up("", "..%2F..%2Fevil.txt", b"x")).await.json()["path"], "evil.txt");

    let mk = dev_req(Method::POST, "/api/files/mkdir", Some(&cookie), Body::from(json!({"dir": "", "name": "照片"}).to_string()), true);
    assert_eq!(send(&e.lan, mk).await.status, StatusCode::OK);
    let list = send(&e.lan, dev_req(Method::GET, "/api/files?path=", Some(&cookie), Body::empty(), false)).await.json();
    assert_eq!(list["entries"][0]["name"], "照片", "folders first");

    let mut range = dev_req(Method::GET, "/api/files/download?path=a.txt", Some(&cookie), Body::empty(), false);
    range.headers_mut().insert(header::RANGE, "bytes=6-10".parse().unwrap());
    let r = send(&e.lan, range).await;
    assert_eq!(r.status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(&r.body[..], b"world");
    assert_eq!(r.headers[header::CACHE_CONTROL], "no-store", "no copy stays in the phone's browser cache");
    let r = send(&e.lan, dev_req(Method::HEAD, "/api/files/download?path=a.txt", Some(&cookie), Body::empty(), false)).await;
    assert_eq!(r.headers[header::CACHE_CONTROL], "no-store");

    for bad in ["..%2Fx", "%2E%2E", ".hidden", "a%5Cb", "C:%2Fx"] {
        let r = send(&e.lan, dev_req(Method::GET, &format!("/api/files/download?path={bad}"), Some(&cookie), Body::empty(), false)).await;
        assert!(matches!(r.status.as_u16(), 400 | 403 | 404), "{bad} -> {}", r.status);
    }

    // Script-capable files never render in our origin.
    send(&e.lan, up("", "x.html", b"<script>alert(1)</script>")).await;
    let r = send(&e.lan, dev_req(Method::GET, "/api/files/download?path=x.html&inline=1", Some(&cookie), Body::empty(), false)).await;
    assert_eq!(r.headers[header::CONTENT_TYPE], "text/plain; charset=utf-8");
    assert_eq!(r.headers[header::CONTENT_SECURITY_POLICY], "sandbox");
    assert_eq!(r.headers[header::CACHE_CONTROL], "no-store");

    // Read-only devices cannot write.
    let device = send(&e.local, host_req(Method::GET, "/api/host/devices", None)).await.json()[0]["id"].as_str().unwrap().to_string();
    send(&e.local, host_req(Method::PATCH, &format!("/api/host/devices/{device}"), Some(json!({"can_write": false})))).await;
    assert_eq!(send(&e.lan, up("", "b.txt", b"x")).await.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn attachments_never_stay_in_a_browser_cache() {
    let e = env();
    let cookie = pair(&e).await;
    let file = send(&e.lan, dev_req(Method::POST, "/api/messages/upload?name=a.jpg", Some(&cookie), Body::from(&b"jpeg"[..]), false)).await.json();
    let id = file["id"].as_i64().unwrap();
    for uri in [format!("/api/messages/{id}/file"), format!("/api/messages/{id}/file?inline=1")] {
        let r = send(&e.lan, dev_req(Method::GET, &uri, Some(&cookie), Body::empty(), false)).await;
        assert_eq!(r.status, StatusCode::OK);
        assert_eq!(r.headers[header::CACHE_CONTROL], "no-store", "phone {uri}");
        // The desktop window's web view keeps a disk cache too.
        let r = send(&e.local, host_req(Method::GET, &uri, None)).await;
        assert_eq!(r.status, StatusCode::OK);
        assert_eq!(r.headers[header::CACHE_CONTROL], "no-store", "host {uri}");
    }
    let r = send(&e.lan, dev_req(Method::GET, "/api/files/download?path=missing.txt", Some(&cookie), Body::empty(), false)).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert_eq!(r.headers[header::CACHE_CONTROL], "no-store");
}

#[tokio::test]
async fn chat_file_upload_is_idempotent() {
    let e = env();
    let cookie = pair(&e).await;
    let req = || dev_req(Method::POST, "/api/messages/upload?client_id=f-1&name=IMG_0001.HEIC", Some(&cookie), Body::from(&b"data"[..]), false);
    let a = send(&e.lan, req()).await.json();
    let b = send(&e.lan, req()).await.json();
    assert_eq!(a["id"], b["id"]);
    assert_eq!(a["file"]["path"], "收件箱/IMG_0001.HEIC");
    let id = a["id"].as_i64().unwrap();
    let r = send(&e.local, host_req(Method::GET, &format!("/api/messages/{id}/file"), None)).await;
    assert_eq!(&r.body[..], b"data");

    // Chat folders are private to conversations: phones cannot browse them.
    let root = send(&e.lan, dev_req(Method::GET, "/api/files?path=", Some(&cookie), Body::empty(), false)).await.json();
    assert!(root["entries"].as_array().unwrap().iter().all(|x| x["name"] != "收件箱"));
    let enc = "%E6%94%B6%E4%BB%B6%E7%AE%B1"; // 收件箱
    for uri in [format!("/api/files?path={enc}"), format!("/api/files/download?path={enc}%2FIMG_0001.HEIC")] {
        assert_eq!(send(&e.lan, dev_req(Method::GET, &uri, Some(&cookie), Body::empty(), false)).await.status, StatusCode::FORBIDDEN);
    }
    let host_root = send(&e.local, host_req(Method::GET, "/api/files?path=", None)).await.json();
    assert!(host_root["entries"].as_array().unwrap().iter().any(|x| x["name"] == "收件箱"));
}

async fn device_id(e: &Env, cookie: &str) -> String {
    send(&e.lan, dev_req(Method::GET, "/api/me", Some(cookie), Body::empty(), false)).await.json()["device_id"].as_str().unwrap().to_string()
}

fn device_session(id: &str) -> Session {
    Session::Device { id: id.into(), name: "phone".into(), can_write: true }
}

fn names(listing: &Value) -> Vec<String> {
    listing["entries"].as_array().map(|v| v.iter().map(|x| x["name"].as_str().unwrap_or("").to_string()).collect()).unwrap_or_default()
}

#[tokio::test]
async fn two_devices_are_kept_apart() {
    let e = env();
    let a = pair_named(&e, "A").await;
    let b = pair_named(&e, "B").await;
    let (a_id, b_id) = (device_id(&e, &a).await, device_id(&e, &b).await);
    assert_ne!(a_id, b_id);
    let mut rx = e.state.events.subscribe();

    send(&e.lan, dev_req(Method::POST, "/api/messages", Some(&a), Body::from(json!({"text": "A only"}).to_string()), true)).await;
    let file = send(&e.lan, dev_req(Method::POST, "/api/messages/upload?name=a.jpg", Some(&a), Body::from(&b"jpeg"[..]), false)).await.json();
    let file_id = file["id"].as_i64().unwrap();

    // B's conversation is empty, and A's attachment does not exist for B.
    let page = send(&e.lan, dev_req(Method::GET, "/api/messages", Some(&b), Body::empty(), false)).await.json();
    assert_eq!(page["messages"].as_array().unwrap().len(), 0);
    let r = send(&e.lan, dev_req(Method::GET, &format!("/api/messages/{file_id}/file"), Some(&b), Body::empty(), false)).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    // Naming A's conversation does not let B post into it.
    let r = send(&e.lan, dev_req(Method::POST, "/api/messages", Some(&b), Body::from(json!({"device": a_id, "text": "hi"}).to_string()), true)).await;
    assert_eq!(r.json()["device_id"], b_id);
    let page = send(&e.lan, dev_req(Method::GET, "/api/messages", Some(&a), Body::empty(), false)).await.json();
    assert_eq!(page["messages"].as_array().unwrap().len(), 2);

    // Live events about A's conversation reach A and the host, never B.
    let (sa, sb) = (device_session(&a_id), device_session(&b_id));
    let (mut a_msgs, mut inbox_events) = (0, 0);
    while let Ok(ev) = rx.try_recv() {
        if ev.name == "msg" && ev.data.contains(&a_id) {
            a_msgs += 1;
            assert!(ev.visible_to(&sa) && ev.visible_to(&Session::Host) && !ev.visible_to(&sb));
        }
        if ev.name == "files" && ev.data.contains("收件箱") {
            inbox_events += 1;
            assert!(ev.visible_to(&Session::Host) && !ev.visible_to(&sa) && !ev.visible_to(&sb));
        }
    }
    assert_eq!((a_msgs, inbox_events), (2, 1));
}

/// Symlinks (made by the computer's user) cannot open a way into 收件箱.
#[cfg(unix)]
#[tokio::test]
async fn links_into_chat_folders_stay_closed() {
    let e = env();
    let cookie = pair(&e).await;
    send(&e.lan, dev_req(Method::POST, "/api/messages/upload?name=IMG_0001.HEIC", Some(&cookie), Body::from(&b"data"[..]), false)).await;
    let root = e.state.sandbox().root().to_path_buf();
    std::fs::create_dir(root.join("照片")).unwrap();
    std::os::unix::fs::symlink(root.join("收件箱"), root.join("x")).unwrap();
    std::os::unix::fs::symlink("../收件箱", root.join("照片/y")).unwrap();
    let get = |uri: String| dev_req(Method::GET, &uri, Some(&cookie), Body::empty(), false);

    let top = send(&e.lan, get("/api/files?path=".into())).await.json();
    assert!(names(&top).contains(&"照片".to_string()) && !names(&top).contains(&"x".to_string()));
    let photos = "%E7%85%A7%E7%89%87"; // 照片
    assert!(names(&send(&e.lan, get(format!("/api/files?path={photos}"))).await.json()).is_empty());
    for uri in ["/api/files?path=x".to_string(), "/api/files/download?path=x%2FIMG_0001.HEIC".into(), format!("/api/files?path={photos}%2Fy")] {
        assert_eq!(send(&e.lan, get(uri.clone())).await.status, StatusCode::FORBIDDEN, "{uri}");
    }
    let up = dev_req(Method::POST, "/api/files/upload?dir=x&name=n.txt", Some(&cookie), Body::from(&b"x"[..]), false);
    assert_eq!(send(&e.lan, up).await.status, StatusCode::FORBIDDEN);
    let mk = |dir: &str, name: &str| {
        dev_req(Method::POST, "/api/files/mkdir", Some(&cookie), Body::from(json!({"dir": dir, "name": name}).to_string()), true)
    };
    assert_eq!(send(&e.lan, mk("x", "n")).await.status, StatusCode::FORBIDDEN);
    assert_eq!(send(&e.lan, mk("", "已发送")).await.status, StatusCode::CONFLICT);
    // Through a link to the shared folder itself, the chat folder cannot be renamed or deleted.
    std::os::unix::fs::symlink(&root, root.join("r")).unwrap();
    let post = |uri: &str, v: Value| dev_req(Method::POST, uri, Some(&cookie), Body::from(v.to_string()), true);
    assert_eq!(send(&e.lan, post("/api/files/rename", json!({"path": "r/收件箱", "name": "mine"}))).await.status, StatusCode::FORBIDDEN);
    assert_eq!(send(&e.lan, post("/api/files/delete", json!({"path": "r/收件箱"}))).await.status, StatusCode::FORBIDDEN);
    assert!(root.join("收件箱/IMG_0001.HEIC").exists());
    // The computer still sees and opens everything.
    assert!(names(&send(&e.local, host_req(Method::GET, "/api/files?path=", None)).await.json()).contains(&"x".to_string()));
    assert_eq!(send(&e.local, host_req(Method::GET, "/api/files?path=x", None)).await.status, StatusCode::OK);
}

/// After switching to a shared folder that contains the old one, the old
/// chat folders are still the host's alone.
#[tokio::test]
async fn earlier_chat_folders_stay_closed_after_a_folder_change() {
    let e = env();
    let cookie = pair(&e).await;
    send(&e.lan, dev_req(Method::POST, "/api/messages/upload?name=IMG_0001.HEIC", Some(&cookie), Body::from(&b"data"[..]), false)).await;
    let old_root = e.state.sandbox().root().to_path_buf();
    let parent = old_root.parent().unwrap().to_path_buf();
    e.state.set_sandbox(Sandbox::new(&parent).unwrap().with_deny(std::slice::from_ref(&e.state.data_dir)));
    let get = |uri: &str| dev_req(Method::GET, uri, Some(&cookie), Body::empty(), false);
    let inbox = "share%2F%E6%94%B6%E4%BB%B6%E7%AE%B1"; // share/收件箱
    assert!(!names(&send(&e.lan, get("/api/files?path=share")).await.json()).contains(&"收件箱".to_string()));
    for uri in [format!("/api/files?path={inbox}"), format!("/api/files/download?path={inbox}%2FIMG_0001.HEIC")] {
        assert_eq!(send(&e.lan, get(&uri)).await.status, StatusCode::FORBIDDEN, "{uri}");
    }
    assert_eq!(send(&e.local, host_req(Method::GET, &format!("/api/files?path={inbox}"), None)).await.status, StatusCode::OK);

    // A fresh start knows them too: attachments remember their folder.
    let parts = StateParts {
        db: Db::open_in_memory().unwrap(),
        data_dir: e.state.data_dir.clone(),
        home_dir: e.state.home_dir.clone(),
        config: Config { port: 1, ..Config::with_root(parent.clone()) },
        sandbox: Sandbox::new(&parent).unwrap(),
        preferred_root: None,
        local_token: TOKEN.into(),
        local_port: LOCAL_PORT,
        lan_port: 1,
        tls: Tls::init(&parent.join("tls"), "test", None, &[]).unwrap(),
        bridge: Arc::new(HeadlessBridge),
        host_name: "test-host".into(),
        addresses: vec![],
        upload_reserve: None,
    };
    let old = NewMessage {
        device_id: "d",
        from_host: false,
        client_id: None,
        kind: "file",
        text: None,
        file_path: Some("收件箱/IMG_0001.HEIC"),
        file_name: Some("IMG_0001.HEIC"),
        file_size: Some(4),
        mime: None,
        created_at: 1,
        file_root: Some(old_root.to_str().unwrap()),
        file_created: None,
    };
    parts.db.insert_device(&super::db::DeviceRow { id: "d".into(), name: "p".into(), token_hash: "h".into(), can_write: true, created_at: 1, last_seen_at: None, ca_fingerprint: None }).unwrap();
    parts.db.insert_message(&old).unwrap();
    let restarted = AppState::new(parts);
    assert!(restarted.is_chat_location(&old_root.join("收件箱").join("IMG_0001.HEIC")));
    assert!(!restarted.is_chat_location(&old_root.join("照片")));
}

/// The watcher sends changes in or to a chat folder to the host only.
#[test]
fn watcher_keeps_chat_folder_changes_to_the_host() {
    let e = env();
    let sandbox = e.state.sandbox();
    let root = sandbox.root().to_path_buf();
    let target = |p: std::path::PathBuf| super::change_target(&e.state, &sandbox, &p);
    assert_eq!(target(root.join("收件箱")), Some(("".into(), true)));
    assert_eq!(target(root.join("已发送")), Some(("".into(), true)));
    assert_eq!(target(root.join("收件箱/IMG_0001.HEIC")), Some(("收件箱".into(), true)));
    assert_eq!(target(root.join("照片/a.jpg")), Some(("照片".into(), false)));
    assert_eq!(target(root.join("a.txt")), Some(("".into(), false)));
}

/// A phone that was offline during a cleanup learns the cutoff when it reconnects.
#[tokio::test]
async fn new_event_streams_start_with_the_last_cleanup() {
    use futures_util::StreamExt as _;
    let e = env();
    let cookie = pair(&e).await;
    send(&e.lan, dev_req(Method::POST, "/api/messages", Some(&cookie), Body::from(json!({"text": "cleared"}).to_string()), true)).await;
    send(&e.local, host_req(Method::POST, "/api/host/clear-messages", Some(json!({})))).await;
    let res = e.lan.clone().oneshot(dev_req(Method::GET, "/api/events", Some(&cookie), Body::empty(), false)).await.unwrap();
    let mut body = res.into_body().into_data_stream();
    let mut seen = String::new();
    while !seen.contains("event: pruned") {
        match tokio::time::timeout(Duration::from_secs(2), body.next()).await {
            Ok(Some(Ok(chunk))) => seen.push_str(&String::from_utf8_lossy(&chunk)),
            _ => break,
        }
    }
    assert!(e.state.cleared_through() > 0);
    assert!(seen.contains("event: pruned") && seen.contains(&format!("\"through\":{}", e.state.cleared_through())), "{seen}");
}

#[tokio::test]
async fn deleting_a_message_removes_it_on_both_sides() {
    let e = env();
    let a = pair_named(&e, "A").await;
    let b = pair_named(&e, "B").await;
    let (a_id, b_id) = (device_id(&e, &a).await, device_id(&e, &b).await);
    let post = |cookie: &str, text: &str| dev_req(Method::POST, "/api/messages", Some(cookie), Body::from(json!({"text": text}).to_string()), true);
    let first = send(&e.lan, post(&a, "a password to copy")).await.json()["id"].as_i64().unwrap();
    let second = send(&e.lan, post(&a, "keep this")).await.json()["id"].as_i64().unwrap();
    let file = send(&e.lan, dev_req(Method::POST, "/api/messages/upload?name=a.jpg", Some(&a), Body::from(&b"jpeg"[..]), false)).await.json();
    let file_id = file["id"].as_i64().unwrap();
    let mut rx = e.state.events.subscribe();
    let del = |cookie: &str, id: i64, q: &str| dev_req(Method::DELETE, &format!("/api/messages/{id}{q}"), Some(cookie), Body::empty(), false);

    // Another phone cannot delete it; its owner can.
    assert_eq!(send(&e.lan, del(&b, first, "")).await.status, StatusCode::NOT_FOUND);
    assert_eq!(send(&e.lan, del(&a, first, "")).await.status, StatusCode::NO_CONTENT);
    assert_eq!(send(&e.lan, del(&a, first, "")).await.status, StatusCode::NOT_FOUND);
    // The computer can delete any message.
    assert_eq!(send(&e.local, host_req(Method::DELETE, &format!("/api/messages/{second}"), None)).await.status, StatusCode::NO_CONTENT);
    let page = send(&e.local, host_req(Method::GET, &format!("/api/messages?device={a_id}"), None)).await.json();
    let ids: Vec<i64> = page["messages"].as_array().unwrap().iter().filter_map(|m| m["id"].as_i64()).collect();
    assert_eq!(ids, [file_id]);

    // Deleting a file message keeps the attachment unless asked; a read-only phone may not trash it.
    send(&e.local, host_req(Method::PATCH, &format!("/api/host/devices/{a_id}"), Some(json!({"can_write": false})))).await;
    assert_eq!(send(&e.lan, del(&a, file_id, "?trash_file=1")).await.status, StatusCode::FORBIDDEN);
    assert_eq!(send(&e.lan, del(&a, file_id, "")).await.status, StatusCode::NO_CONTENT);
    assert!(e.state.sandbox().root().join("收件箱/a.jpg").exists());

    // Live events reach A and the host only; a stream that opens later still learns the ids.
    let (sa, sb) = (device_session(&a_id), device_session(&b_id));
    let mut deleted = 0;
    while let Ok(ev) = rx.try_recv() {
        if ev.name == "msg_deleted" {
            deleted += 1;
            assert!(ev.visible_to(&sa) && ev.visible_to(&Session::Host) && !ev.visible_to(&sb));
        }
    }
    assert_eq!(deleted, 3);
    assert_eq!(e.state.deleted_visible_to(&sa), [first, second, file_id]);
    assert!(e.state.deleted_visible_to(&sb).is_empty());
}

#[tokio::test]
async fn a_files_upload_retried_with_its_id_keeps_one_copy() {
    let e = env();
    let cookie = pair(&e).await;
    let up = |id: &str| dev_req(Method::POST, &format!("/api/files/upload?dir=&name=v.mov&upload_id={id}"), Some(&cookie), Body::from(&b"video"[..]), false);
    let first = send(&e.lan, up("u-1")).await.json();
    let again = send(&e.lan, up("u-1")).await.json();
    assert_eq!((first["path"].as_str(), again["path"].as_str()), (Some("v.mov"), Some("v.mov")));
    assert_eq!(send(&e.lan, up("u-2")).await.json()["path"], "v (1).mov", "a different file of the same name");
    assert!(!e.state.sandbox().root().join("v (2).mov").exists());
}

/// Refused as soon as the headers arrive: nothing is written to disk.
#[tokio::test]
async fn uploads_that_cannot_fit_are_refused_up_front() {
    use super::disk::{DiskInfo, fake_disk};
    const GB: u64 = 1 << 30;
    let e = env();
    let cookie = pair(&e).await;
    let root = e.state.sandbox().root().to_path_buf();
    let up = |name: &str, declared: Option<u64>, data: &'static [u8]| {
        let mut req = dev_req(Method::POST, &format!("/api/files/upload?dir=&name={name}"), Some(&cookie), Body::from(data), false);
        if let Some(n) = declared {
            req.headers_mut().insert(header::CONTENT_LENGTH, n.to_string().parse().unwrap());
        }
        req
    };
    let leftovers = || std::fs::read_dir(&root).unwrap().filter_map(|x| x.ok()).filter(|x| x.file_name() != "收件箱").count();
    let check = |q: &str| dev_req(Method::GET, &format!("/api/upload-check?{q}"), Some(&cookie), Body::empty(), false);

    // 100 GB disk: the reserve is 5 GB. With 6 GB free, a 2 GB file does not fit.
    fake_disk(&root, DiskInfo { available: 6 * GB, total: 100 * GB, fat: false });
    let r = send(&e.lan, up("big.mov", Some(2 * GB), b"x")).await;
    assert_eq!(r.status, StatusCode::INSUFFICIENT_STORAGE);
    assert!(r.json()["error"].as_str().unwrap().contains("电脑磁盘空间不足"));
    assert_eq!(leftovers(), 0);
    // The phone can ask first, for a Files folder or for a chat attachment.
    let gb2 = 2 * GB;
    assert_eq!(send(&e.lan, check(&format!("dir=&size={gb2}"))).await.status, StatusCode::INSUFFICIENT_STORAGE);
    assert_eq!(send(&e.lan, check(&format!("size={gb2}"))).await.status, StatusCode::INSUFFICIENT_STORAGE);
    assert_eq!(send(&e.lan, check("size=5")).await.status, StatusCode::NO_CONTENT);
    // A chat attachment that does not fit is refused the same way.
    let mut chat = dev_req(Method::POST, "/api/messages/upload?name=big.mov", Some(&cookie), Body::from(&b"x"[..]), false);
    chat.headers_mut().insert(header::CONTENT_LENGTH, gb2.to_string().parse().unwrap());
    assert_eq!(send(&e.lan, chat).await.status, StatusCode::INSUFFICIENT_STORAGE);
    assert_eq!(send(&e.lan, up("small.txt", Some(5), b"hello")).await.status, StatusCode::OK);

    // A FAT32 stick: one file must stay under 4 GB, however much space is left.
    fake_disk(&root, DiskInfo { available: 100 * GB, total: 200 * GB, fat: true });
    assert_eq!(send(&e.lan, up("huge.mkv", Some(5 * GB), b"x")).await.status, StatusCode::PAYLOAD_TOO_LARGE);

    // Fewer bytes than declared: refused, and no half file stays behind.
    let r = send(&e.lan, up("short.bin", Some(100), b"only ten b")).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert!(!root.join("short.bin").exists());
    assert_eq!(leftovers(), 1, "only small.txt");
}

/// A reservation counts only what is still to come: the disk's free space
/// already counts the bytes that landed.
#[test]
fn reservations_shrink_as_the_data_lands() {
    use super::disk::{DiskInfo, fake_disk};
    use super::transfer::preflight;
    const GB: u64 = 1 << 30;
    let e = env();
    let root = e.state.sandbox().root().to_path_buf();
    // 100 GB disk: the reserve is 5 GB, and 1500 bytes more than that are free.
    fake_disk(&root, DiskInfo { available: 5 * GB + 1500, total: 100 * GB, fat: false });
    let mut first = preflight(&e.state, &root, 1000).unwrap().unwrap();
    assert_eq!(e.state.reserved_uploads(), 1000);
    let refused = preflight(&e.state, &root, 1000).err().unwrap();
    assert_eq!(refused.status, StatusCode::INSUFFICIENT_STORAGE, "both together would cut into the reserve");
    // The numbers add up: what is free, and how much of it running uploads still need.
    assert!(refused.message.contains("（其中 1 KB 留给正在进行的上传）"), "{}", refused.message);
    first.landed(800);
    assert_eq!(e.state.reserved_uploads(), 200);
    let second = preflight(&e.state, &root, 1000).unwrap();
    drop((first, second));
    assert_eq!(e.state.reserved_uploads(), 0);
    assert!(!preflight(&e.state, &root, 5000).err().unwrap().message.contains("正在进行"), "nothing else is running");
}

/// A streamed body with no Content-Length and no size in the URL: its length
/// is unknown, so it could not be held to the free-space reserve.
fn unsized_body(data: &'static [u8]) -> Body {
    Body::from_stream(futures_util::stream::once(async move { Ok::<_, std::io::Error>(Bytes::from_static(data)) }))
}

#[tokio::test]
async fn uploads_of_unknown_length_are_refused() {
    let e = env();
    let cookie = pair(&e).await;
    let root = e.state.sandbox().root().to_path_buf();
    for uri in ["/api/files/upload?dir=&name=x.bin", "/api/messages/upload?name=x.bin"] {
        let r = send(&e.lan, dev_req(Method::POST, uri, Some(&cookie), unsized_body(b"data"), false)).await;
        assert_eq!(r.status, StatusCode::LENGTH_REQUIRED, "{uri}");
        // The size in the URL is enough.
        let sized = send(&e.lan, dev_req(Method::POST, &format!("{uri}&size=4"), Some(&cookie), unsized_body(b"data"), false)).await;
        assert_eq!(sized.status, StatusCode::OK, "{uri}");
    }
    assert!(root.join("x.bin").exists() && root.join("收件箱/x.bin").exists());
}

/// iOS 26.5 can send a picked file as an empty body (WebKit bug 319985): the
/// size in the URL catches it, so it is never saved as a complete file.
#[tokio::test]
async fn an_upload_that_lost_its_data_is_refused() {
    let e = env();
    let cookie = pair(&e).await;
    let root = e.state.sandbox().root().to_path_buf();
    let mut empty = dev_req(Method::POST, "/api/files/upload?dir=&name=v.mov&size=5", Some(&cookie), Body::empty(), false);
    empty.headers_mut().insert(header::CONTENT_LENGTH, "0".parse().unwrap());
    let r = send(&e.lan, empty).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert!(r.json()["error"].as_str().unwrap().contains("文件大小"));
    // Without a Content-Length the size is what must arrive.
    let short = dev_req(Method::POST, "/api/messages/upload?name=v.mov&size=5", Some(&cookie), unsized_body(b""), false);
    assert_eq!(send(&e.lan, short).await.status, StatusCode::BAD_REQUEST);
    assert!(!root.join("v.mov").exists() && !root.join("收件箱/v.mov").exists());
    let whole = dev_req(Method::POST, "/api/messages/upload?name=v.mov&size=5", Some(&cookie), Body::from(&b"video"[..]), false);
    assert_eq!(send(&e.lan, whole).await.json()["file"]["size"], 5);
}

/// A message sent again after it was deleted (the first attempt went
/// through, the phone never heard back) stays deleted.
#[tokio::test]
async fn a_deleted_message_stays_deleted_when_its_retry_arrives() {
    let e = env();
    let cookie = pair(&e).await;
    let id = device_id(&e, &cookie).await;
    let root = e.state.sandbox().root().to_path_buf();
    let text = |cid: &str| dev_req(Method::POST, "/api/messages", Some(&cookie), Body::from(json!({"client_id": cid, "text": "a password"}).to_string()), true);
    let upload = |cid: &str| dev_req(Method::POST, &format!("/api/messages/upload?client_id={cid}&name=a.jpg"), Some(&cookie), Body::from(&b"jpeg"[..]), false);
    let t = send(&e.lan, text("t-1")).await.json()["id"].as_i64().unwrap();
    let f = send(&e.lan, upload("f-1")).await.json()["id"].as_i64().unwrap();
    for m in [t, f] {
        assert_eq!(send(&e.local, host_req(Method::DELETE, &format!("/api/messages/{m}"), None)).await.status, StatusCode::NO_CONTENT);
    }
    let r = send(&e.lan, text("t-1")).await;
    assert_eq!(r.status, StatusCode::GONE);
    assert_eq!(r.json()["error"], "这条消息已在另一端删除");
    assert_eq!(send(&e.lan, upload("f-1")).await.status, StatusCode::GONE);
    assert!(!root.join("收件箱/a (1).jpg").exists(), "refused before the file was written");
    // The same after the computer clears every chat.
    send(&e.lan, text("t-2")).await;
    send(&e.local, host_req(Method::POST, "/api/host/clear-messages", Some(json!({})))).await;
    assert_eq!(send(&e.lan, text("t-2")).await.status, StatusCode::GONE);
    let page = send(&e.local, host_req(Method::GET, &format!("/api/messages?device={id}"), None)).await.json();
    assert!(page["messages"].as_array().unwrap().is_empty());
    assert_eq!(send(&e.lan, text("t-3")).await.status, StatusCode::OK, "new messages still go through");
}

/// Once an attachment is moved away, a newer file that takes its name
/// (here from another phone) is not that message's file; editing the
/// attachment itself keeps it.
#[tokio::test]
async fn a_newer_file_with_the_same_name_is_not_the_old_attachment() {
    use std::io::Write as _;
    let e = env();
    let a = pair_named(&e, "A").await;
    let b = pair_named(&e, "B").await;
    let a_id = device_id(&e, &a).await;
    let root = e.state.sandbox().root().to_path_buf();
    let newer = send(&e.lan, dev_req(Method::POST, "/api/messages/upload?name=IMG_0001.HEIC", Some(&b), Body::from(&b"from B"[..]), false)).await.json();
    assert_eq!(newer["file"]["path"], "收件箱/IMG_0001.HEIC");
    let saved = std::fs::metadata(root.join("收件箱/IMG_0001.HEIC")).unwrap();
    // A's earlier photo of that name, created an hour or a minute and a half
    // before; and one from v0.1.0, which recorded no creation time.
    let older = |created: Option<i64>, at: i64| NewMessage {
        device_id: &a_id,
        from_host: false,
        client_id: None,
        kind: "file",
        text: None,
        file_path: Some("收件箱/IMG_0001.HEIC"),
        file_name: Some("IMG_0001.HEIC"),
        file_size: Some(saved.len() as i64),
        mime: None,
        created_at: at,
        file_root: Some(root.to_str().unwrap()),
        file_created: created,
    };
    let now = super::state::created_ms(&saved).unwrap();
    let (earlier, _) = e.state.db.insert_message(&older(Some(now - 3_600_000), now - 3_000_000)).unwrap();
    let (bigger, _) = e.state.db.insert_message(&older(Some(now - 90_000), now - 80_000)).unwrap();
    let (legacy, _) = e.state.db.insert_message(&older(None, now - 60_000)).unwrap();

    let page = send(&e.lan, dev_req(Method::GET, "/api/messages", Some(&a), Body::empty(), false)).await.json();
    assert!(page["messages"].as_array().unwrap().iter().all(|m| m["file"]["exists"] == false), "{page}");
    for m in [earlier.id, bigger.id, legacy.id] {
        let r = send(&e.lan, dev_req(Method::GET, &format!("/api/messages/{m}/file"), Some(&a), Body::empty(), false)).await;
        assert_eq!(r.status, StatusCode::NOT_FOUND);
    }
    // Deleting A's message with "move the file to the Trash" leaves B's photo alone.
    let del = dev_req(Method::DELETE, &format!("/api/messages/{}?trash_file=1", earlier.id), Some(&a), Body::empty(), false);
    assert_eq!(send(&e.lan, del).await.status, StatusCode::NO_CONTENT);
    assert!(root.join("收件箱/IMG_0001.HEIC").exists());
    let b_page = send(&e.lan, dev_req(Method::GET, "/api/messages", Some(&b), Body::empty(), false)).await.json();
    assert_eq!(b_page["messages"][0]["file"]["exists"], true, "B's own message still has its file");
    // A v0.1.0 message saved after the file was created is still taken as its own.
    let (kept, _) = e.state.db.insert_message(&older(None, now + 1_000)).unwrap();
    assert!(e.state.message_out(e.state.db.message(kept.id).unwrap().unwrap()).file.unwrap().exists);

    // The computer's user edits B's photo in place an hour later (a new size and time): it is still B's.
    let mut edited = std::fs::OpenOptions::new().append(true).open(root.join("收件箱/IMG_0001.HEIC")).unwrap();
    edited.write_all(b", edited").unwrap();
    edited.set_modified(std::time::SystemTime::now() + Duration::from_secs(3600)).unwrap();
    drop(edited);
    let b_page = send(&e.lan, dev_req(Method::GET, "/api/messages", Some(&b), Body::empty(), false)).await.json();
    assert_eq!(b_page["messages"][0]["file"]["exists"], true, "{b_page}");
    let id = newer["id"].as_i64().unwrap();
    let r = send(&e.lan, dev_req(Method::GET, &format!("/api/messages/{id}/file"), Some(&b), Body::empty(), false)).await;
    assert_eq!(&r.body[..], b"from B, edited");
}

/// A phone that was away while the computer deleted or cleared messages
/// learns it when it reconnects, even if LanShare restarted meanwhile.
#[tokio::test]
async fn deletions_reach_a_phone_that_was_away_across_a_restart() {
    use futures_util::StreamExt as _;
    let e = env();
    let cookie = pair(&e).await;
    let device = device_id(&e, &cookie).await;
    let post = |text: &str| dev_req(Method::POST, "/api/messages", Some(&cookie), Body::from(json!({"text": text}).to_string()), true);
    let cleared = send(&e.lan, post("before the clear")).await.json()["id"].as_i64().unwrap();
    send(&e.local, host_req(Method::POST, "/api/host/clear-messages", Some(json!({})))).await;
    assert_eq!(e.state.cleared_through(), cleared, "a clear is passed on as the last id it removed");
    let gone = send(&e.lan, post("a password")).await.json()["id"].as_i64().unwrap();
    assert_eq!(send(&e.local, host_req(Method::DELETE, &format!("/api/messages/{gone}"), None)).await.status, StatusCode::NO_CONTENT);

    // An automatic cleanup's cutoff is kept as well.
    e.state.mark_pruned(1_000);
    let Env { _dir: dir, .. } = e;
    let e = env_in(dir);
    assert_eq!(e.state.cleared_through(), cleared);
    assert_eq!(e.state.pruned_before(), 1_000);
    assert_eq!(e.state.deleted_visible_to(&device_session(&device)), [gone]);
    assert!(e.state.deleted_visible_to(&device_session("another-phone")).is_empty());
    let res = e.lan.clone().oneshot(dev_req(Method::GET, "/api/events", Some(&cookie), Body::empty(), false)).await.unwrap();
    let mut body = res.into_body().into_data_stream();
    let mut seen = String::new();
    while !(seen.contains("event: msg_deleted") && seen.contains("event: pruned")) {
        match tokio::time::timeout(Duration::from_secs(2), body.next()).await {
            Ok(Some(Ok(chunk))) => seen.push_str(&String::from_utf8_lossy(&chunk)),
            _ => break,
        }
    }
    assert!(seen.contains(&format!("\"ids\":[{gone}]")) && seen.contains(&format!("\"through\":{cleared}")), "{seen}");

    // A cleanup cutoff from a clock that ran ahead is never later than now.
    e.state.mark_pruned(super::util::now_ms() + 3_600_000);
    assert!(e.state.pruned_before() <= super::util::now_ms());
}

#[tokio::test]
async fn removing_a_device_stops_its_upload() {
    use futures_util::StreamExt as _;
    let e = env();
    let cookie = pair(&e).await;
    let id = device_id(&e, &cookie).await;
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let first = futures_util::stream::once(async { Ok::<_, std::io::Error>(Bytes::from_static(b"first chunk")) });
    let second = futures_util::stream::once(async move {
        let _ = rx.await;
        Ok(Bytes::from_static(b"second chunk"))
    });
    let req = dev_req(Method::POST, "/api/files/upload?dir=&name=big.bin&size=23", Some(&cookie), Body::from_stream(first.chain(second)), false);
    let lan = e.lan.clone();
    let upload = tokio::spawn(async move { send(&lan, req).await.status });

    let root = e.state.sandbox().root().to_path_buf();
    let parts = || std::fs::read_dir(&root).unwrap().filter_map(|x| x.ok()).filter(|x| x.file_name().to_string_lossy().ends_with(".part")).count();
    for _ in 0..300 {
        if parts() > 0 && e.state.reserved_uploads() == 12 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(parts(), 1, "the upload should be in progress");
    assert_eq!(e.state.reserved_uploads(), 12, "the 11 bytes that landed are no longer reserved");
    assert!(e.state.awake.is_awake(), "the computer stays awake during the transfer");
    assert_eq!(send(&e.local, host_req(Method::DELETE, &format!("/api/host/devices/{id}"), None)).await.status, StatusCode::NO_CONTENT);
    let _ = tx.send(());
    assert_eq!(upload.await.unwrap(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(parts(), 0, "the partial file is removed");
    assert!(!root.join("big.bin").exists());
}

/// Each way a phone sends a file, while the shared folder is gone: refused
/// with a 503 (the phone offers 重试) that says why.
async fn uploads_refused(e: &Env, cookie: &str, error: &str) {
    let req = |method: Method, uri: &str, body: &'static [u8]| dev_req(method, uri, Some(cookie), Body::from(body), false);
    for r in [
        req(Method::GET, "/api/upload-check?size=4", b""),
        req(Method::GET, "/api/upload-check?dir=&size=4", b""),
        req(Method::POST, "/api/messages/upload?client_id=f-1&name=IMG_0001.HEIC&size=4", b"data"),
        req(Method::POST, "/api/files/upload?dir=&name=notes.txt&upload_id=u-1&size=4", b"data"),
        req(Method::GET, "/api/files?path=", b""),
    ] {
        let uri = r.uri().to_string();
        let res = send(&e.lan, r).await;
        assert_eq!((res.status, res.json()["error"].as_str()), (StatusCode::SERVICE_UNAVAILABLE, Some(error)), "{uri}");
    }
}

async fn root_unavailable(e: &Env) -> Value {
    send(&e.local, host_req(Method::GET, "/api/host/overview", None)).await.json()["shared_root_unavailable"].clone()
}

/// Backlog item 12: with the shared folder gone, uploads are refused before
/// anything is written and nothing is created in its place (with a drive
/// unplugged, that would be under /Volumes). Once it is back they work again.
#[tokio::test]
async fn uploads_wait_for_a_shared_folder_that_went_away() {
    let e = env();
    let cookie = pair(&e).await;
    let root = e.state.sandbox().root().to_path_buf();
    let mut rx = e.state.events.subscribe();
    std::fs::remove_dir_all(&root).unwrap();

    uploads_refused(&e, &cookie, "电脑上的共享文件夹不见了，可能被移动、改名或删除了").await;
    assert!(!root.exists(), "nothing is created where the shared folder was");
    assert_eq!(e.state.reserved_uploads(), 0);
    let msgs = send(&e.lan, dev_req(Method::GET, "/api/messages", Some(&cookie), Body::empty(), false)).await.json();
    assert_eq!(msgs["messages"], json!([]), "no message for a file that was not saved");
    assert_eq!(root_unavailable(&e).await, "folder");
    let told = std::iter::from_fn(|| rx.try_recv().ok()).filter(|ev| ev.name == "shared_root" && ev.visible_to(&Session::Host)).count();
    assert_eq!(told, 1, "Settings hears about it once, not once per request");

    // Back in place: the same uploads go through.
    std::fs::create_dir(&root).unwrap();
    let up = |uri: &str| dev_req(Method::POST, uri, Some(&cookie), Body::from(&b"data"[..]), false);
    assert_eq!(send(&e.lan, dev_req(Method::GET, "/api/upload-check?size=4", Some(&cookie), Body::empty(), false)).await.status, StatusCode::NO_CONTENT);
    let chat = send(&e.lan, up("/api/messages/upload?client_id=f-1&name=IMG_0001.HEIC&size=4")).await;
    assert_eq!((chat.status, chat.json()["file"]["exists"].clone()), (StatusCode::OK, json!(true)));
    assert_eq!(send(&e.lan, up("/api/files/upload?dir=&name=notes.txt&upload_id=u-1&size=4")).await.status, StatusCode::OK);
    assert_eq!(std::fs::read(root.join("收件箱").join("IMG_0001.HEIC")).unwrap(), b"data");
    assert_eq!(std::fs::read(root.join("notes.txt")).unwrap(), b"data");
    assert_eq!(root_unavailable(&e).await, Value::Null);
}

/// The same with the folder on a drive of its own: the phone and Settings
/// say that the disk is not connected.
#[cfg(unix)]
#[tokio::test]
async fn an_unplugged_drive_is_named_as_the_cause() {
    let e = env();
    let cookie = pair(&e).await;
    let root = e.state.sandbox().root().to_path_buf();
    e.state.set_sandbox(e.state.sandbox().on_its_own_disk());
    std::fs::remove_dir_all(&root).unwrap();

    uploads_refused(&e, &cookie, "电脑上的共享文件夹现在不可用，所在的磁盘可能没有连接").await;
    assert!(!root.exists());
    assert_eq!(root_unavailable(&e).await, "disk");
}

/// A drive unplugged during an upload: the phone hears why it failed.
/// (Unix: Windows refuses to delete a folder with a file open in it.)
#[cfg(unix)]
#[tokio::test]
async fn an_upload_cut_off_by_an_unplug_says_why() {
    use futures_util::StreamExt as _;
    let e = env();
    let cookie = pair(&e).await;
    let root = e.state.sandbox().root().to_path_buf();
    e.state.set_sandbox(e.state.sandbox().on_its_own_disk());
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let first = futures_util::stream::once(async { Ok::<_, std::io::Error>(Bytes::from_static(b"first chunk")) });
    let second = futures_util::stream::once(async move {
        let _ = rx.await;
        Ok(Bytes::from_static(b"second chunk"))
    });
    let req = dev_req(Method::POST, "/api/files/upload?dir=&name=big.bin&size=23", Some(&cookie), Body::from_stream(first.chain(second)), false);
    let lan = e.lan.clone();
    let upload = tokio::spawn(async move { send(&lan, req).await });

    let parts = || std::fs::read_dir(&root).map(|d| d.filter_map(|x| x.ok()).filter(|x| x.file_name().to_string_lossy().ends_with(".part")).count()).unwrap_or(0);
    for _ in 0..300 {
        if parts() > 0 && e.state.reserved_uploads() == 12 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(parts(), 1, "the upload should be in progress");
    std::fs::remove_dir_all(&root).unwrap();
    let _ = tx.send(());
    let res = upload.await.unwrap();
    assert_eq!((res.status, res.json()["error"].as_str()), (StatusCode::SERVICE_UNAVAILABLE, Some("电脑上的共享文件夹现在不可用，所在的磁盘可能没有连接")));
    assert!(!root.exists());
}

/// Started while the folder chosen in Settings was unavailable: the default
/// folder stands in, Settings says so, and the chosen folder is used again
/// once it is back.
#[tokio::test]
async fn the_chosen_folder_is_used_again_once_it_is_back() {
    let dir = tempfile::tempdir().unwrap();
    let chosen = dir.path().join("T7").join("Media");
    let e = env_with(dir, Some(chosen.clone()));
    let cookie = pair(&e).await;
    let overview = send(&e.local, host_req(Method::GET, "/api/host/overview", None)).await.json();
    assert_eq!(overview["preferred_root"], chosen.display().to_string());
    assert_eq!(overview["shared_root_unavailable"], Value::Null, "the stand-in itself works");
    assert!(!e.state.reopen_preferred_root(), "not back yet");

    std::fs::create_dir_all(&chosen).unwrap();
    let chosen = dunce::canonicalize(&chosen).unwrap();
    super::disk::fake_disk(&chosen, super::disk::DiskInfo { available: 1 << 40, total: 2 << 40, fat: false });
    assert!(e.state.reopen_preferred_root());
    assert_eq!(e.state.sandbox().root(), chosen);
    assert_eq!(send(&e.local, host_req(Method::GET, "/api/host/overview", None)).await.json()["preferred_root"], Value::Null);
    let r = send(&e.lan, dev_req(Method::POST, "/api/messages/upload?name=IMG_0001.HEIC&size=4", Some(&cookie), Body::from(&b"data"[..]), false)).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(chosen.join("收件箱").join("IMG_0001.HEIC").exists());
}

#[tokio::test]
async fn pause_and_cleanup_settings_survive_a_restart() {
    let e = env();
    let cookie = pair(&e).await;
    let device = device_id(&e, &cookie).await;

    // Cleanup deletes only messages older than the chosen period.
    let old = NewMessage {
        device_id: &device,
        from_host: false,
        client_id: None,
        kind: "text",
        text: Some("from last year"),
        file_path: None,
        file_name: None,
        file_size: None,
        mime: None,
        created_at: super::util::now_ms() - 40 * 86_400_000,
        file_root: None,
        file_created: None,
    };
    e.state.db.insert_message(&old).unwrap();
    send(&e.lan, dev_req(Method::POST, "/api/messages", Some(&cookie), Body::from(json!({"text": "today"}).to_string()), true)).await;
    assert_eq!(send(&e.local, host_req(Method::POST, "/api/host/chat-retention", Some(json!({"days": 7})))).await.status, StatusCode::BAD_REQUEST);
    let r = send(&e.local, host_req(Method::POST, "/api/host/chat-retention", Some(json!({"days": 30})))).await.json();
    assert_eq!(r["deleted"], 1);
    let page = send(&e.local, host_req(Method::GET, &format!("/api/messages?device={device}"), None)).await.json();
    let texts: Vec<&str> = page["messages"].as_array().unwrap().iter().filter_map(|m| m["text"].as_str()).collect();
    assert_eq!(texts, ["today"]);

    send(&e.local, host_req(Method::POST, "/api/host/paused", Some(json!({"paused": true})))).await;
    let Env { _dir: dir, .. } = e;
    let e = env_in(dir);
    assert!(e.state.is_paused(), "a pause survives a restart");
    let overview = send(&e.local, host_req(Method::GET, "/api/host/overview", None)).await.json();
    assert_eq!((overview["paused"].as_bool(), overview["chat_retention_days"].as_u64()), (Some(true), Some(30)));
    assert_eq!(send(&e.lan, dev_req(Method::GET, "/api/me", Some(&cookie), Body::empty(), false)).await.status, StatusCode::SERVICE_UNAVAILABLE);
}

#[derive(Clone, Default)]
struct Capture(Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Every log line of this test process, at the most verbose level.
fn captured_logs() -> Capture {
    static CAPTURE: OnceLock<Capture> = OnceLock::new();
    CAPTURE
        .get_or_init(|| {
            let capture = Capture::default();
            let writer = capture.clone();
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::TRACE)
                .with_ansi(false)
                .with_writer(move || writer.clone())
                .try_init()
                .expect("no other test installs a global subscriber");
            capture
        })
        .clone()
}

/// The privacy rule in core::logging: no message text and no file names.
#[tokio::test]
async fn logs_never_contain_message_text_or_file_names() {
    let logs = captured_logs();
    let e = env();
    let cookie = pair_named(&e, "Phone").await;
    let device = device_id(&e, &cookie).await;
    let dev = |method: Method, uri: &str, body: Option<Value>| match body {
        Some(v) => dev_req(method, uri, Some(&cookie), Body::from(v.to_string()), true),
        None => dev_req(method, uri, Some(&cookie), Body::from(&b"some bytes"[..]), false),
    };
    for req in [
        dev(Method::POST, "/api/messages", Some(json!({"text": "LOGMARK text from the phone"}))),
        dev(Method::POST, "/api/messages/upload?name=LOGMARK-chat.jpg", None),
        dev(Method::POST, "/api/files/upload?dir=&name=LOGMARK-file.txt", None),
        dev(Method::POST, "/api/files/rename", Some(json!({"path": "LOGMARK-file.txt", "name": "LOGMARK-renamed.txt"}))),
        dev(Method::POST, "/api/files/mkdir", Some(json!({"dir": "", "name": "LOGMARK-folder"}))),
        dev(Method::GET, "/api/files/download?path=LOGMARK-renamed.txt", None),
        dev(Method::GET, "/api/files/download?path=LOGMARK-missing.txt", None),
        dev(Method::POST, "/api/files/rename", Some(json!({"path": "LOGMARK-renamed.txt", "name": "LOGMARK-folder"}))),
    ] {
        send(&e.lan, req).await;
    }
    for (method, uri, body) in [
        (Method::POST, "/api/messages", Some(json!({"device": device, "text": "LOGMARK reply from the computer"}))),
        (Method::POST, "/api/host/open", Some(json!({"path": "LOGMARK-missing.txt"}))),
        (Method::POST, "/api/host/open-url", Some(json!({"url": "LOGMARK is not a link"}))),
        (Method::GET, "/api/files?path=LOGMARK-folder", None),
    ] {
        send(&e.local, host_req(method, uri, body)).await;
    }
    // A shared folder that went away is logged without its path.
    let named = e.state.sandbox().root().join("LOGMARK-shared");
    e.state.set_sandbox(Sandbox::new(&named).unwrap().with_deny(std::slice::from_ref(&e.state.data_dir)));
    std::fs::remove_dir(&named).unwrap();
    send(&e.lan, dev(Method::POST, "/api/messages/upload?name=LOGMARK-gone.jpg", None)).await;
    let text = String::from_utf8_lossy(&logs.0.lock().unwrap()).into_owned();
    assert!(text.contains("[notification]"), "the capture should see the server's log lines");
    assert!(text.contains("the shared folder is missing"));
    assert!(!text.contains("LOGMARK"), "a log line leaked message text or a file name:\n{text}");
}

#[tokio::test]
async fn static_shell_has_security_headers() {
    let e = env();
    let r = send(&e.lan, dev_req(Method::GET, "/pair", None, Body::empty(), false)).await;
    // dist/ may be missing in CI before the frontend is built.
    if r.status == StatusCode::OK {
        assert!(r.headers[header::CONTENT_SECURITY_POLICY].to_str().unwrap().contains("default-src 'self'"));
    }
    assert_eq!(r.headers[header::X_FRAME_OPTIONS], "DENY");
    assert_eq!(send(&e.lan, dev_req(Method::GET, "/api/nope", None, Body::empty(), false)).await.status, StatusCode::NOT_FOUND);
}

#[test]
fn shared_folder_fallbacks() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let data = home.join("Library/Application Support/app");
    std::fs::create_dir_all(&data).unwrap();

    // Missing folder whose parent exists: created in place.
    let wanted = home.join("Shares/LanShare");
    std::fs::create_dir_all(home.join("Shares")).unwrap();
    let mut cfg = Config { port: 1, ..Config::with_root(wanted.clone()) };
    let (sb, preferred) = super::open_shared_folder(&mut cfg, &home, &data).unwrap();
    assert_eq!(sb.root(), dunce::canonicalize(&wanted).unwrap());
    assert_eq!(preferred, None);

    // Unplugged drive (parent missing): fall back to ~/LanShare, keep the setting.
    let unplugged = dir.path().join("Volumes/T7/Media");
    let mut cfg = Config { port: 1, ..Config::with_root(unplugged.clone()) };
    let (sb, preferred) = super::open_shared_folder(&mut cfg, &home, &data).unwrap();
    assert!(sb.root().ends_with("LanShare"));
    assert_eq!(cfg.shared_root, unplugged);
    assert_eq!(preferred, Some(unplugged), "used again once it is back");

    // Unsafe choice (home itself): fall back and overwrite the setting.
    let mut cfg = Config { port: 1, ..Config::with_root(home.clone()) };
    let (sb, preferred) = super::open_shared_folder(&mut cfg, &home, &data).unwrap();
    assert_eq!(cfg.shared_root, sb.root());
    assert_eq!(preferred, None);
}

/// First bytes of a certificate's Subject Key Identifier.
fn subject_key_id(cert_pem: &str) -> Vec<u8> {
    use x509_parser::extensions::ParsedExtension;
    let (_, pem) = x509_parser::pem::parse_x509_pem(cert_pem.as_bytes()).unwrap();
    let x509 = pem.parse_x509().unwrap();
    x509.extensions()
        .iter()
        .find_map(|ext| match ext.parsed_extension() {
            ParsedExtension::SubjectKeyIdentifier(id) => Some(id.0.to_vec()),
            _ => None,
        })
        .unwrap()
}

#[test]
fn ca_names_tell_installs_apart() {
    assert_eq!(tls::ca_common_name("Mac", "2DD31B"), "LanShare Local CA (Mac) 2DD31B");
    assert_eq!(tls::ca_common_name(&"x".repeat(60), "ABCDEF"), format!("LanShare Local CA ({}) ABCDEF", "x".repeat(40)));

    // Two installs on one computer (a dev run and the release app) get different names.
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let ta = Tls::init(a.path(), "Mac", None, &[]).unwrap();
    let tb = Tls::init(b.path(), "Mac", None, &[]).unwrap();
    assert_ne!(ta.ca_name(), tb.ca_name());
    for t in [&ta, &tb] {
        let name = t.ca_name();
        let tag = name.strip_prefix("LanShare Local CA (Mac) ").unwrap();
        assert!(tag.len() == 6 && tag.chars().all(|c| c.is_ascii_digit() || ('A'..='F').contains(&c)), "{name}");
        assert_eq!(tag, hex::encode_upper(&subject_key_id(&t.ca_pem())[..3]));
        assert_eq!(Tls::common_name(&t.ca_pem()).unwrap(), name);
    }

    // So does every reset.
    let before = ta.ca_name();
    ta.reset("Mac", &[]).unwrap();
    assert_ne!(ta.ca_name(), before);
    assert!(ta.ca_name().starts_with("LanShare Local CA (Mac) "));
    assert_eq!(Tls::common_name(&std::fs::read_to_string(a.path().join("ca.pem")).unwrap()).unwrap(), ta.ca_name());
}

#[test]
fn existing_ca_keeps_its_name() {
    // A CA from an older version (no tag) is reused as it is, not regenerated.
    let dir = tempfile::tempdir().unwrap();
    Tls::write_ca(dir.path(), rcgen::KeyPair::generate().unwrap(), "LanShare Local CA (Mac)").unwrap();
    let pem = std::fs::read_to_string(dir.path().join("ca.pem")).unwrap();
    let t = Tls::init(dir.path(), "Mac", Some("mymac.local".into()), &[]).unwrap();
    assert_eq!(t.ca_name(), "LanShare Local CA (Mac)");
    assert_eq!(t.ca_pem(), pem);
    assert_eq!(std::fs::read_to_string(dir.path().join("ca.pem")).unwrap(), pem);
}

#[test]
fn local_dns_name_prefers_bonjour() {
    assert_eq!(net::pick_local_dns_name(Some("MyMac"), "Mac").unwrap(), "mymac.local");
    assert_eq!(net::pick_local_dns_name(None, "Mac").unwrap(), "mac.local");
    assert_eq!(net::pick_local_dns_name(Some(""), "Mac.local").unwrap(), "mac.local");
    assert!(net::pick_local_dns_name(None, "我的电脑").is_none());
}

#[cfg(target_os = "macos")]
#[test]
fn bonjour_name_matches_scutil() {
    let Ok(out) = std::process::Command::new("scutil").args(["--get", "LocalHostName"]).output() else { return };
    if !out.status.success() {
        return; // no LocalHostName set
    }
    let expected = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert_eq!(net::bonjour_host_name().as_deref(), Some(expected.as_str()));
    assert_eq!(net::local_dns_name(), tls::local_dns_name(&expected));
}

#[test]
fn leaf_follows_local_name() {
    let dir = tempfile::tempdir().unwrap();
    let ip: IpAddr = "192.168.1.5".parse().unwrap();
    let leaf = || std::fs::read_to_string(dir.path().join("server.pem")).unwrap();
    let t = Tls::init(dir.path(), "Mac", Some("mac.local".into()), &[ip]).unwrap();
    let (ca, first) = (t.ca_pem(), leaf());
    assert!(Tls::leaf_usable(&first, &ca, Some("mac.local"), &[ip]).unwrap());

    // Renamed while LanShare was not running: the next start re-issues the leaf and keeps the CA.
    let t = Tls::init(dir.path(), "Mac", Some("mymac.local".into()), &[ip]).unwrap();
    assert_ne!(leaf(), first);
    assert_eq!(t.ca_pem(), ca);
    assert!(Tls::leaf_usable(&leaf(), &ca, Some("mymac.local"), &[ip]).unwrap());
    assert!(!Tls::leaf_usable(&leaf(), &ca, Some("mac.local"), &[ip]).unwrap());

    // Same name: nothing to do. Unusable name: the current one stays.
    assert!(!t.ensure_names(Some("mymac.local".into()), &[ip]).unwrap());
    assert!(!t.ensure_names(None, &[ip]).unwrap());

    // Renamed while running: re-issued, like an address change.
    let before = leaf();
    assert!(t.ensure_names(Some("studio.local".into()), &[ip]).unwrap());
    assert_eq!(t.leaf_dns().as_deref(), Some("studio.local"));
    assert_ne!(leaf(), before);
    assert!(Tls::leaf_usable(&leaf(), &ca, Some("studio.local"), &[ip]).unwrap());

    // A later address change keeps the new name.
    let ip2: IpAddr = "10.0.0.9".parse().unwrap();
    assert!(t.ensure_names(None, &[ip, ip2]).unwrap());
    assert!(Tls::leaf_usable(&leaf(), &ca, Some("studio.local"), &[ip, ip2]).unwrap());
}

#[tokio::test]
async fn ca_name_is_public_and_in_overview() {
    let e = env();
    let name = e.state.tls.ca_name();
    assert!(name.starts_with("LanShare Local CA (test) "), "{name}");
    let r = send(&e.lan, dev_req(Method::GET, "/api/ca", None, Body::empty(), false)).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.json()["name"], name.as_str());
    let o = send(&e.local, host_req(Method::GET, "/api/host/overview", None)).await;
    assert_eq!(o.json()["ca_name"], name.as_str());
}

#[test]
fn host_name_prefers_computer_name() {
    assert_eq!(super::pick_host_name(Some("MyMac"), "Mac"), "MyMac");
    assert_eq!(super::pick_host_name(Some("小明的 MacBook Pro"), "Mac"), "小明的 MacBook Pro");
    assert_eq!(super::pick_host_name(Some("  "), "Mac.local"), "Mac");
    assert_eq!(super::pick_host_name(None, "DESKTOP-AB12"), "DESKTOP-AB12");
    assert_eq!(super::pick_host_name(None, " .local "), "我的电脑");
}

#[cfg(target_os = "macos")]
#[test]
fn computer_name_matches_scutil() {
    let Ok(out) = std::process::Command::new("scutil").args(["--get", "ComputerName"]).output() else { return };
    if !out.status.success() {
        return; // no Computer Name set
    }
    let expected = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert_eq!(net::computer_name().as_deref(), Some(expected.as_str()));
    assert_eq!(super::host_name(), expected);
}

mod cert_reset;
mod chat_folders;
mod file_races;
mod m8;
mod stops;
