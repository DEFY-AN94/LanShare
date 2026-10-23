//! Streaming file transfer: downloads with Range support, uploads straight to disk.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use axum::body::Body;
use axum::extract::Request;
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::Response;
use futures_util::StreamExt;
use mime_guess::Mime;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::OwnedRwLockReadGuard;
use tower::ServiceExt as _;
use tower_http::services::ServeFile;

use super::auth::Session;
use super::disk::{self, FAT_MAX_FILE, human_size};
use super::error::{ApiError, ApiResult};
use super::paths::unique_path;
use super::state::{PATH_CHANGE_WAIT, Shared};
use super::util::random_hex;

/// RFC 8187 attr-char: everything else is percent-encoded in `filename*`.
const ATTR_CHAR: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'!')
    .remove(b'#')
    .remove(b'$')
    .remove(b'&')
    .remove(b'+')
    .remove(b'-')
    .remove(b'.')
    .remove(b'^')
    .remove(b'_')
    .remove(b'`')
    .remove(b'|')
    .remove(b'~');

pub fn content_disposition(kind: &str, file_name: &str) -> HeaderValue {
    let fallback: String = file_name
        .chars()
        .map(|c| if (c.is_ascii_graphic() && c != '"' && c != '\\' && c != '%') || c == ' ' { c } else { '_' })
        .collect();
    let encoded = utf8_percent_encode(file_name, ATTR_CHAR);
    HeaderValue::from_str(&format!("{kind}; filename=\"{fallback}\"; filename*=UTF-8''{encoded}"))
        .unwrap_or_else(|_| HeaderValue::from_static("attachment"))
}

const TEXT_EXTS: &[&str] = &[
    "txt", "md", "log", "csv", "json", "xml", "yaml", "yml", "ini", "conf", "toml", "js", "ts", "py", "rs", "go", "java", "c", "cpp", "h",
    "sh", "css", "sql", "html", "htm",
];

/// Decides whether a file may be shown inline and with which Content-Type.
/// Anything that could run script in our origin (HTML, SVG, XML…) is only
/// shown as plain text or downloaded.
pub fn inline_policy(name: &str) -> (bool, Mime) {
    let mime = mime_guess::from_path(name).first_or_octet_stream();
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match (mime.type_().as_str(), mime.subtype().as_str()) {
        ("image", "svg") => (false, mime),
        ("image", _) | ("video", _) | ("audio", _) => (true, mime),
        ("application", "pdf") => (true, mime),
        _ if TEXT_EXTS.contains(&ext.as_str()) || mime.type_() == mime_guess::mime::TEXT => (true, mime_guess::mime::TEXT_PLAIN_UTF_8),
        _ => (false, mime),
    }
}

fn clone_head(parts: &Parts, strip_range: bool) -> Request {
    let mut r = Request::new(Body::empty());
    *r.method_mut() = parts.method.clone();
    *r.uri_mut() = parts.uri.clone();
    *r.headers_mut() = parts.headers.clone();
    if strip_range {
        r.headers_mut().remove(header::RANGE);
    }
    r
}

