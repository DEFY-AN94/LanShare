//! Sending into a conversation: the HTTP routes and the desktop shell (menu
//! bar, global shortcut) go through the same steps, so events, the 已发送
//! folder, the free-space checks and duplicate detection are the same.

use std::time::Instant;

use axum::body::Body;
use axum::http::StatusCode;
use serde::Deserialize;

use super::auth::Session;
use super::db::NewMessage;
use super::error::{ApiError, ApiResult};
use super::events::Audience;
use super::model::MessageOut;
use super::paths::sanitize_filename;
use super::state::{INBOX_DIR, SENT_DIR, Shared, created_ms};
use super::transfer::{preflight, receive_upload, unless_root_gone, upload_length};
use super::util::now_ms;

pub const MAX_TEXT_CHARS: usize = 100_000;

/// The host names a conversation explicitly; a device always uses its own.
pub(crate) fn conversation(state: &Shared, session: &Session, requested: Option<&str>) -> ApiResult<(String, String)> {
    match session {
        Session::Device { id, name, .. } => Ok((id.clone(), name.clone())),
        Session::Host => {
            let id = requested.filter(|s| !s.is_empty()).ok_or_else(|| ApiError::bad_request("缺少设备参数"))?;
            let device = state.db.device(id)?.ok_or_else(|| ApiError::not_found("设备不存在"))?;
            Ok((device.id, device.name))
        }
    }
}

pub(crate) fn valid_client_id(id: Option<&str>) -> ApiResult<Option<&str>> {
    match id {
        None | Some("") => Ok(None),
        Some(s) if s.len() <= 64 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') => Ok(Some(s)),
        Some(_) => Err(ApiError::bad_request("client_id 无效")),
    }
}

/// A retry of a message deleted meanwhile (it went through, the answer was
/// lost, then one side deleted it) must not bring it back.
fn refuse_if_deleted(state: &Shared, device_id: &str, session: &Session, client_id: Option<&str>) -> ApiResult<()> {
    if let Some(cid) = client_id
        && state.db.was_deleted(device_id, session.is_host(), cid)?
    {
        return Err(ApiError::new(StatusCode::GONE, "这条消息已在另一端删除"));
    }
    Ok(())
}

/// Tells both sides about a new message; a phone's message also counts as
/// unread and raises a notification, unless the window is in front.
fn after_insert(state: &Shared, session: &Session, device_name: &str, out: &MessageOut, is_new: bool) {
    if !is_new {
        return;
    }
    state.emit(Audience::HostAndDevice(out.device_id.clone()), "msg", out);
    if session.is_host() {
        return;
    }
    if state.is_viewing(&out.device_id)
        && let Err(e) = state.db.mark_read(&out.device_id)
    {
        tracing::warn!("could not mark a conversation read: {e}");
    }
    state.unread_changed();
    if !state.bridge.window_focused() {
        let notice = state.notifier.notice(device_name, out, state.notify_hide_text(), Instant::now());
        state.bridge.notify_message(&notice);
    }
}

pub fn send_text(state: &Shared, session: &Session, device: Option<&str>, client_id: Option<&str>, text: &str) -> ApiResult<MessageOut> {
    let (device_id, device_name) = conversation(state, session, device)?;
    if text.trim().is_empty() {
        return Err(ApiError::bad_request("消息不能为空"));
    }
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err(ApiError::bad_request("消息太长了，请改用文件发送"));
    }
    let client_id = valid_client_id(client_id)?;
    refuse_if_deleted(state, &device_id, session, client_id)?;
    let (row, is_new) = state.db.insert_message(&NewMessage {
        device_id: &device_id,
        from_host: session.is_host(),
        client_id,
        kind: "text",
        text: Some(text),
        file_path: None,
        file_name: None,
        file_size: None,
        mime: None,
        created_at: now_ms(),
        file_root: None,
        file_created: None,
    })?;
    let out = state.message_out(row);
    after_insert(state, session, &device_name, &out, is_new);
    Ok(out)
}

#[derive(Deserialize)]
pub struct UploadRequest {
    pub device: Option<String>,
    pub client_id: Option<String>,
    pub name: String,
    /// The file's size as the phone sees it (see `upload_length`).
    pub size: Option<u64>,
}

/// Saves an attachment to 收件箱 (from a phone) or 已发送 (from the computer)
/// and adds its message. `body` is taken only once every check has passed.
pub async fn store_upload(state: &Shared, session: &Session, q: &UploadRequest, declared: Option<u64>, body: &mut Option<Body>) -> ApiResult<MessageOut> {
    let (device_id, device_name) = conversation(state, session, q.device.as_deref())?;
    let client_id = valid_client_id(q.client_id.as_deref())?;
    // A retried upload that already went through returns the original message.
    if let Some(cid) = client_id
        && let Some(existing) = state.db.message_by_client_id(&device_id, session.is_host(), cid)?
    {
        return Ok(state.message_out(existing));
    }
    refuse_if_deleted(state, &device_id, session, client_id)?;
    let name = sanitize_filename(&q.name).ok_or_else(|| ApiError::bad_request("文件名无效"))?;
    let length = upload_length(declared, q.size)?;
    let sandbox = state.usable_sandbox()?;
    let dir = sandbox.ensure_subdir(if session.is_host() { SENT_DIR } else { INBOX_DIR })?;
    let mut reservation = preflight(state, &dir, length)?;
    // Before the first byte lands: these chat folders stay host-only for good.
    state.remember_chat_root(sandbox.root()).map_err(ApiError::internal)?;
    let body = body.take().ok_or_else(|| ApiError::internal("upload body already used"))?;
    let (path, size) = receive_upload(state, session, body, &dir, &name, length, reservation.as_mut()).await?;
    let created = std::fs::metadata(&path).ok().as_ref().and_then(created_ms);
    let rel = sandbox.relative(&path).ok_or_else(|| ApiError::internal("upload landed outside the shared folder"))?;
    let final_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(name);
    let mime = mime_guess::from_path(&final_name).first_or_octet_stream().to_string();
    let root = sandbox.root().to_string_lossy().into_owned();
    let (row, is_new) = state.db.insert_message(&NewMessage {
        device_id: &device_id,
        from_host: session.is_host(),
        client_id,
        kind: "file",
        text: None,
        file_path: Some(&rel),
        file_name: Some(&final_name),
        file_size: Some(size as i64),
        mime: Some(&mime),
        created_at: now_ms(),
        file_root: Some(&root),
        file_created: created,
    })?;
    let out = state.message_out(row);
    state.emit_files_changed(rel.rsplit_once('/').map(|(d, _)| d).unwrap_or(""));
    after_insert(state, session, &device_name, &out, is_new);
    Ok(out)
}

/// Sends bytes the computer already holds (a picture from the clipboard) as
/// an attachment from the host, through the same checks as an upload.
pub async fn send_bytes(state: &Shared, device_id: &str, name: &str, bytes: Vec<u8>) -> ApiResult<MessageOut> {
    let len = bytes.len() as u64;
    let q = UploadRequest { device: Some(device_id.to_string()), client_id: None, name: name.to_string(), size: Some(len) };
    let mut body = Some(Body::from(bytes));
    store_upload(state, &Session::Host, &q, Some(len), &mut body).await.map_err(|e| unless_root_gone(state, e))
}
