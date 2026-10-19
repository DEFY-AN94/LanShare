//! `/api/me`, unpairing, and the SSE event stream.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::State;
use axum::http::header;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use futures_util::{Stream, StreamExt};
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;

use crate::core::auth::{DEVICE_COOKIE, Session, clear_device_cookie_header, cookie, device_cookie_header};
use crate::core::error::{ApiError, ApiResult};
use crate::core::events::Audience;
use crate::core::model::Me;
use crate::core::state::{PresenceGuard, Shared};

pub async fn me(State(state): State<Shared>, Extension(session): Extension<Session>, headers: axum::http::HeaderMap) -> Response {
    let me = match &session {
        Session::Host => Me {
            kind: "host",
            device_id: None,
            device_name: None,
            host_name: state.host_name.clone(),
            can_write: true,
            paused: state.is_paused(),
        },
        Session::Device { id, name, can_write } => Me {
            kind: "device",
            device_id: Some(id.clone()),
            device_name: Some(name.clone()),
            host_name: state.host_name.clone(),
            can_write: *can_write,
            paused: state.is_paused(),
        },
    };
    let mut res = Json(me).into_response();
    // Re-issue the device cookie so an often-used phone never hits the expiry.
    if !session.is_host()
        && let Some(value) = cookie(&headers, DEVICE_COOKIE)
    {
        res.headers_mut().insert(header::SET_COOKIE, device_cookie_header(&value));
    }
    res
}

pub async fn unpair(State(state): State<Shared>, Extension(session): Extension<Session>) -> ApiResult<Response> {
    let Some(id) = session.device_id() else {
        return Err(ApiError::bad_request("电脑端不能解除自己的配对"));
    };
    state.db.delete_device(id)?;
    state.revoke(id);
    state.emit(Audience::Host, "devices", &json!({}));
    let mut res = Json(json!({ "ok": true })).into_response();
    res.headers_mut().insert(header::SET_COOKIE, clear_device_cookie_header());
    Ok(res)
}

pub async fn events(State(state): State<Shared>, Extension(session): Extension<Session>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.events.subscribe();
    let state_pruned = state.pruned_before();
    let guard = PresenceGuard::new(state.clone(), session.device_id().map(str::to_owned));
    let init = (rx, guard, session, state, false);
    let stream = futures_util::stream::unfold(init, |(mut rx, guard, session, state, done)| async move {
        if done {
            return None;
        }
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    // Pausing cuts every phone off immediately; the host keeps its stream.
                    if state.is_paused() && !session.is_host() && ev.name != "paused" {
                        return None;
                    }
                    if !ev.visible_to(&session) {
                        continue;
                    }
                    let end = ev.ends_stream_for(&session) || (ev.name == "paused" && state.is_paused() && !session.is_host());
                    let event = Event::default().event(ev.name).data(ev.data);
                    return Some((Ok(event), (rx, guard, session, state, end)));
                }
                Err(RecvError::Lagged(_)) => {
                    let event = Event::default().event("resync").data("{}");
                    return Some((Ok(event), (rx, guard, session, state, false)));
                }
                Err(RecvError::Closed) => return None,
            }
        }
    });
    // Send a first byte right away so the browser reports the stream as open
    // without waiting for the first real event or keep-alive.
    let hello = futures_util::stream::once(async { Ok(Event::default().comment("connected")) });
    // A client that was offline during a cleanup or clear drops those messages now.
    let pruned = Some(state_pruned).filter(|&before| before > 0).map(|before| Ok(Event::default().event("pruned").data(json!({ "before": before }).to_string())));
    Sse::new(hello.chain(futures_util::stream::iter(pruned)).chain(stream)).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)).text("ping"))
}