/// Serves `path` (already sandbox-checked) with Range/ETag support. The body
/// stops mid-way if sharing is paused or the device is removed.
pub async fn serve_path(state: &Shared, session: &Session, path: &Path, file_name: &str, want_inline: bool, parts: &Parts) -> Response {
    let (inline_ok, mime) = inline_policy(file_name);
    let inline = want_inline && inline_ok;
    let svc = ServeFile::new_with_mime(path, &mime).ignore_multi_range_requests(true);
    let res = match svc.clone().oneshot(clone_head(parts, false)).await {
        Ok(r) => r,
        Err(never) => match never {},
    };
    // tower-http does not implement If-Range: if the validator is stale, send the whole file.
    let res = match (res.status(), parts.headers.get(header::IF_RANGE)) {
        (StatusCode::PARTIAL_CONTENT | StatusCode::RANGE_NOT_SATISFIABLE, Some(if_range)) => {
            let current = if if_range.as_bytes().starts_with(b"\"") {
                res.headers().get(header::ETAG)
            } else {
                res.headers().get(header::LAST_MODIFIED)
            };
            if current == Some(if_range) {
                res
            } else {
                match svc.oneshot(clone_head(parts, true)).await {
                    Ok(r) => r,
                    Err(never) => match never {},
                }
            }
        }
        _ => res,
    };
    let (st, sess) = (state.clone(), session.clone());
    // Lives as long as the body: the computer stays awake until the download ends.
    let awake = state.awake.hold();
    let mut res: Response = res.map(move |body| {
        let guarded = Body::new(body).into_data_stream().map(move |chunk| {
            let _ = &awake;
            if st.transfer_allowed(&sess) {
                chunk.map_err(std::io::Error::other)
            } else {
                Err(std::io::Error::other("transfer cancelled"))
            }
        });
        Body::from_stream(guarded)
    });
    let ok = matches!(res.status(), StatusCode::OK | StatusCode::PARTIAL_CONTENT);
    let h = res.headers_mut();
    if ok {
        h.insert(header::CONTENT_DISPOSITION, content_disposition(if inline { "inline" } else { "attachment" }, file_name));
    }
    // A sandboxed document cannot run script or reach our cookies' origin.
    // PDFs are exempt because browser PDF viewers break under `sandbox`.
    if mime.subtype() != mime_guess::mime::PDF {
        h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("sandbox"));
    }
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    // No copy stays in a browser cache after unpairing or clearing the chat.
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    res
}

fn is_part_file(name: &str) -> bool {
    name.starts_with(".lanshare-") && name.ends_with(".part")
}

/// Removes upload leftovers from crashed or aborted transfers older than `max_age`.
fn sweep_stale_parts(dir: &Path, max_age: Duration) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let cutoff = SystemTime::now() - max_age;
    for e in rd.flatten() {
        if is_part_file(&e.file_name().to_string_lossy()) && e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t < cutoff) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// At startup: clear leftovers a crash or quit left behind, a few levels deep.
pub fn sweep_tree(root: &Path, depth: usize) {
    sweep_stale_parts(root, Duration::from_secs(60));
    if depth == 0 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(root) else { return };
    for e in rd.flatten() {
        let name = e.file_name();
        if !name.to_string_lossy().starts_with('.') && e.file_type().is_ok_and(|t| t.is_dir()) {
            sweep_tree(&e.path(), depth - 1);
        }
    }
}

