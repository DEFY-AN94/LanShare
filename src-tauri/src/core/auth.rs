//! Who is calling, and from where.
//!
//! - LAN listener (HTTPS): only private-network peers; paired devices carry
//!   a `ls_dev=<id>.<secret>` cookie whose secret is stored hashed.
//! - Loopback listener (HTTP): only the desktop window, signed in with the
//!   per-launch local token. Only this listener can act as the host.

use std::net::SocketAddr;

use axum::Json;
use axum::extract::{ConnectInfo, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;
use serde_json::json;

use super::net::is_lan_peer;
use super::state::Shared;
use super::util::{ct_eq, now_ms, sha256_hex};

pub const DEVICE_COOKIE: &str = "ls_dev";
pub const LOCAL_COOKIE: &str = "ls_local";
/// Browsers cap cookie lifetime at ~400 days; /api/me refreshes it.
const DEVICE_COOKIE_MAX_AGE: i64 = 400 * 24 * 3600;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Listener {
    Lan,
    Local,
}

#[derive(Clone, Debug)]
pub enum Session {
    Host,
    Device { id: String, name: String, can_write: bool },
}

impl Session {
    pub fn is_host(&self) -> bool {
        matches!(self, Session::Host)
    }
    pub fn device_id(&self) -> Option<&str> {
        match self {
            Session::Device { id, .. } => Some(id),
            Session::Host => None,
        }
    }
    pub fn can_write(&self) -> bool {
        match self {
            Session::Host => true,
            Session::Device { can_write, .. } => *can_write,
        }
    }
}

fn deny(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

/// HTTP/2 has no Host header; the authority lives in the URI instead.
pub fn request_host(headers: &HeaderMap, uri: &Uri) -> Option<String> {
    headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
        .or_else(|| uri.authority().map(|a| a.as_str().to_owned()))
}

/// HTTP/2 may split cookies over several headers.
pub fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_owned())
}

pub fn device_cookie_header(value: &str) -> HeaderValue {
    HeaderValue::from_str(&format!(
        "{DEVICE_COOKIE}={value}; Path=/; Max-Age={DEVICE_COOKIE_MAX_AGE}; HttpOnly; Secure; SameSite=Strict"
    ))
    .expect("cookie value is url-safe")
}

pub fn clear_device_cookie_header() -> HeaderValue {
    HeaderValue::from_static("ls_dev=; Path=/; Max-Age=0; HttpOnly; Secure; SameSite=Strict")
}

// ---------------------------------------------------------------- guards

/// Outermost layer of the LAN router.
pub async fn lan_guard(State(state): State<Shared>, ConnectInfo(peer): ConnectInfo<SocketAddr>, mut req: Request, next: Next) -> Response {
    if !is_lan_peer(peer.ip()) {
        tracing::warn!("rejected non-LAN peer {peer}");
        return deny(StatusCode::FORBIDDEN, "只允许局域网内的设备访问");
    }
    if state.is_paused() {
        return deny(StatusCode::SERVICE_UNAVAILABLE, "电脑端已暂停共享");
    }
    req.extensions_mut().insert(Listener::Lan);
    next.run(req).await
}

/// Outermost layer of the loopback router. The Host check defeats DNS
/// rebinding: a web page can resolve its own name to 127.0.0.1 but cannot
/// make the browser send `Host: 127.0.0.1:<port>`.
pub async fn local_guard(State(state): State<Shared>, mut req: Request, next: Next) -> Response {
    let host = request_host(req.headers(), req.uri()).unwrap_or_default();
    let ok = host == format!("127.0.0.1:{}", state.local_port) || host == format!("localhost:{}", state.local_port);
    if !ok {
        return deny(StatusCode::FORBIDDEN, "forbidden");
    }
    req.extensions_mut().insert(Listener::Local);
    next.run(req).await
}

/// Blocks cross-site requests: state-changing calls must carry our custom
/// header (which forces a CORS preflight we never approve) and, when the
/// browser sends an Origin, it must be our own.
pub async fn csrf_guard(req: Request, next: Next) -> Response {
    let method = req.method().clone();
    if method != Method::GET && method != Method::HEAD && method != Method::OPTIONS {
        if req.headers().get("x-lanshare").map(|v| v.as_bytes()) != Some(b"1") {
            return deny(StatusCode::FORBIDDEN, "请求被拒绝（缺少安全标记）");
        }
        if let Some(origin) = req.headers().get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
            let host = request_host(req.headers(), req.uri()).unwrap_or_default();
            let scheme = if req.extensions().get::<Listener>() == Some(&Listener::Local) { "http" } else { "https" };
            if origin != format!("{scheme}://{host}") {
                return deny(StatusCode::FORBIDDEN, "请求被拒绝（来源不匹配）");
            }
        }
    }
    next.run(req).await
}

/// Resolves the caller into a `Session` extension, or answers 401.
pub async fn require_session(State(state): State<Shared>, mut req: Request, next: Next) -> Response {
    let listener = req.extensions().get::<Listener>().copied();
    let session = match listener {
        Some(Listener::Local) => cookie(req.headers(), LOCAL_COOKIE)
            .filter(|t| ct_eq(t.as_bytes(), state.local_token.as_bytes()))
            .map(|_| Session::Host),
        Some(Listener::Lan) => authenticate_device(&state, req.headers()),
        None => None,
    };
    let Some(session) = session else {
        return deny(StatusCode::UNAUTHORIZED, "这台设备还没有配对，或配对已被移除");
    };
    req.extensions_mut().insert(session);
    next.run(req).await
}

fn authenticate_device(state: &Shared, headers: &HeaderMap) -> Option<Session> {
    let value = cookie(headers, DEVICE_COOKIE)?;
    let (id, secret) = value.split_once('.')?;
    let device = state.db.device(id).ok().flatten()?;
    if !ct_eq(sha256_hex(secret).as_bytes(), device.token_hash.as_bytes()) {
        return None;
    }
    let _ = state.db.touch_device(id, now_ms());
    Some(Session::Device { id: device.id, name: device.name, can_write: device.can_write })
}

pub async fn require_host(req: Request, next: Next) -> Response {
    match req.extensions().get::<Session>() {
        Some(Session::Host) => next.run(req).await,
        _ => deny(StatusCode::FORBIDDEN, "只有电脑端可以执行这个操作"),
    }
}

// ---------------------------------------------------------------- local login

#[derive(Deserialize)]
pub struct LocalLogin {
    token: String,
}

/// The desktop window opens `/__local_login?token=…` once at startup.
pub async fn local_login(State(state): State<Shared>, req_listener: Option<axum::Extension<Listener>>, Query(q): Query<LocalLogin>) -> Response {
    if req_listener.map(|e| e.0) != Some(Listener::Local) || !ct_eq(q.token.as_bytes(), state.local_token.as_bytes()) {
        return deny(StatusCode::NOT_FOUND, "not found");
    }
    let mut res = Redirect::to("/").into_response();
    res.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&format!("{LOCAL_COOKIE}={}; Path=/; HttpOnly; SameSite=Strict", state.local_token)).expect("token is url-safe"),
    );
    res
}
