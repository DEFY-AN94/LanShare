//! A path cannot be swapped between its check and its use: a phone's rename
//! waits for checks in progress (at most `PATH_CHANGE_WAIT`), and transfers
//! hold paths only while they open or place a file.

use futures_util::StreamExt as _;

use super::*;
use crate::core::bridge::Clip;
use crate::core::quick;
use crate::core::state::PATH_CHANGE_WAIT;

fn rename(cookie: &str, path: &str, name: &str) -> Request<Body> {
    dev_req(Method::POST, "/api/files/rename", Some(cookie), Body::from(json!({ "path": path, "name": name }).to_string()), true)
}

#[tokio::test]
async fn a_rename_waits_for_a_path_being_checked() {
    let e = env();
    let cookie = pair(&e).await;
    std::fs::create_dir(e.state.sandbox().root().join("photos")).unwrap();
    let check = e.state.path_check().await;
    let lan = e.lan.clone();
    let c = cookie.clone();
    let renaming = tokio::spawn(async move { send(&lan, rename(&c, "photos", "pictures")).await });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!renaming.is_finished(), "waits while a path is being checked");
    drop(check);
    assert_eq!(renaming.await.unwrap().status, StatusCode::OK);
}

#[tokio::test]
async fn a_rename_gives_up_after_waiting() {
    let e = env();
    let cookie = pair(&e).await;
    std::fs::create_dir(e.state.sandbox().root().join("photos")).unwrap();
    let _check = e.state.path_check().await;
    let started = std::time::Instant::now();
    let r = send(&e.lan, rename(&cookie, "photos", "pictures")).await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    assert!(r.json()["error"].as_str().unwrap().contains("稍后再试"));
    assert!(started.elapsed() >= PATH_CHANGE_WAIT);
    assert!(e.state.sandbox().root().join("photos").is_dir());
}

#[tokio::test]
async fn a_download_waits_for_a_rename_in_progress() {
    let e = env();
    let cookie = pair(&e).await;
    std::fs::write(e.state.sandbox().root().join("a.txt"), b"hello").unwrap();
    let change = e.state.path_change().await.unwrap();
    let lan = e.lan.clone();
    let c = cookie.clone();
    let downloading = tokio::spawn(async move { send(&lan, dev_req(Method::GET, "/api/files/download?path=a.txt", Some(&c), Body::empty(), false)).await });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!downloading.is_finished());
    drop(change);
    assert_eq!(downloading.await.unwrap().body.as_ref(), b"hello");
}

/// A request body that sends `first`, then waits until `go` before the rest.
fn held_body(first: &'static [u8], rest: &'static [u8]) -> (Body, tokio::sync::oneshot::Sender<()>) {
    let (go, wait) = tokio::sync::oneshot::channel::<()>();
    let a = futures_util::stream::once(async move { Ok::<_, std::io::Error>(Bytes::from_static(first)) });
    let b = futures_util::stream::once(async move {
        let _ = wait.await;
        Ok(Bytes::from_static(rest))
    });
    (Body::from_stream(a.chain(b)), go)
}