/// Deletes the temp file unless the upload completed (also when the request
/// future is dropped mid-transfer).
struct TempFile {
    path: PathBuf,
    keep: bool,
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Files received from a phone get the same "downloaded from elsewhere" mark
/// a browser would add, so macOS checks them before opening or running them.
#[cfg(target_os = "macos")]
pub fn mark_quarantined(path: &Path) {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let secs = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let value = format!("0081;{secs:08x};LanShare;");
    let (Ok(p), Ok(name)) = (CString::new(path.as_os_str().as_bytes()), CString::new("com.apple.quarantine")) else { return };
    // SAFETY: valid NUL-terminated strings and a buffer of the given length.
    unsafe {
        libc::setxattr(p.as_ptr(), name.as_ptr(), value.as_ptr().cast(), value.len(), 0, 0);
    }
}

/// Windows: the Zone.Identifier stream makes SmartScreen treat it as downloaded.
#[cfg(windows)]
pub fn mark_quarantined(path: &Path) {
    let mut ads = path.as_os_str().to_owned();
    ads.push(":Zone.Identifier");
    let _ = std::fs::write(PathBuf::from(ads), "[ZoneTransfer]\r\nZoneId=3\r\n");
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn mark_quarantined(_path: &Path) {}

/// The size the client declared for the request body.
pub fn declared_len(headers: &HeaderMap) -> Option<u64> {
    headers.get(header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok())
}

/// Room an upload in progress holds on the disk. Its length is promised up
/// front and handed back as the data lands, since the disk's free space
/// counts it from then on; whatever is left is released when the upload ends.
pub struct Reservation {
    state: Shared,
    /// Promised bytes not yet written.
    bytes: u64,
}

impl Reservation {
    /// `n` more bytes are being written.
    pub fn landed(&mut self, n: u64) {
        let n = n.min(self.bytes);
        if n > 0 {
            self.bytes -= n;
            self.state.release_upload(n);
        }
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.state.release_upload(self.bytes);
    }
}

fn too_big_for_fat() -> ApiError {
    ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "保存位置是 FAT32 格式（常见于 U 盘），单个文件不能超过 4 GB")
}

/// `in_flight`: what uploads already running still have to write.
fn no_room(available: u64, in_flight: u64, need: u64, reserve: u64) -> ApiError {
    let running = if in_flight > 0 { format!("（其中 {} 留给正在进行的上传）", human_size(in_flight)) } else { String::new() };
    ApiError::new(
        StatusCode::INSUFFICIENT_STORAGE,
        format!(
            "电脑磁盘空间不足：还剩 {}{running}，这个文件 {}，还要给电脑留出 {}",
            human_size(available),
            human_size(need),
            human_size(reserve)
        ),
    )
}

/// Refuses an upload before its body is read if it cannot fit: over 4 GB on
/// a FAT disk, or more than the free space minus the reserve (section 3 of
/// PLAN.md) and what other uploads in progress still have to write. Call it
/// before taking the body, so a refusal reaches the client, and pass the
/// reservation to [`receive_upload`].
pub fn preflight(state: &Shared, dir: &Path, length: u64) -> ApiResult<Option<Reservation>> {
    let info = match disk::disk_info(dir) {
        Ok(info) => info,
        // Unknown disk: the write itself reports a full disk.
        Err(_) => return Ok(None),
    };
    if info.fat && length > FAT_MAX_FILE {
        return Err(too_big_for_fat());
    }
    let reserve = state.upload_reserve.unwrap_or_else(|| disk::reserve_for(info.total));
    let mut in_flight = 0;
    let fits = |reserved: u64| {
        in_flight = reserved;
        info.available.saturating_sub(reserved) >= length.saturating_add(reserve)
    };
    if !state.try_reserve_upload(length, fits) {
        return Err(no_room(info.available, in_flight, length, reserve));
    }
    Ok(Some(Reservation { state: state.clone(), bytes: length }))
}

/// The same check without reserving anything: lets a phone ask before it
/// sends a large file, so a refusal is a clear message rather than a reset.
pub fn check_fits(state: &Shared, dir: &Path, size: u64) -> ApiResult<()> {
    preflight(state, dir, size).map(drop)
}

/// The upload's length: its Content-Length, checked against the file size
/// the phone put in the URL. On iOS 26.5 and later a picked file can go out
/// as an empty body (WebKit bug 319985), which must not be saved as a
/// complete, empty file. An upload with neither is refused: without a length
/// it could not be held to the free-space reserve.
pub fn upload_length(declared: Option<u64>, size: Option<u64>) -> ApiResult<u64> {
    match (declared, size) {
        (Some(d), Some(s)) if d != s => Err(ApiError::bad_request(format!(
            "手机发出的数据（{}）和文件大小（{}）不符，请重新选择这个文件发送",
            human_size(d),
            human_size(s)
        ))),
        (Some(n), _) | (None, Some(n)) => Ok(n),
        (None, None) => Err(ApiError::new(StatusCode::LENGTH_REQUIRED, "上传没有说明文件大小，请从 LanShare 的页面重新上传")),
    }
}

/// Runs blocking work on paths checked under `check` (`AppState::path_check`)
/// and keeps the check until the work is done, also if the request goes away
/// meanwhile, but no longer than `PATH_CHANGE_WAIT`: a system prompt (a
/// Windows "delete for good?" dialog) must not hold up every rename.
pub async fn checked_blocking<T: Send + 'static>(check: OwnedRwLockReadGuard<()>, work: impl FnOnce() -> T + Send + 'static) -> ApiResult<T> {
    let mut work = tokio::task::spawn_blocking(work);
    let (done, result) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let out = match tokio::time::timeout(PATH_CHANGE_WAIT, &mut work).await {
            Ok(out) => out,
            Err(_) => {
                drop(check);
                work.await
            }
        };
        let _ = done.send(out);
    });
    result.await.map_err(ApiError::internal)?.map_err(ApiError::internal)
}

