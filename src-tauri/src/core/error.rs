//! API errors are returned to the UI as `{"error": "<中文说明>"}`.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use super::paths::{PathError, Unavailable};

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
}

pub type ApiResult<T> = Result<T, ApiError>;

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self { status, message: message.into() }
    }
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }
    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, message)
    }
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, message)
    }
    pub fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "这台设备还没有配对，或配对已被移除")
    }
    pub fn internal(err: impl std::fmt::Display) -> Self {
        tracing::error!("internal error: {err}");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "电脑端出错了，请稍后再试")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError::internal(e)
    }
}

impl From<std::io::Error> for ApiError {
    fn from(e: std::io::Error) -> Self {
        use std::io::ErrorKind::*;
        match e.kind() {
            NotFound => ApiError::not_found("文件或文件夹不存在"),
            PermissionDenied => ApiError::forbidden("电脑上没有权限访问这个位置"),
            AlreadyExists => ApiError::conflict("已存在同名的文件或文件夹"),
            StorageFull => ApiError::new(StatusCode::INSUFFICIENT_STORAGE, "电脑磁盘空间不足"),
            _ => ApiError::internal(e),
        }
    }
}

/// 503: the phone offers 重试, which works once the disk or folder is back.
impl From<Unavailable> for ApiError {
    fn from(u: Unavailable) -> Self {
        ApiError::new(StatusCode::SERVICE_UNAVAILABLE, u.to_string())
    }
}

impl From<PathError> for ApiError {
    fn from(e: PathError) -> Self {
        match e {
            PathError::Invalid => ApiError::bad_request(e.to_string()),
            PathError::NotFound => ApiError::not_found(e.to_string()),
            PathError::Outside => ApiError::forbidden(e.to_string()),
            PathError::Io(io) => io.into(),
        }
    }
}
