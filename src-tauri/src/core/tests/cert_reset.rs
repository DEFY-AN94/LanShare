//! A new root certificate means every phone pairs again: devices remember
//! the root they paired under, and one paired under another is marked
//! "needs re-pairing" with its history kept. New roots are limited to server
//! authentication; older ones are kept as they are.

use futures_util::StreamExt as _;

use super::*;
use crate::core::quick;

/// What a root certificate's EKU says, and whether it is critical.
fn eku(cert_pem: &str) -> Option<(bool, bool, bool)> {
    use x509_parser::extensions::ParsedExtension;
    let (_, pem) = x509_parser::pem::parse_x509_pem(cert_pem.as_bytes()).unwrap();
    let x509 = pem.parse_x509().unwrap();
    x509.extensions().iter().find_map(|ext| match ext.parsed_extension() {
        ParsedExtension::ExtendedKeyUsage(u) => {
            let only_server = u.server_auth && !(u.any || u.client_auth || u.code_signing || u.email_protection || u.time_stamping || u.ocsp_signing) && u.other.is_empty();
            Some((ext.critical, u.server_auth, only_server))
        }
        _ => None,
    })
}

async fn devices(e: &Env) -> Vec<Value> {
    send(&e.local, host_req(Method::GET, "/api/host/devices", None)).await.json().as_array().unwrap().clone()
}

async fn needs_repair(e: &Env, id: &str) -> bool {
    devices(e).await.iter().find(|d| d["id"] == id).unwrap()["needs_repair"].as_bool().unwrap()
}

async fn me(e: &Env, cookie: &str) -> StatusCode {
    send(&e.lan, dev_req(Method::GET, "/api/me", Some(cookie), Body::empty(), false)).await.status
}

#[tokio::test]
async fn after_a_reset_every_phone_pairs_again() {
    let e = env();
    let cookie = pair(&e).await;
    let id = device_id(&e, &cookie).await;
    let r = send(&e.lan, dev_req(Method::POST, "/api/messages", Some(&cookie), Body::from(json!({ "text": "before the reset" }).to_string()), true)).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(!needs_repair(&e, &id).await);
    // Open at the reset: an event stream, and a pairing that has not been answered yet.
    let mut stream = e.lan.clone().oneshot(dev_req(Method::GET, "/api/events", Some(&cookie), Body::empty(), false)).await.unwrap().into_body().into_data_stream();
    assert!(String::from_utf8_lossy(&stream.next().await.unwrap().unwrap()).contains("connected"));
    let code = send(&e.local, host_req(Method::POST, "/api/host/pairing", Some(json!({})))).await.json()["code"].as_str().unwrap().to_string();
    let request = send(&e.lan, dev_req(Method::POST, "/api/pair/request", None, Body::from(json!({ "code": code, "name": "Test iPad B" }).to_string()), true)).await.json();
    let before = e.state.tls.ca_fingerprint();

    assert_eq!(send(&e.local, host_req(Method::POST, "/api/host/reset-cert", Some(json!({})))).await.status, StatusCode::NO_CONTENT);
    assert_ne!(e.state.tls.ca_fingerprint(), before);

    // The phone hears it was removed, and its cookie no longer works.
    let mut seen = String::new();
    while let Ok(Some(Ok(chunk))) = tokio::time::timeout(Duration::from_secs(1), stream.next()).await {
        seen.push_str(&String::from_utf8_lossy(&chunk));
    }
    assert!(seen.contains("event: revoked"), "{seen}");
    assert_eq!(me(&e, &cookie).await, StatusCode::UNAUTHORIZED);
    // The pairing under way is gone.
    let approve = format!("/api/host/pairing/requests/{}/approve", request["request_id"].as_str().unwrap());
    assert_eq!(send(&e.local, host_req(Method::POST, &approve, Some(json!({})))).await.status, StatusCode::NOT_FOUND);

    // The computer still has the device and its history, marked, and cannot send to it.
    assert!(needs_repair(&e, &id).await);
    let history = send(&e.local, host_req(Method::GET, &format!("/api/messages?device={id}"), None)).await;
    assert_eq!(history.status, StatusCode::OK);
    assert!(history.body.windows(16).any(|w| w == b"before the reset"));
    let to_it = send(&e.local, host_req(Method::POST, "/api/messages", Some(json!({ "device": id, "text": "hello?" })))).await;
    assert_eq!((to_it.status, to_it.json()["error"].as_str()), (StatusCode::CONFLICT, Some(crate::core::chat::NEEDS_REPAIR)));
    let mut file = host_req(Method::POST, &format!("/api/messages/upload?device={id}&name=a.txt&size=4"), None);
    *file.body_mut() = Body::from("data");
    assert_eq!(send(&e.local, file).await.status, StatusCode::CONFLICT);
    assert!(!e.state.sandbox().root().join("已发送").join("a.txt").exists());
    // Nor is it offered to the clipboard or the tray.
    let target = send(&e.local, host_req(Method::POST, "/api/host/clipboard/target", Some(json!({ "device": id })))).await;
    assert_eq!(target.status, StatusCode::CONFLICT);
    assert!(quick::recent_devices(&e.state).is_empty());
    assert!(quick::clipboard_target(&e.state).is_none());
    assert_eq!(quick::send_clipboard(&e.state, Some(&id)).await.unwrap_err(), crate::core::chat::NEEDS_REPAIR);

    // Pairing again works, as a new device.
    let again = pair(&e).await;
    let new_id = device_id(&e, &again).await;
    assert_ne!(new_id, id);
    assert!(!needs_repair(&e, &new_id).await);
    assert_eq!(quick::recent_devices(&e.state).iter().map(|(d, _)| d.as_str()).collect::<Vec<_>>(), [new_id.as_str()]);

    // And so it stays after a restart.
    let Env { _dir: dir, .. } = e;
    let e = env_in(dir);
    assert!(needs_repair(&e, &id).await);
    assert!(!needs_repair(&e, &new_id).await);
    assert_eq!(me(&e, &cookie).await, StatusCode::UNAUTHORIZED);
    assert_eq!(me(&e, &again).await, StatusCode::OK);
}

