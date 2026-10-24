//! Transfers and event streams stop when they should: at once when the
//! computer removes a phone or pauses sharing, also while no data moves, and
//! after `TRANSFER_IDLE` without data. Pairing requests are read within bounds.

use std::task::Poll;

use futures_util::StreamExt as _;

use super::*;
use crate::core::transfer::TRANSFER_IDLE;

/// A request body that never sends anything; `polled` fires once it is asked for data.
fn silent_body() -> (Body, tokio::sync::oneshot::Receiver<()>) {
    let (tx, polled) = tokio::sync::oneshot::channel();
    let mut tx = Some(tx);
    let body = Body::from_stream(futures_util::stream::poll_fn(move |_| {
        if let Some(tx) = tx.take() {
            let _ = tx.send(());
        }
        Poll::<Option<Result<Bytes, std::io::Error>>>::Pending
    }));
    (body, polled)
}

/// A request body that sends `data`, then runs `at_end` as it ends.
fn body_then(data: &'static [u8], at_end: impl FnOnce() + Send + 'static) -> Body {
    let mut at_end = Some(at_end);
    let end = futures_util::stream::poll_fn(move |_| {
        if let Some(f) = at_end.take() {
            f();
        }
        Poll::<Option<Result<Bytes, std::io::Error>>>::Ready(None)
    });
    Body::from_stream(futures_util::stream::once(async move { Ok(Bytes::from_static(data)) }).chain(end))
}

