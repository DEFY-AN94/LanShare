//! `/api/me`, unpairing, and the SSE event stream.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::State;
use axum::http::header;
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use futures_util::Stream;
use serde_json::json;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;

use crate::core::auth::{DEVICE_COOKIE, Session, clear_device_cookie_header, cookie, device_cookie_header};
use crate::core::error::{ApiError, ApiResult};
use crate::core::events::{Audience, ServerEvent};
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
    state.device_removed(id);
    state.emit(Audience::Host, "devices", &json!({}));
    let mut res = Json(json!({ "ok": true })).into_response();
    res.headers_mut().insert(header::SET_COOKIE, clear_device_cookie_header());
    Ok(res)
}

/// How often an open event stream says it is still alive.
const PING_EVERY: Duration = Duration::from_secs(15);

pub async fn events(State(state): State<Shared>, Extension(session): Extension<Session>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.events.subscribe();
    // Send a first byte right away so the browser reports the stream as open
    // without waiting for the first real event or keep-alive.
    let mut first = vec![Event::default().comment("connected")];
    // A client that was offline during a cleanup, a clear or a delete drops those messages now.
    let (pruned, cleared) = (state.pruned_before(), state.cleared_through());
    if pruned > 0 || cleared > 0 {
        first.push(Event::default().event("pruned").data(json!({ "before": pruned, "through": cleared }).to_string()));
    }
    let deleted = state.deleted_visible_to(&session);
    if !deleted.is_empty() {
        first.push(Event::default().event("msg_deleted").data(json!({ "ids": deleted }).to_string()));
    }
    let feed = Feed {
        rx,
        _presence: PresenceGuard::new(state.clone(), session.device_id().map(str::to_owned)),
        session,
        state,
        // A named event, not an SSE comment: the page cannot see comments, and
        // it needs some sign of life to notice a stream iOS left dead.
        ping: tokio::time::interval_at(tokio::time::Instant::now() + PING_EVERY, PING_EVERY),
        first: first.into_iter(),
        done: false,
    };
    Sse::new(futures_util::stream::unfold(feed, |mut feed| async move { Some((Ok(feed.next().await?), feed)) }))
}

/// One open event stream.
struct Feed {
    rx: broadcast::Receiver<ServerEvent>,
    _presence: PresenceGuard,
    session: Session,
    state: Shared,
    ping: tokio::time::Interval,
    /// Sent before anything else.
    first: std::vec::IntoIter<Event>,
    done: bool,
}

impl Feed {
    async fn next(&mut self) -> Option<Event> {
        if self.done {
            return None;
        }
        let (event, end) = match self.first.next() {
            Some(event) => (event, false),
            None => self.receive().await?,
        };
        // Checked before every event, so a phone removed or paused hears
        // that and nothing else, also if it missed the event that said so.
        // The host keeps its stream.
        if let Some(last) = self.cut_off() {
            self.done = true;
            return Some(last);
        }
        self.done = end;
        Some(event)
    }

    /// The next event for this stream, and whether it ends the stream.
    async fn receive(&mut self) -> Option<(Event, bool)> {
        loop {
            let received = tokio::select! {
                r = self.rx.recv() => r,
                _ = self.ping.tick() => return Some((Event::default().event("ping").data("{}"), false)),
            };
            match received {
                Ok(ev) if ev.visible_to(&self.session) => {
                    let end = ev.ends_stream_for(&self.session);
                    return Some((Event::default().event(ev.name).data(ev.data), end));
                }
                Ok(_) => continue,
                Err(RecvError::Lagged(_)) => return Some((Event::default().event("resync").data("{}"), false)),
                Err(RecvError::Closed) => return None,
            }
        }
    }

    /// The last event of a phone's stream once it was removed or sharing is paused.
    fn cut_off(&self) -> Option<Event> {
        let id = self.session.device_id()?;
        if self.state.is_revoked(id) {
            Some(Event::default().event("revoked").data("{}"))
        } else if self.state.is_paused() {
            Some(Event::default().event("paused").data(json!({ "paused": true }).to_string()))
        } else {
            None
        }
    }
}
