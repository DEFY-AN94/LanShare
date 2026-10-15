//! In-process API tests: real routers, auth and handlers, no sockets.
//! They run on every platform (including Windows CI).

use std::net::{IpAddr, SocketAddr};
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
    let dir = tempfile::tempdir().unwrap();
    let sandbox = Sandbox::new(&dir.path().join("share")).unwrap();
    let tls = Tls::init(&dir.path().join("tls"), "test", None, &[]).unwrap();
    let state = AppState::new(StateParts {
        db: Db::open_in_memory().unwrap(),
        data_dir: dir.path().join("data"),
        home_dir: dir.path().to_path_buf(),
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
    let mut cfg = Config { shared_root: wanted.clone(), port: 1 };
    let sb = super::open_shared_folder(&mut cfg, &home, &data).unwrap();
    assert_eq!(sb.root(), std::fs::canonicalize(&wanted).unwrap());

    // Unplugged drive (parent missing): fall back to ~/LanShare, keep the setting.
    let unplugged = dir.path().join("Volumes/T7/Media");
    let mut cfg = Config { shared_root: unplugged.clone(), port: 1 };
    let sb = super::open_shared_folder(&mut cfg, &home, &data).unwrap();
    assert!(sb.root().ends_with("LanShare"));
    assert_eq!(cfg.shared_root, unplugged);

    // Unsafe choice (home itself): fall back and overwrite the setting.
    let mut cfg = Config { shared_root: home.clone(), port: 1 };
    let sb = super::open_shared_folder(&mut cfg, &home, &data).unwrap();
    assert_eq!(cfg.shared_root, sb.root());
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
