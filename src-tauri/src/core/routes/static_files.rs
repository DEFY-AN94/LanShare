//! Serves the built web UI (`dist/`) to both the desktop window and phones.

use axum::extract::OriginalUri;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

/// Debug builds read `dist/` from disk at runtime; release builds embed it.
#[derive(RustEmbed)]
#[folder = "../dist/"]
#[allow_missing = true]
struct Assets;

const PAGE_CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; \
media-src 'self' blob:; connect-src 'self'; font-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'";

pub async fn handler(method: Method, OriginalUri(uri): OriginalUri, headers: HeaderMap) -> Response {
    if method != Method::GET && method != Method::HEAD {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    let path = uri.path().trim_start_matches('/');
    if path.starts_with("api/") {
        return (StatusCode::NOT_FOUND, axum::Json(serde_json::json!({ "error": "接口不存在" }))).into_response();
    }
    let candidate = if path.is_empty() { "index.html" } else { path };
    let (name, file) = match Assets::get(candidate) {
        Some(f) => (candidate, f),
        // Client-side routes (e.g. /pair) get the app shell; missing files 404.
        None if !candidate.rsplit('/').next().unwrap_or("").contains('.') => match Assets::get("index.html") {
            Some(f) => ("index.html", f),
            None => return not_built(),
        },
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    let etag = format!("\"{}\"", hex::encode(file.metadata.sha256_hash()));
    let is_html = name.ends_with(".html");
    let cache = if name.starts_with("assets/") { "public, max-age=31536000, immutable" } else { "no-cache" };
    let mut res = if headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()) == Some(etag.as_str()) {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        let mime = mime_guess::from_path(name).first_or_octet_stream();
        let ctype = if mime.type_() == mime_guess::mime::TEXT || name.ends_with(".js") || name.ends_with(".webmanifest") {
            format!("{mime}; charset=utf-8")
        } else {
            mime.to_string()
        };
        ([(header::CONTENT_TYPE, ctype)], file.data.into_owned()).into_response()
    };
    let h = res.headers_mut();
    h.insert(header::ETAG, HeaderValue::from_str(&etag).expect("hex etag"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    if is_html {
        h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(PAGE_CSP));
    }
    res
}

fn not_built() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        "网页界面还没有构建：请在项目目录运行 npm run build",
    )
        .into_response()
}
