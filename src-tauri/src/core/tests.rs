//! In-process API tests: real routers, auth and handlers, no sockets.
//! They run on every platform (including Windows CI).

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use axum::Extension;
use serde_json::{Value, json};
use tower::ServiceExt as _;

use super::bridge::HeadlessBridge;
use super::config::Config;
use super::db::Db;
use super::paths::Sandbox;
use super::state::{AppState, Shared, StateParts};
use super::tls::Tls;
use super::{model::Address, routes};

const LOCAL_PORT: u16 = 53299;
const TOKEN: &str = "test-local-token";

struct Env {
    _dir: tempfile::TempDir,
    state: Shared,
    lan: Router,
    local: Router,
}

fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let sandbox = Sandbox::new(&dir.path().join("share")).unwrap();
    let tls = Tls::init(&dir.path().join("tls"), "test", None, &[]).unwrap();
    let state = AppState::new(StateParts {
        db: Db::open_in_memory().unwrap(),
        data_dir: dir.path().join("data"),
        config: Config { shared_root: sandbox.root().to_path_buf(), port: 53210 },
        sandbox,
        local_token: TOKEN.into(),
        local_port: LOCAL_PORT,
        lan_port: 53210,
        tls,
        bridge: Arc::new(HeadlessBridge),
        host_name: "test-host".into(),
        addresses: vec![Address { ip: "192.168.1.5".into(), iface: "en0".into() }],
    });
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
    let session = send(&e.local, host_req(Method::POST, "/api/host/pairing", Some(json!({})))).await.json();
    let code = session["code"].as_str().unwrap().to_string();
    let r = send(&e.lan, dev_req(Method::POST, "/api/pair/request", None, Body::from(json!({"code": code, "name": "iPhone"}).to_string()), true)).await;
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

    for bad in ["..%2Fx", "%2E%2E", ".hidden", "a%5Cb", "C:%2Fx"] {
        let r = send(&e.lan, dev_req(Method::GET, &format!("/api/files/download?path={bad}"), Some(&cookie), Body::empty(), false)).await;
        assert!(matches!(r.status.as_u16(), 400 | 403 | 404), "{bad} -> {}", r.status);
    }

    // Script-capable files never render in our origin.
    send(&e.lan, up("", "x.html", b"<script>alert(1)</script>")).await;
    let r = send(&e.lan, dev_req(Method::GET, "/api/files/download?path=x.html&inline=1", Some(&cookie), Body::empty(), false)).await;
    assert_eq!(r.headers[header::CONTENT_TYPE], "text/plain; charset=utf-8");
    assert_eq!(r.headers[header::CONTENT_SECURITY_POLICY], "sandbox");

    // Read-only devices cannot write.
    let device = send(&e.local, host_req(Method::GET, "/api/host/devices", None)).await.json()[0]["id"].as_str().unwrap().to_string();
    send(&e.local, host_req(Method::PATCH, &format!("/api/host/devices/{device}"), Some(json!({"can_write": false})))).await;
    assert_eq!(send(&e.lan, up("", "b.txt", b"x")).await.status, StatusCode::FORBIDDEN);
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
