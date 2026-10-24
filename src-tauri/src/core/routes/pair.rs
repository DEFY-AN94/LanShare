//! Public pairing endpoints used by a phone that is not paired yet.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::extract::{ConnectInfo, Path, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::core::auth::{Listener, device_cookie_header};
use crate::core::error::{ApiError, ApiResult};
use crate::core::events::Audience;
use crate::core::pairing::Poll;
use crate::core::state::Shared;
use crate::core::util::now_ms;

/// A pairing request is a code or token and a device name.
const PAIR_BODY_MAX: usize = 4 * 1024;
const PAIR_BODY_WAIT: Duration = Duration::from_secs(10);
const PAIR_READS_PER_IP: usize = 4;
const PAIR_READS_TOTAL: usize = 32;

/// Pairing requests are the only bodies read from devices that are not
/// paired: each is held to `PAIR_BODY_MAX` and `PAIR_BODY_WAIT`, and only a
/// few are read at a time, before the handler and its rate limit see them.
#[derive(Clone, Default)]
pub struct PairLimits {
    reading: Arc<Mutex<Reading>>,
}

#[derive(Default)]
struct Reading {
    total: usize,
    per_ip: HashMap<IpAddr, usize>,
}

/// One request being read; frees its place when dropped.
struct ReadPermit {
    reading: Arc<Mutex<Reading>>,
    ip: IpAddr,
}

impl Drop for ReadPermit {
    fn drop(&mut self) {
        let mut r = self.reading.lock().unwrap_or_else(|p| p.into_inner());
        r.total -= 1;
        if let Some(n) = r.per_ip.get_mut(&self.ip) {
            *n -= 1;
            if *n == 0 {
                r.per_ip.remove(&self.ip);
            }
        }
    }
}

impl PairLimits {
    fn admit(&self, ip: IpAddr) -> Option<ReadPermit> {
        let ip = ip.to_canonical();
        let mut r = self.reading.lock().unwrap_or_else(|p| p.into_inner());
        let n = r.per_ip.get(&ip).copied().unwrap_or(0);
        if r.total >= PAIR_READS_TOTAL || n >= PAIR_READS_PER_IP {
            return None;
        }
        r.total += 1;
        r.per_ip.insert(ip, n + 1);
        Some(ReadPermit { reading: self.reading.clone(), ip })
    }
}

/// Middleware for `POST /api/pair/request`.
pub async fn limit_request(State(limits): State<PairLimits>, ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request, next: Next) -> Response {
    let Some(_permit) = limits.admit(peer.ip()) else {
        return ApiError::new(StatusCode::TOO_MANY_REQUESTS, "连接请求太多，请稍后再试").into_response();
    };
    let (parts, body) = req.into_parts();
    let bytes = match tokio::time::timeout(PAIR_BODY_WAIT, to_bytes(body, PAIR_BODY_MAX)).await {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(_)) => return ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "连接请求无效，请重新扫码").into_response(),
        Err(_) => return ApiError::new(StatusCode::REQUEST_TIMEOUT, "连接请求超时，请重试").into_response(),
    };
    next.run(Request::from_parts(parts, Body::from(bytes))).await
}

#[derive(Deserialize)]
pub struct PairBody {
    token: Option<String>,
    code: Option<String>,
    name: Option<String>,
}

fn clean_device_name(raw: Option<&str>) -> String {
    let name: String = raw.unwrap_or("").chars().filter(|c| !c.is_control()).take(40).collect();
    let name = name.trim();
    if name.is_empty() { "新设备".to_string() } else { name.to_string() }
}

pub async fn request(
    State(state): State<Shared>,
    Extension(listener): Extension<Listener>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(body): Json<PairBody>,
) -> ApiResult<Response> {
    if listener != Listener::Lan {
        return Err(ApiError::not_found("not found"));
    }
    let name = clean_device_name(body.name.as_deref());
    let req = state
        .pairing
        .lock()
        .unwrap()
        .request(body.token.as_deref(), body.code.as_deref(), &name, peer.ip().to_canonical(), now_ms())
        .map_err(|e| ApiError::new(StatusCode::FORBIDDEN, e.message()))?;
    state.emit(Audience::Host, "pairing", &req.to_out());
    state.bridge.show_window();
    state.bridge.notify("新设备请求连接", &format!("「{}」请求连接这台电脑，确认码 {}", req.name, req.confirm_code));
    Ok(Json(json!({ "request_id": req.id, "confirm_code": req.confirm_code, "host_name": state.host_name })).into_response())
}

pub async fn status(State(state): State<Shared>, Extension(listener): Extension<Listener>, Path(id): Path<String>) -> ApiResult<Response> {
    if listener != Listener::Lan {
        return Err(ApiError::not_found("not found"));
    }
    let poll = state.pairing.lock().unwrap().poll(&id, now_ms());
    let (status, cookie) = match poll {
        Poll::Pending => ("pending", None),
        Poll::Approved { cookie } => ("approved", Some(cookie)),
        Poll::Rejected => ("rejected", None),
        Poll::Expired => ("expired", None),
    };
    let mut res = Json(json!({ "status": status })).into_response();
    if let Some(c) = cookie {
        res.headers_mut().insert(header::SET_COOKIE, device_cookie_header(&c));
    }
    Ok(res)
}

/// Name of the current CA, so the iPhone guide can say which entry to trust.
/// Public like the certificate itself (which contains the name).
pub async fn ca_info(State(state): State<Shared>) -> Json<Value> {
    Json(json!({ "name": state.tls.ca_name() }))
}

/// The CA certificate is public; serving it lets iPhones install and trust it.
pub async fn ca_cert(State(state): State<Shared>) -> Response {
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("application/x-x509-ca-cert")),
            (header::CONTENT_DISPOSITION, HeaderValue::from_static("attachment; filename=\"LanShare-CA.crt\"")),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        state.tls.ca_pem(),
    )
        .into_response()
}
