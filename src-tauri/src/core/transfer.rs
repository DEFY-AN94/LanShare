//! Streaming file transfer: downloads with Range support, uploads straight to disk.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use axum::body::Body;
use axum::extract::Request;
use axum::http::request::Parts;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use futures_util::StreamExt;
use mime_guess::Mime;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use tokio::io::{AsyncWriteExt, BufWriter};
use tower::ServiceExt as _;
use tower_http::services::ServeFile;

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

/// Serves `path` (already sandbox-checked) with Range/ETag support.
pub async fn serve_path(path: &Path, file_name: &str, want_inline: bool, parts: &Parts) -> Response {
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
    let mut res: Response = res.map(Body::new);
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

/// Removes upload leftovers from crashed or aborted transfers (older than a day).
fn sweep_stale_parts(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let cutoff = SystemTime::now() - Duration::from_secs(24 * 3600);
    for e in rd.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(".lanshare-") && name.ends_with(".part")
            && e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t < cutoff)
        {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Streams the request body into `dir/name` (renamed `name (1)` etc. on
/// collision). Writes to a hidden temp file first so a half-finished upload
/// never appears under its real name.
pub async fn receive_upload(state: &Shared, body: Body, dir: &Path, name: &str) -> ApiResult<(PathBuf, u64)> {
    let dir_owned = dir.to_path_buf();
    let _ = tokio::task::spawn_blocking(move || sweep_stale_parts(&dir_owned)).await;

    let tmp = dir.join(format!(".lanshare-{}.part", random_hex(8)));
    let file = tokio::fs::File::create(&tmp).await?;
    let mut out = BufWriter::with_capacity(1 << 20, file);
    let mut stream = body.into_data_stream();
    let mut total: u64 = 0;
    let result: ApiResult<()> = async {
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| ApiError::bad_request("上传中断"))?;
            total += chunk.len() as u64;
            out.write_all(&chunk).await?;
        }
        out.flush().await?;
        Ok(())
    }
    .await;
    drop(out);
    if let Err(e) = result {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(e);
    }

    let _guard = state.upload_lock.lock().await;
    let target = unique_path(dir, name);
    if let Err(e) = tokio::fs::rename(&tmp, &target).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(e.into());
    }
    Ok((target, total))
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
