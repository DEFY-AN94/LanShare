//! Chat messages: one conversation per paired device.

use axum::body::Body;
use axum::extract::{Path, Query, Request, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::{Extension, Json};
use serde::Deserialize;

use crate::core::auth::Session;
use crate::core::db::NewMessage;
use crate::core::error::{ApiError, ApiResult};
use crate::core::events::Audience;
use crate::core::model::{MessageOut, MessagePage};
use crate::core::paths::sanitize_filename;
use crate::core::state::{INBOX_DIR, SENT_DIR, Shared};
use crate::core::transfer::{REJECTED_UPLOAD_DRAIN, discard_body, receive_upload, serve_path};
use crate::core::util::{now_ms, preview};

const MAX_TEXT_CHARS: usize = 100_000;

/// The host names a conversation explicitly; a device always uses its own.
fn conversation(state: &Shared, session: &Session, requested: Option<&str>) -> ApiResult<(String, String)> {
    match session {
        Session::Device { id, name, .. } => Ok((id.clone(), name.clone())),
        Session::Host => {
            let id = requested.filter(|s| !s.is_empty()).ok_or_else(|| ApiError::bad_request("缺少设备参数"))?;
            let device = state.db.device(id)?.ok_or_else(|| ApiError::not_found("设备不存在"))?;
            Ok((device.id, device.name))
        }
    }
}

fn valid_client_id(id: Option<&str>) -> ApiResult<Option<&str>> {
    match id {
        None | Some("") => Ok(None),
        Some(s) if s.len() <= 64 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') => Ok(Some(s)),
        Some(_) => Err(ApiError::bad_request("client_id 无效")),
    }
}

#[derive(Deserialize)]
pub struct ListQuery {
    device: Option<String>,
    before: Option<i64>,
    after: Option<i64>,
    limit: Option<usize>,
}

pub async fn list(State(state): State<Shared>, Extension(session): Extension<Session>, Query(q): Query<ListQuery>) -> ApiResult<Json<MessagePage>> {
    let (device_id, _) = conversation(&state, &session, q.device.as_deref())?;
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let (rows, has_more) = state.db.messages(&device_id, q.before, q.after, limit)?;
    Ok(Json(MessagePage { messages: rows.into_iter().map(|m| state.message_out(m)).collect(), has_more }))
}

#[derive(Deserialize)]
pub struct SendBody {
    device: Option<String>,
    client_id: Option<String>,
    text: String,
}

fn after_insert(state: &Shared, session: &Session, device_name: &str, out: &MessageOut, is_new: bool) {
    if !is_new {
        return;
    }
    state.emit(Audience::HostAndDevice(out.device_id.clone()), "msg", out);
    if !session.is_host() {
        let body = match (&out.text, &out.file) {
            (Some(t), _) => preview(t, 80),
            (_, Some(f)) => format!("[文件] {}", f.name),
            _ => String::new(),
        };
        state.bridge.notify(device_name, &body);
    }
}

pub async fn send(State(state): State<Shared>, Extension(session): Extension<Session>, Json(body): Json<SendBody>) -> ApiResult<Json<MessageOut>> {
    let (device_id, device_name) = conversation(&state, &session, body.device.as_deref())?;
    if body.text.trim().is_empty() {
        return Err(ApiError::bad_request("消息不能为空"));
    }
    if body.text.chars().count() > MAX_TEXT_CHARS {
        return Err(ApiError::bad_request("消息太长了，请改用文件发送"));
    }
    let client_id = valid_client_id(body.client_id.as_deref())?;
    let (row, is_new) = state.db.insert_message(&NewMessage {
        device_id: &device_id,
        from_host: session.is_host(),
        client_id,
        kind: "text",
        text: Some(&body.text),
        file_path: None,
        file_name: None,
        file_size: None,
        mime: None,
        created_at: now_ms(),
        file_root: None,
    })?;
    let out = state.message_out(row);
    after_insert(&state, &session, &device_name, &out, is_new);
    Ok(Json(out))
}

#[derive(Deserialize)]
pub struct UploadQuery {
    device: Option<String>,
    client_id: Option<String>,
    name: String,
}

pub async fn upload(
    State(state): State<Shared>,
    Extension(session): Extension<Session>,
    Query(q): Query<UploadQuery>,
    headers: HeaderMap,
    body: Body,
) -> ApiResult<Json<MessageOut>> {
    let mut body = Some(body);
    let result = store_upload(&state, &session, &q, &mut body).await;
    // Left unread by a retry that already went through, or by a rejection.
    if let Some(rest) = body {
        discard_body(&headers, rest, if result.is_ok() { u64::MAX } else { REJECTED_UPLOAD_DRAIN }).await;
    }
    result.map(Json)
}

async fn store_upload(state: &Shared, session: &Session, q: &UploadQuery, body: &mut Option<Body>) -> ApiResult<MessageOut> {
    let (device_id, device_name) = conversation(state, session, q.device.as_deref())?;
    let client_id = valid_client_id(q.client_id.as_deref())?;
    // A retried upload that already went through returns the original message.
    if let Some(cid) = client_id
        && let Some(existing) = state.db.message_by_client_id(&device_id, session.is_host(), cid)?
    {
        return Ok(state.message_out(existing));
    }
    let name = sanitize_filename(&q.name).ok_or_else(|| ApiError::bad_request("文件名无效"))?;
    let sandbox = state.sandbox();
    let dir = sandbox.ensure_subdir(if session.is_host() { SENT_DIR } else { INBOX_DIR })?;
    let body = body.take().ok_or_else(|| ApiError::internal("upload body already used"))?;
    let (path, size) = receive_upload(state, session, body, &dir, &name).await?;
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
    })?;
    let out = state.message_out(row);
    state.emit_files_changed(rel.rsplit_once('/').map(|(d, _)| d).unwrap_or(""));
    after_insert(state, session, &device_name, &out, is_new);
    Ok(out)
}

#[derive(Deserialize)]
pub struct FileQuery {
    inline: Option<u8>,
}

pub async fn file(
    State(state): State<Shared>,
    Extension(session): Extension<Session>,
    Path(id): Path<i64>,
    Query(q): Query<FileQuery>,
    req: Request,
) -> ApiResult<Response> {
    let msg = state.db.message(id)?.ok_or_else(|| ApiError::not_found("消息不存在"))?;
    if let Some(mine) = session.device_id()
        && mine != msg.device_id
    {
        return Err(ApiError::not_found("消息不存在"));
    }
    let rel = msg.file_path.ok_or_else(|| ApiError::not_found("这条消息没有文件"))?;
    let path = state.attachment_path(msg.file_root.as_deref(), &rel).ok_or_else(|| ApiError::not_found("文件已被移动或删除"))?;
    let name = msg.file_name.unwrap_or_else(|| rel.rsplit('/').next().unwrap_or("file").to_string());
    let (parts, _) = req.into_parts();
    Ok(serve_path(&state, &session, &path, &name, q.inline == Some(1), &parts).await)
}