/// What a folder is, to tell later whether a rename put another in its place
/// (a link given its name). A case-only rename keeps it.
#[cfg(unix)]
fn folder_id(dir: &Path) -> std::io::Result<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::metadata(dir)?;
    Ok((m.dev(), m.ino()))
}

/// Windows disks ignore case in names.
#[cfg(not(unix))]
fn folder_id(dir: &Path) -> std::io::Result<String> {
    Ok(dunce::canonicalize(dir)?.to_string_lossy().to_lowercase())
}

/// Streams the request body into `dir/name` (renamed `name (1)` etc. on
/// collision). Writes to a hidden temp file first so a half-finished upload
/// never appears under its real name. `length` is what [`upload_length`]
/// returned, and `reservation` what [`preflight`] gave before the body was
/// taken. `check` was held since `dir` was checked: it is let go once the
/// temp file exists, and taken again to move the finished file into place,
/// in the same folder, so a long upload never holds up renames.
#[allow(clippy::too_many_arguments)]
pub async fn receive_upload(
    state: &Shared,
    session: &Session,
    body: Body,
    dir: &Path,
    name: &str,
    length: u64,
    mut reservation: Option<&mut Reservation>,
    check: OwnedRwLockReadGuard<()>,
) -> ApiResult<(PathBuf, u64)> {
    let _awake = state.awake.hold();
    let tmp_path = dir.join(format!(".lanshare-{}.part", random_hex(8)));
    let (d, t) = (dir.to_path_buf(), tmp_path.clone());
    let (file, folder) = checked_blocking(check, move || -> std::io::Result<_> {
        sweep_stale_parts(&d, Duration::from_secs(24 * 3600));
        let folder = folder_id(&d)?;
        Ok((std::fs::File::create(&t)?, folder))
    })
    .await??;
    let mut tmp = TempFile { path: tmp_path, keep: false };
    let mut out = BufWriter::with_capacity(1 << 20, tokio::fs::File::from_std(file));
    let mut stream = body.into_data_stream();
    let mut total: u64 = 0;
    while let Some(chunk) = stream.next().await {
        if !state.transfer_allowed(session) {
            return Err(ApiError::new(axum::http::StatusCode::SERVICE_UNAVAILABLE, "传输已被电脑端停止"));
        }
        let chunk = chunk.map_err(|_| ApiError::bad_request("上传中断"))?;
        total += chunk.len() as u64;
        if total > length {
            return Err(ApiError::bad_request("上传的数据比声明的大小多，已取消"));
        }
        if let Some(r) = reservation.as_deref_mut() {
            r.landed(chunk.len() as u64);
        }
        out.write_all(&chunk).await?;
    }
    if total != length {
        return Err(ApiError::bad_request("上传不完整，请重试"));
    }
    out.flush().await?;
    // On disk before it gets its real name, so a power cut cannot leave a
    // file that looks complete but is not.
    out.into_inner().sync_all().await?;

    let (st, d, t, n) = (state.clone(), dir.to_path_buf(), tmp.path.clone(), name.to_string());
    let target = checked_blocking(state.path_check().await, move || -> ApiResult<PathBuf> {
        // Gone altogether (an unplugged disk) is reported as such by the caller.
        if folder_id(&d)? != folder {
            return Err(ApiError::conflict("上传的目标文件夹刚被改动，请重新上传"));
        }
        let _guard = st.upload_lock.blocking_lock();
        let target = unique_path(&d, &n);
        #[cfg(windows)]
        let created = std::fs::metadata(&t).and_then(|m| m.created()).ok();
        std::fs::rename(&t, &target)?;
        #[cfg(windows)]
        keep_creation_time(&target, created);
        Ok(target)
    })
    .await??;
    tmp.keep = true;
    if !session.is_host() {
        mark_quarantined(&target);
    }
    Ok((target, total))
}