/// The computer also makes a new root by itself when the old one cannot be
/// used (its key is gone or it is about to expire).
#[tokio::test]
async fn a_root_replaced_at_startup_means_pairing_again_too() {
    let e = env();
    let cookie = pair(&e).await;
    let id = device_id(&e, &cookie).await;
    let Env { _dir: dir, .. } = e;
    std::fs::remove_file(dir.path().join("tls").join("ca.key.pem")).unwrap();
    let e = env_in(dir);
    assert_eq!(me(&e, &cookie).await, StatusCode::UNAUTHORIZED);
    assert!(needs_repair(&e, &id).await);
}

/// Devices paired before devices remembered their root count as paired
/// under the current one.
#[tokio::test]
async fn devices_from_before_this_version_keep_working() {
    let e = env();
    let cookie = pair(&e).await;
    let id = device_id(&e, &cookie).await;
    let Env { _dir: dir, .. } = e;
    rusqlite::Connection::open(dir.path().join("data").join("lanshare.db")).unwrap().execute("UPDATE devices SET ca_fingerprint = NULL", []).unwrap();
    let e = env_in(dir);
    assert_eq!(me(&e, &cookie).await, StatusCode::OK);
    assert!(!needs_repair(&e, &id).await);
}

#[tokio::test]
async fn new_roots_are_limited_to_server_authentication() {
    let e = env();
    assert_eq!(eku(&e.state.tls.ca_pem()), Some((false, true, true)), "serverAuth only, not critical");
    send(&e.local, host_req(Method::POST, "/api/host/reset-cert", Some(json!({})))).await;
    assert_eq!(eku(&e.state.tls.ca_pem()), Some((false, true, true)));
}

/// A root from v0.3.0 or earlier (no EKU) is kept as it is, and still signs
/// a usable server certificate.
#[test]
fn a_root_from_before_is_kept_as_it_is() {
    use rcgen::{BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, KeyPair, KeyUsagePurpose};
    let dir = tempfile::tempdir().unwrap();
    let key = KeyPair::generate().unwrap();
    let mut p = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "LanShare Local CA (Mac) 2DD31B");
    p.distinguished_name = dn;
    p.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    p.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign, KeyUsagePurpose::DigitalSignature];
    let pem = p.self_signed(&key).unwrap().pem();
    std::fs::write(dir.path().join("ca.pem"), &pem).unwrap();
    std::fs::write(dir.path().join("ca.key.pem"), key.serialize_pem()).unwrap();
    let ip: IpAddr = "192.168.1.5".parse().unwrap();
    let t = Tls::init(dir.path(), "Mac", Some("mymac.local".into()), &[ip]).unwrap();
    assert_eq!(t.ca_pem(), pem);
    assert_eq!(eku(&t.ca_pem()), None);
    let leaf = std::fs::read_to_string(dir.path().join("server.pem")).unwrap();
    assert!(Tls::leaf_usable(&leaf, &pem, Some("mymac.local"), &[ip]).unwrap());
}

/// Step 1 of 添加设备: an address with no secret, which opens no pairing.
#[tokio::test]
async fn the_certificate_step_carries_no_secret() {
    let e = env();
    let setup = send(&e.local, host_req(Method::GET, "/api/host/cert-setup", None)).await;
    assert_eq!(setup.status, StatusCode::OK);
    let setup = setup.json();
    let urls = setup["urls"].as_array().unwrap();
    assert_eq!(urls[0]["url"], "https://192.168.1.5:53210/pair?cert");
    assert!(urls.iter().all(|u| !u["url"].as_str().unwrap().contains("t=") && u["qr_svg"].as_str().unwrap().contains("<svg")));
    assert_eq!(setup["ca_fingerprint"], e.state.tls.ca_fingerprint().as_str());
    assert_eq!(setup["ca_name"], e.state.tls.ca_name().as_str());
    assert_eq!(setup["paused"], false);
    // No pairing session was opened.
    let r = send(&e.lan, dev_req(Method::POST, "/api/pair/request", None, Body::from(json!({ "code": "000000", "name": "x" }).to_string()), true)).await;
    assert_eq!((r.status, r.json()["error"].as_str()), (StatusCode::FORBIDDEN, Some(crate::core::pairing::PairError::NoSession.message())));
    // Only the computer asks.
    let cookie = pair(&e).await;
    assert_eq!(send(&e.lan, dev_req(Method::GET, "/api/host/cert-setup", Some(&cookie), Body::empty(), false)).await.status, StatusCode::FORBIDDEN);
    // The certificate itself stays downloadable on the phone.
    let ca = send(&e.lan, dev_req(Method::GET, "/api/ca.crt", None, Body::empty(), false)).await;
    assert_eq!((ca.status, String::from_utf8_lossy(&ca.body).into_owned()), (StatusCode::OK, e.state.tls.ca_pem()));
}