fn files_in(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_dir(dir).map(|d| d.filter_map(|x| x.ok()).map(|x| x.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default()
}

async fn until_asleep(e: &Env) {
    for _ in 0..100 {
        if !e.state.awake.is_awake() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the computer is still kept awake");
}

const UPLOADS: [(&str, &str); 2] = [("/api/files/upload?dir=&name=stalled.bin&size=1000", ""), ("/api/messages/upload?name=stalled.bin&size=1000", "收件箱")];

#[tokio::test(start_paused = true)]
async fn an_upload_that_sends_nothing_is_cancelled_after_two_minutes() {
    for (uri, folder) in UPLOADS {
        let e = env();
        let cookie = pair(&e).await;
        let (body, polled) = silent_body();
        let lan = e.lan.clone();
        let started = tokio::time::Instant::now();
        let upload = tokio::spawn(async move { send(&lan, dev_req(Method::POST, uri, Some(&cookie), body, false)).await });
        polled.await.unwrap();
        let dir = e.state.sandbox().root().join(folder);
        assert_eq!(files_in(&dir).iter().filter(|n| n.ends_with(".part")).count(), 1, "{uri}");
        assert_eq!(e.state.reserved_uploads(), 1000);
        assert!(e.state.awake.is_awake());

        let r = upload.await.unwrap();
        assert_eq!(r.status, StatusCode::REQUEST_TIMEOUT, "{uri}");
        assert!(r.json()["error"].as_str().unwrap().contains("长时间没有收到数据"));
        assert!(started.elapsed() >= TRANSFER_IDLE);
        assert!(files_in(&dir).iter().all(|n| !n.ends_with(".part") && n != "stalled.bin"), "{uri}: {:?}", files_in(&dir));
        assert_eq!(e.state.reserved_uploads(), 0);
        until_asleep(&e).await;
    }
}

#[tokio::test]
async fn removing_a_phone_or_pausing_stops_an_upload_that_sends_nothing() {
    for how in ["remove", "pause", "pause and resume"] {
        for (uri, folder) in UPLOADS {
            let e = env();
            let cookie = pair(&e).await;
            let id = device_id(&e, &cookie).await;
            let (body, polled) = silent_body();
            let lan = e.lan.clone();
            let c = cookie.clone();
            let upload = tokio::spawn(async move { send(&lan, dev_req(Method::POST, uri, Some(&c), body, false)).await });
            polled.await.unwrap();
            match how {
                "remove" => assert_eq!(send(&e.local, host_req(Method::DELETE, &format!("/api/host/devices/{id}"), None)).await.status, StatusCode::NO_CONTENT),
                "pause" => e.state.set_paused(true),
                _ => {
                    e.state.set_paused(true);
                    e.state.set_paused(false);
                }
            }
            let r = tokio::time::timeout(Duration::from_secs(2), upload).await.expect("stops at once").unwrap();
            assert_eq!((r.status, r.json()["error"].as_str()), (StatusCode::SERVICE_UNAVAILABLE, Some("传输已被电脑端停止")), "{how} {uri}");
            let dir = e.state.sandbox().root().join(folder);
            assert!(files_in(&dir).iter().all(|n| !n.ends_with(".part") && n != "stalled.bin"), "{how} {uri}");
            assert_eq!(e.state.reserved_uploads(), 0);
            until_asleep(&e).await;
            if how == "pause and resume" {
                let again = dev_req(Method::POST, "/api/files/upload?dir=&name=again.txt&size=4", Some(&cookie), Body::from("data"), false);
                assert_eq!(send(&e.lan, again).await.status, StatusCode::OK, "a new upload goes through");
            }
        }
    }
}

/// The phone is removed, or sharing paused, just as the last byte arrives.
#[tokio::test]
async fn a_file_that_lands_as_its_phone_is_removed_is_not_kept() {
    for how in ["remove", "pause", "device gone before its message"] {
        for (uri, folder) in [("/api/files/upload?dir=&name=late.txt&size=4", ""), ("/api/messages/upload?name=late.txt&size=4", "收件箱")] {
            let e = env();
            let cookie = pair(&e).await;
            let id = device_id(&e, &cookie).await;
            let state = e.state.clone();
            let body = body_then(b"data", move || match how {
                "remove" => {
                    state.db.delete_device(&id).unwrap();
                    state.revoke(&id);
                }
                "pause" => state.set_paused(true),
                // Removing a device deletes it from the database first.
                _ => {
                    state.db.delete_device(&id).unwrap();
                }
            });
            let r = send(&e.lan, dev_req(Method::POST, uri, Some(&cookie), body, false)).await;
            let dir = e.state.sandbox().root().join(folder);
            if how == "device gone before its message" && folder.is_empty() {
                // Files uploads have no message: the file is the upload.
                assert_eq!(r.status, StatusCode::OK);
                continue;
            }
            assert_ne!(r.status, StatusCode::OK, "{how} {uri}");
            assert!(files_in(&dir).iter().all(|n| !n.ends_with(".part") && n != "late.txt"), "{how} {uri}: {:?}", files_in(&dir));
            assert_eq!(e.state.reserved_uploads(), 0);
        }
    }
}

/// Unix: Windows does not rename a folder with a file open in it.
#[cfg(unix)]
#[tokio::test]
async fn a_failed_upload_leaves_no_temp_file_in_its_renamed_folder() {
    for how in ["the phone goes away", "removed"] {
        let e = env();
        let cookie = pair(&e).await;
        let id = device_id(&e, &cookie).await;
        let root = e.state.sandbox().root().to_path_buf();
        std::fs::create_dir(root.join("photos")).unwrap();
        let (fail, wait) = tokio::sync::oneshot::channel::<()>();
        let first = futures_util::stream::once(async { Ok::<_, std::io::Error>(Bytes::from_static(b"first")) });
        let then = futures_util::stream::once(async move {
            let _ = wait.await;
            Err(std::io::Error::other("connection lost"))
        });
        let req = dev_req(Method::POST, "/api/files/upload?dir=photos&name=big.bin&size=100", Some(&cookie), Body::from_stream(first.chain(then)), false);
        let lan = e.lan.clone();
        let upload = tokio::spawn(async move { send(&lan, req).await.status });
        for _ in 0..300 {
            if files_in(&root.join("photos")).iter().any(|n| n.ends_with(".part")) && e.state.reserved_uploads() == 95 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let rename = dev_req(Method::POST, "/api/files/rename", Some(&cookie), Body::from(json!({ "path": "photos", "name": "album" }).to_string()), true);
        assert_eq!(send(&e.lan, rename).await.status, StatusCode::OK);
        assert!(files_in(&root.join("album")).iter().any(|n| n.ends_with(".part")), "the upload moved with its folder");
        if how == "removed" {
            assert_eq!(send(&e.local, host_req(Method::DELETE, &format!("/api/host/devices/{id}"), None)).await.status, StatusCode::NO_CONTENT);
            assert_eq!(upload.await.unwrap(), StatusCode::SERVICE_UNAVAILABLE);
        } else {
            let _ = fail.send(());
            assert_eq!(upload.await.unwrap(), StatusCode::BAD_REQUEST);
        }
        assert!(files_in(&root.join("album")).is_empty(), "{how}: {:?}", files_in(&root.join("album")));
        assert!(!root.join("photos").exists());
    }
}

/// Starts a download of a file of several chunks and reads its first one.
async fn download_started(e: &Env, cookie: &str) -> axum::body::BodyDataStream {
    std::fs::write(e.state.sandbox().root().join("big.bin"), vec![7u8; 4 << 20]).unwrap();
    let res = e.lan.clone().oneshot(dev_req(Method::GET, "/api/files/download?path=big.bin", Some(cookie), Body::empty(), false)).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let mut body = res.into_body().into_data_stream();
    assert!(!body.next().await.unwrap().unwrap().is_empty());
    assert!(e.state.awake.is_awake());
    body
}

/// What the rest of a download brings: it must fail, not end as if complete.
async fn rest_fails(mut body: axum::body::BodyDataStream) {
    let mut chunks = 0;
    loop {
        match tokio::time::timeout(Duration::from_secs(2), body.next()).await.expect("ends") {
            Some(Ok(_)) => chunks += 1,
            Some(Err(_)) => break,
            None => panic!("the download ended as if complete"),
        }
    }
    assert!(chunks <= 1, "at most the chunk already handed over: {chunks}");
}

#[tokio::test(start_paused = true)]
async fn a_download_nobody_reads_lets_go_after_two_minutes() {
    let e = env();
    let cookie = pair(&e).await;
    let body = download_started(&e, &cookie).await;
    tokio::time::sleep(TRANSFER_IDLE - Duration::from_secs(1)).await;
    assert!(e.state.awake.is_awake(), "not before");
    tokio::time::sleep(Duration::from_secs(2)).await;
    until_asleep(&e).await;
    rest_fails(body).await;
}

#[tokio::test]
async fn removing_a_phone_or_pausing_stops_a_download_nobody_reads() {
    for how in ["remove", "pause and resume"] {
        let e = env();
        let cookie = pair(&e).await;
        let id = device_id(&e, &cookie).await;
        let body = download_started(&e, &cookie).await;
        if how == "remove" {
            assert_eq!(send(&e.local, host_req(Method::DELETE, &format!("/api/host/devices/{id}"), None)).await.status, StatusCode::NO_CONTENT);
        } else {
            e.state.set_paused(true);
            e.state.set_paused(false);
        }
        until_asleep(&e).await;
        rest_fails(body).await;
    }
}

/// The limit is on time without data, not on the length of a transfer.
#[tokio::test(start_paused = true)]
async fn slow_but_steady_transfers_go_on() {
    let e = env();
    let cookie = pair(&e).await;
    let minute = || tokio::time::sleep(Duration::from_secs(60));
    let chunks = futures_util::stream::iter(0..4).then(move |i| async move {
        if i > 0 {
            minute().await;
        }
        Ok::<_, std::io::Error>(Bytes::from_static(b"part"))
    });
    let started = tokio::time::Instant::now();
    let r = send(&e.lan, dev_req(Method::POST, "/api/files/upload?dir=&name=slow.bin&size=16", Some(&cookie), Body::from_stream(chunks), false)).await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.json());
    assert!(started.elapsed() >= Duration::from_secs(180));
    assert_eq!(std::fs::read(e.state.sandbox().root().join("slow.bin")).unwrap(), b"partpartpartpart");

    let mut body = download_started(&e, &cookie).await;
    let mut total = 0;
    while let Some(chunk) = body.next().await {
        total += chunk.expect("not cut off").len();
        if total < 512 << 10 {
            minute().await;
        }
    }
    assert!(total > 3 << 20, "all of it: {total}");
    assert!(started.elapsed() >= Duration::from_secs(300));
}

#[tokio::test]
async fn downloads_read_to_the_end_are_complete() {
    let e = env();
    let cookie = pair(&e).await;
    let data: Vec<u8> = (0..3 << 20).map(|i| (i % 251) as u8).collect();
    std::fs::write(e.state.sandbox().root().join("big.bin"), &data).unwrap();
    let r = send(&e.lan, dev_req(Method::GET, "/api/files/download?path=big.bin", Some(&cookie), Body::empty(), false)).await;
    assert_eq!((r.status, r.body.len()), (StatusCode::OK, data.len()));
    assert!(r.body == data);
    let mut ranged = dev_req(Method::GET, "/api/files/download?path=big.bin", Some(&cookie), Body::empty(), false);
    ranged.headers_mut().insert(header::RANGE, "bytes=100-199".parse().unwrap());
    let r = send(&e.lan, ranged).await;
    assert_eq!((r.status, &r.body[..]), (StatusCode::PARTIAL_CONTENT, &data[100..200]));
    until_asleep(&e).await;
}

/// Everything an event stream sends until it ends (or goes quiet for a second).
async fn rest_of(stream: &mut axum::body::BodyDataStream) -> (String, bool) {
    let mut seen = String::new();
    loop {
        match tokio::time::timeout(Duration::from_secs(1), stream.next()).await {
            Ok(Some(Ok(chunk))) => seen.push_str(&String::from_utf8_lossy(&chunk)),
            Ok(Some(Err(_))) | Ok(None) => return (seen, true),
            Err(_) => return (seen, false),
        }
    }
}

async fn open_events(app: &Router, req: Request<Body>) -> axum::body::BodyDataStream {
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    res.into_body().into_data_stream()
}

#[tokio::test]
async fn a_removed_phone_hears_so_and_its_stream_ends() {
    for how in ["removed", "unpaired", "removed while not reading", "removed before reading"] {
        let e = env();
        let cookie = pair(&e).await;
        let id = device_id(&e, &cookie).await;
        let mut stream = open_events(&e.lan, dev_req(Method::GET, "/api/events", Some(&cookie), Body::empty(), false)).await;
        if how != "removed before reading" {
            assert!(String::from_utf8_lossy(&stream.next().await.unwrap().unwrap()).contains("connected"));
        }
        match how {
            "unpaired" => assert_eq!(send(&e.lan, dev_req(Method::POST, "/api/me/unpair", Some(&cookie), Body::empty(), false)).await.status, StatusCode::OK),
            _ => assert_eq!(send(&e.local, host_req(Method::DELETE, &format!("/api/host/devices/{id}"), None)).await.status, StatusCode::NO_CONTENT),
        }
        if how == "removed while not reading" {
            // Far more than the stream keeps for a reader that falls behind.
            for _ in 0..600 {
                e.state.emit(crate::core::events::Audience::Everyone, "files", &json!({ "dir": "after-removal" }));
            }
        }
        let (seen, ended) = rest_of(&mut stream).await;
        assert!(ended, "{how}: {seen}");
        assert_eq!(seen.matches("event: ").count(), 1, "{how}: only that it was removed: {seen}");
        assert!(seen.contains("event: revoked"), "{how}: {seen}");
        if how == "removed before reading" {
            assert!(!seen.contains("connected"), "{seen}");
        }
    }
}

#[tokio::test]
async fn pausing_ends_a_phones_stream_with_the_reason_but_not_the_computers() {
    let e = env();
    let cookie = pair(&e).await;
    let mut phone = open_events(&e.lan, dev_req(Method::GET, "/api/events", Some(&cookie), Body::empty(), false)).await;
    let mut host = open_events(&e.local, host_req(Method::GET, "/api/events", None)).await;
    assert!(String::from_utf8_lossy(&phone.next().await.unwrap().unwrap()).contains("connected"));
    assert!(String::from_utf8_lossy(&host.next().await.unwrap().unwrap()).contains("connected"));
    e.state.set_paused(true);
    let (seen, ended) = rest_of(&mut phone).await;
    assert!(ended && seen.contains("event: paused") && seen.contains("\"paused\":true"), "{seen}");
    e.state.set_paused(false);
    let (seen, ended) = rest_of(&mut host).await;
    assert!(!ended && seen.contains("\"paused\":true") && seen.contains("\"paused\":false"), "{seen}");
}

fn pair_req(body: Body) -> Request<Body> {
    dev_req(Method::POST, "/api/pair/request", None, body, true)
}

#[tokio::test(start_paused = true)]
async fn pairing_requests_are_read_within_bounds() {
    let e = env();
    let large = Body::from(vec![b' '; 5000]);
    assert_eq!(send(&e.lan, pair_req(large)).await.status, StatusCode::PAYLOAD_TOO_LARGE);
    let (body, _polled) = silent_body();
    let started = tokio::time::Instant::now();
    assert_eq!(send(&e.lan, pair_req(body)).await.status, StatusCode::REQUEST_TIMEOUT);
    assert_eq!(started.elapsed(), Duration::from_secs(10));
    // A real one still gets through.
    pair(&e).await;
}

#[tokio::test]
async fn only_a_few_pairing_requests_are_read_at_once() {
    let e = env();
    let mut reading = Vec::new();
    for _ in 0..4 {
        let (body, polled) = silent_body();
        let lan = e.lan.clone();
        reading.push(tokio::spawn(async move { send(&lan, pair_req(body)).await }));
        polled.await.unwrap();
    }
    let (body, mut polled) = silent_body();
    assert_eq!(send(&e.lan, pair_req(body)).await.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(polled.try_recv().is_err(), "refused without reading it");
    for r in reading {
        r.abort();
        let _ = r.await;
    }
    pair(&e).await;
}
