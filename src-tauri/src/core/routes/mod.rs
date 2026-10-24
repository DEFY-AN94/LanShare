//! HTTP routing for both listeners.

mod files;
mod host;
mod messages;
mod pair;
mod session;
mod static_files;

use std::net::SocketAddr;

use axum::Router;
use axum::extract::{ConnectInfo, Request};
use axum::http::{HeaderValue, header};
use axum::middleware::{Next, from_fn, from_fn_with_state};
use axum::response::Response;
use axum::routing::{delete, get, patch, post};
use tower_http::set_header::SetResponseHeaderLayer;

use super::auth::{csrf_guard, lan_guard, local_guard, local_login, require_host, require_session};
use super::state::Shared;

fn api(state: &Shared) -> Router<Shared> {
    let host_only = Router::new()
        .route("/api/host/overview", get(host::overview))
        .route("/api/host/pairing", post(host::start_pairing).delete(host::cancel_pairing))
        .route("/api/host/pairing/requests", get(host::pairing_requests))
        .route("/api/host/pairing/requests/{id}/approve", post(host::approve))
        .route("/api/host/pairing/requests/{id}/reject", post(host::reject))
        .route("/api/host/devices", get(host::devices))
        .route("/api/host/devices/{id}", patch(host::update_device).delete(host::delete_device))
        .route("/api/host/paused", post(host::set_paused))
        .route("/api/host/open", post(host::open))
        .route("/api/host/reveal", post(host::reveal))
        .route("/api/host/open-url", post(host::open_url))
        .route("/api/host/shared-root/pick", post(host::pick_shared_root))
        .route("/api/host/reset-cert", post(host::reset_cert))
        .route("/api/host/clear-messages", post(host::clear_messages))
        .route("/api/host/chat-retention", post(host::set_chat_retention))
        .route("/api/host/logs/open", post(host::open_logs))
        .route("/api/host/login-item", get(host::login_item).post(host::set_login_item))
        .route("/api/host/login-item/suggested", post(host::dismiss_login_item_suggestion))
        .route("/api/host/login-item/settings", post(host::open_login_item_settings))
        .route("/api/host/logs/verbose", post(host::set_verbose_logging))
        .route("/api/host/viewing", post(host::set_viewing))
        .route("/api/host/notifications", post(host::set_notifications))
        .route("/api/host/clipboard/shortcut", post(host::set_clipboard_shortcut))
        .route("/api/host/clipboard/shortcut/recording", post(host::set_shortcut_recording))
        .route("/api/host/clipboard/target", post(host::set_clipboard_target))
        .route("/api/host/messages/{id}/open", post(host::open_message_file))
        .route("/api/host/messages/{id}/reveal", post(host::reveal_message_file))
        .route_layer(from_fn(require_host));

    let authed = Router::new()
        .route("/api/me", get(session::me))
        .route("/api/me/unpair", post(session::unpair))
        .route("/api/events", get(session::events))
        .route("/api/messages", get(messages::list).post(messages::send))
        .route("/api/messages/upload", post(messages::upload))
        .route("/api/messages/{id}", delete(messages::delete))
        .route("/api/messages/{id}/file", get(messages::file))
        .route("/api/files", get(files::list))
        .route("/api/files/download", get(files::download))
        .route("/api/files/upload", post(files::upload))
        .route("/api/upload-check", get(files::upload_check))
        .route("/api/files/mkdir", post(files::mkdir))
        .route("/api/files/rename", post(files::rename))
        .route("/api/files/delete", post(files::delete))
        .merge(host_only)
        .route_layer(from_fn_with_state(state.clone(), require_session));

    Router::new()
        .route("/api/pair/request", post(pair::request).layer(from_fn_with_state(pair::PairLimits::default(), pair::limit_request)))
        .route("/api/pair/status/{id}", get(pair::status))
        .route("/api/ca", get(pair::ca_info))
        .route("/api/ca.crt", get(pair::ca_cert))
        .route("/__local_login", get(local_login))
        .merge(authed)
        .fallback(static_files::handler)
        .layer(SetResponseHeaderLayer::if_not_present(header::CACHE_CONTROL, HeaderValue::from_static("no-store")))
        .layer(SetResponseHeaderLayer::if_not_present(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")))
        .layer(SetResponseHeaderLayer::overriding(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer")))
        .layer(SetResponseHeaderLayer::if_not_present(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY")))
        .layer(from_fn(csrf_guard))
}

/// HTTPS router for phones on the LAN.
pub fn lan_router(state: Shared) -> Router {
    api(&state).layer(from_fn_with_state(state.clone(), lan_guard)).layer(from_fn(access_log)).with_state(state)
}

/// Debug log of page and asset requests from phones (API calls are left out:
/// their paths carry file names).
async fn access_log(ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request, next: Next) -> Response {
    let (method, path) = (req.method().clone(), req.uri().path().to_owned());
    let ua = req.headers().get(header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or("-").to_owned();
    let res = next.run(req).await;
    if !path.starts_with("/api/") {
        tracing::debug!("{} {method} {path} -> {} ua={ua}", peer.ip().to_canonical(), res.status().as_u16());
    }
    res
}

/// Plain-HTTP router for the desktop window on 127.0.0.1.
pub fn local_router(state: Shared) -> Router {
    api(&state).layer(from_fn_with_state(state.clone(), local_guard)).with_state(state)
}