async fn until_part_file_in(dir: &std::path::Path) {
    for _ in 0..300 {
        let parts = std::fs::read_dir(dir).unwrap().filter_map(|x| x.ok()).filter(|x| x.file_name().to_string_lossy().ends_with(".part")).count();
        if parts > 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the upload did not start");
}

/// Uploads hold paths only while they open and place their file: a long
/// upload does not make renames and deletes wait or fail.
#[tokio::test]
async fn an_upload_in_progress_does_not_hold_up_renames() {
    let e = env();
    let cookie = pair(&e).await;
    let root = e.state.sandbox().root().to_path_buf();
    std::fs::create_dir(root.join("photos")).unwrap();
    let (body, go) = held_body(b"first chunk", b"second chunk");
    let req = dev_req(Method::POST, "/api/files/upload?dir=&name=big.bin&size=23", Some(&cookie), body, false);
    let lan = e.lan.clone();
    let upload = tokio::spawn(async move { send(&lan, req).await });
    until_part_file_in(&root).await;
    let r = tokio::time::timeout(Duration::from_secs(1), send(&e.lan, rename(&cookie, "photos", "pictures"))).await.expect("not held up");
    assert_eq!(r.status, StatusCode::OK);
    let _ = go.send(());
    assert_eq!(upload.await.unwrap().status, StatusCode::OK);
    assert_eq!(std::fs::read(root.join("big.bin")).unwrap(), b"first chunksecond chunk");
}

/// A folder renamed away during an upload, with a link given its name, does
/// not receive the file through the link.
#[cfg(unix)]
#[tokio::test]
async fn an_upload_whose_folder_was_swapped_for_a_link_is_refused() {
    let e = env();
    let cookie = pair(&e).await;
    let root = e.state.sandbox().root().to_path_buf();
    let outside = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.join("photos")).unwrap();
    let (body, go) = held_body(b"first chunk", b"second chunk");
    let req = dev_req(Method::POST, "/api/files/upload?dir=photos&name=big.bin&size=23", Some(&cookie), body, false);
    let lan = e.lan.clone();
    let upload = tokio::spawn(async move { send(&lan, req).await });
    until_part_file_in(&root.join("photos")).await;
    std::fs::rename(root.join("photos"), root.join("elsewhere")).unwrap();
    std::os::unix::fs::symlink(outside.path(), root.join("photos")).unwrap();
    let _ = go.send(());
    let r = upload.await.unwrap();
    assert_eq!(r.status, StatusCode::CONFLICT, "{:?}", r.json());
    assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0, "nothing written through the link");
}

/// A picture copied in Finder from the shared folder goes to a phone only if
/// phones could download it there anyway, and is read from the file checked.
#[tokio::test]
async fn clipboard_pictures_from_the_shared_folder_must_be_downloadable() {
    let (e, bridge) = super::m8::recorded();
    let cookie = pair(&e).await;
    let device = device_id(&e, &cookie).await;
    let root = e.state.sandbox().root().to_path_buf();
    std::fs::create_dir_all(root.join("photos")).unwrap();
    std::fs::create_dir_all(root.join("收件箱")).unwrap();
    std::fs::write(root.join("photos").join("cat.png"), b"\x89PNGcat").unwrap();
    std::fs::write(root.join("收件箱").join("other-phone.png"), b"\x89PNGother").unwrap();
    let set = |paths: Vec<std::path::PathBuf>, image: Option<Vec<u8>>| *bridge.clipboard.lock().unwrap() = Some(Clip::Files { paths, image });

    set(vec![root.join("photos").join("cat.png")], None);
    let (_, sent) = quick::send_clipboard(&e.state, Some(&device)).await.unwrap();
    assert_eq!(sent.file.unwrap().name, "cat.png");

    // From a chat folder: refused, and the clipboard's own image does not go instead.
    set(vec![root.join("收件箱").join("other-phone.png")], Some(b"\x89PNGimage".to_vec()));
    let err = quick::send_clipboard(&e.state, Some(&device)).await.unwrap_err();
    assert!(err.contains("手机打不开的位置"), "{err}");

    #[cfg(unix)]
    {
        // Through a link in the shared folder that leads outside it.
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("private.png"), b"\x89PNGprivate").unwrap();
        std::os::unix::fs::symlink(outside.path().join("private.png"), root.join("photos").join("link.png")).unwrap();
        set(vec![root.join("photos").join("link.png")], None);
        let err = quick::send_clipboard(&e.state, Some(&device)).await.unwrap_err();
        assert!(err.contains("手机打不开的位置"), "{err}");
    }
    let sent: Vec<_> = e.state.db.messages(&device, None, None, 50).unwrap().0.into_iter().filter(|m| m.from_host).collect();
    assert_eq!(sent.len(), 1, "only the downloadable picture went");
}
