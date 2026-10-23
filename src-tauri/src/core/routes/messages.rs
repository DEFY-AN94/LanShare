//! Chat messages: one conversation per paired device.

use axum::body::Body;
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::{Extension, Json};
use serde::Deserialize;

use crate::core::auth::Session;
use crate::core::bridge::Withdraw;
use crate::core::chat::{self, UploadRequest};
use crate::core::error::{ApiError, ApiResult};
use crate::core::events::Audience;
use crate::core::model::{MessageOut, MessagePage};
use crate::core::state::Shared;
use crate::core::transfer::{REJECTED_UPLOAD_DRAIN, declared_len, discard_body, serve_path, unless_root_gone};

#[derive(Deserialize)]
pub struct ListQuery {
    device: Option<String>,
    before: Option<i64>,
    after: Option<i64>,
    limit: Option<usize>,
}

pub async fn list(State(state): State<Shared>, Extension(session): Extension<Session>, Query(q): Query<ListQuery>) -> ApiResult<Json<MessagePage>> {
    let (device_id, _) = chat::conversation(&state, &session, q.device.as_deref())?;
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

pub async fn send(State(state): State<Shared>, Extension(session): Extension<Session>, Json(body): Json<SendBody>) -> ApiResult<Json<MessageOut>> {
    chat::send_text(&state, &session, body.device.as_deref(), body.client_id.as_deref(), &body.text).map(Json)
}

pub async fn upload(
    State(state): State<Shared>,
    Extension(session): Extension<Session>,
    Query(q): Query<UploadRequest>,
    headers: HeaderMap,
    body: Body,
) -> ApiResult<Json<MessageOut>> {
    let mut body = Some(body);
    let result = chat::store_upload(&state, &session, &q, declared_len(&headers), &mut body).await.map_err(|e| unless_root_gone(&state, e));
    // Left unread by a retry that already went through, or by a rejection.
    if let Some(rest) = body {
        discard_body(&headers, rest, if result.is_ok() { u64::MAX } else { REJECTED_UPLOAD_DRAIN }).await;
    }
    result.map(Json)
}

#[derive(Deserialize)]
pub struct DeleteQuery {
    /// 1: also move the attachment to the Trash / Recycle Bin (it is kept by default).
    trash_file: Option<u8>,
}

/// Deletes one message from a conversation the caller belongs to.
pub async fn delete(State(state): State<Shared>, Extension(session): Extension<Session>, Path(id): Path<i64>, Query(q): Query<DeleteQuery>) -> ApiResult<StatusCode> {
    let msg = state.db.message(id)?.ok_or_else(|| ApiError::not_found("消息不存在"))?;
    if let Some(mine) = session.device_id()
        && mine != msg.device_id
    {
        return Err(ApiError::not_found("消息不存在"));
    }
    if q.trash_file == Some(1)
        && let Some(rel) = msg.file_path.as_deref()
    {
        if !session.can_write() {
            return Err(ApiError::forbidden("这台设备只有读取权限，不能删除电脑上的文件"));
        }
        let check = state.path_check().await;
        if let Some(path) = state.attachment_path(&msg) {
            super::files::trash(path, check).await?;
            state.emit_files_changed(rel.rsplit_once('/').map(|(d, _)| d).unwrap_or(""));
        }
    }
    state.db.delete_message(id)?;
    state.emit(Audience::HostAndDevice(msg.device_id.clone()), "msg_deleted", &serde_json::json!({ "ids": [id] }));
    state.emit(Audience::Host, "devices", &serde_json::json!({}));
    state.bridge.withdraw_notifications(&Withdraw::Ids(state.notifier.for_deleted(&msg.device_id, &[id])));
    state.unread_changed();
    Ok(StatusCode::NO_CONTENT)
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
    let rel = msg.file_path.as_deref().ok_or_else(|| ApiError::not_found("这条消息没有文件"))?;
    // Until serve_path has opened the file (see AppState::path_check).
    let _check = state.path_check().await;
    let path = state.attachment_path(&msg).ok_or_else(|| ApiError::not_found("文件已被移动或删除"))?;
    let name = msg.file_name.clone().unwrap_or_else(|| rel.rsplit('/').next().unwrap_or("file").to_string());
    let (parts, _) = req.into_parts();
    Ok(serve_path(&state, &session, &path, &name, q.inline == Some(1), &parts).await)
}