/// Windows file-system tunneling gives a file that takes a name the creation
/// time of a file that left that name moments before. The upload keeps its
/// own, so it is never taken for an earlier attachment (`state::same_file`).
#[cfg(windows)]
fn keep_creation_time(path: &Path, created: Option<SystemTime>) {
    use std::os::windows::fs::FileTimesExt;
    let Some(created) = created else { return };
    if let Ok(file) = std::fs::OpenOptions::new().write(true).open(path) {
        let _ = file.set_times(std::fs::FileTimes::new().set_created(created));
    }
}

/// An upload that failed because the shared folder went away meanwhile (its
/// disk unplugged mid-transfer) says so, instead of a generic error.
pub fn unless_root_gone(state: &Shared, e: ApiError) -> ApiError {
    let disk_trouble = matches!(e.status, StatusCode::FORBIDDEN | StatusCode::NOT_FOUND | StatusCode::INTERNAL_SERVER_ERROR);
    match state.usable_sandbox() {
        Err(why) if disk_trouble => why.into(),
        _ => e,
    }
}

/// The most of a rejected upload that is read before giving up on the connection.
pub const REJECTED_UPLOAD_DRAIN: u64 = 16 << 20;

/// Reads and drops a request body the handler did not use, so the client gets
/// the response. Answering while the client is still sending makes hyper close
/// the connection with data unread: the client fails mid-send, and on Windows
/// the reset also throws away the response it had already received, so a
/// retried upload that went through looks failed. A client that sent
/// `Expect: 100-continue` holds the body back until it is polled, so there is
/// nothing to drain. Gives up after `limit` bytes or a 20 s stall.
pub async fn discard_body(headers: &HeaderMap, body: Body, limit: u64) {
    let expects_continue = headers.get(header::EXPECT).is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"100-continue"));
    if expects_continue {
        return;
    }
    let mut stream = body.into_data_stream();
    let mut seen: u64 = 0;
    while seen <= limit {
        match tokio::time::timeout(Duration::from_secs(20), stream.next()).await {
            Ok(Some(Ok(chunk))) => seen += chunk.len() as u64,
            _ => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_rules() {
        assert_eq!(inline_policy("a.jpg"), (true, mime_guess::mime::IMAGE_JPEG));
        assert!(inline_policy("clip.MOV").0);
        assert!(inline_policy("doc.pdf").0);
        assert!(!inline_policy("x.svg").0);
        assert!(!inline_policy("app.exe").0);
        // Script-capable text is shown as text/plain, never rendered.
        assert_eq!(inline_policy("page.html"), (true, mime_guess::mime::TEXT_PLAIN_UTF_8));
        assert_eq!(inline_policy("notes.md"), (true, mime_guess::mime::TEXT_PLAIN_UTF_8));
    }

    #[test]
    fn disposition_encodes_unicode() {
        let v = content_disposition("attachment", "报告 \"v1\".pdf");
        assert_eq!(v.to_str().unwrap(), "attachment; filename=\"__ _v1_.pdf\"; filename*=UTF-8''%E6%8A%A5%E5%91%8A%20%22v1%22.pdf");
    }
}
