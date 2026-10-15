//! Public pairing endpoints used by a phone that is not paired yet.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderValue, StatusCode, header};
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
