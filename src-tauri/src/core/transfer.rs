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
use tower::ServiceExt as _;
use tower_http::services::ServeFile;

use super::auth::Session;
use super::error::{ApiError, ApiResult};
use super::paths::unique_path;
use super::state::Shared;
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
    let mut res: Response = res.map(move |body| {
        let guarded = Body::new(body).into_data_stream().map(move |chunk| {
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
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, no-cache"));
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

/// Streams the request body into `dir/name` (renamed `name (1)` etc. on
/// collision). Writes to a hidden temp file first so a half-finished upload
/// never appears under its real name.
pub async fn receive_upload(state: &Shared, session: &Session, body: Body, dir: &Path, name: &str) -> ApiResult<(PathBuf, u64)> {
    let dir_owned = dir.to_path_buf();
    let _ = tokio::task::spawn_blocking(move || sweep_stale_parts(&dir_owned, Duration::from_secs(24 * 3600))).await;

    let mut tmp = TempFile { path: dir.join(format!(".lanshare-{}.part", random_hex(8))), keep: false };
    let file = tokio::fs::File::create(&tmp.path).await?;
    let mut out = BufWriter::with_capacity(1 << 20, file);
    let mut stream = body.into_data_stream();
    let mut total: u64 = 0;
    while let Some(chunk) = stream.next().await {
        if !state.transfer_allowed(session) {
            return Err(ApiError::new(axum::http::StatusCode::SERVICE_UNAVAILABLE, "传输已被电脑端停止"));
        }
        let chunk = chunk.map_err(|_| ApiError::bad_request("上传中断"))?;
        total += chunk.len() as u64;
        out.write_all(&chunk).await?;
    }
    out.flush().await?;
    drop(out);

    let _guard = state.upload_lock.lock().await;
    let target = unique_path(dir, name);
    tokio::fs::rename(&tmp.path, &target).await?;
    tmp.keep = true;
    if !session.is_host() {
        mark_quarantined(&target);
    }
    Ok((target, total))
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
