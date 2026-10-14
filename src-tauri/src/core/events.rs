//! Server → browser events, fanned out over SSE.

use serde::Serialize;

use super::auth::Session;

#[derive(Clone, Debug)]
pub enum Audience {
    Host,
    Device(String),
    /// A conversation: the host plus one device.
    HostAndDevice(String),
    Everyone,
}

#[derive(Clone, Debug)]
pub struct ServerEvent {
    pub audience: Audience,
    pub name: &'static str,
    pub data: String,
}

impl ServerEvent {
    pub fn new(audience: Audience, name: &'static str, data: &impl Serialize) -> Self {
        Self { audience, name, data: serde_json::to_string(data).unwrap_or_else(|_| "null".into()) }
    }

    pub fn visible_to(&self, s: &Session) -> bool {
        match (&self.audience, s) {
            (Audience::Everyone, _) => true,
            (Audience::Host, Session::Host) => true,
            (Audience::HostAndDevice(_), Session::Host) => true,
            (Audience::Device(id) | Audience::HostAndDevice(id), Session::Device { id: mine, .. }) => id == mine,
            _ => false,
        }
    }

    /// `revoked` closes the target device's stream after delivery.
    pub fn ends_stream_for(&self, s: &Session) -> bool {
        self.name == "revoked" && self.visible_to(s) && !s.is_host()
    }
}
