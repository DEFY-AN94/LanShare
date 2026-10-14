//! Browsing and managing the shared folder.

use std::path::Path as FsPath;
use std::time::UNIX_EPOCH;

use axum::body::Body;
use axum::extract::{Query, Request, State};
use axum::response::Response;
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::core::auth::Session;
use crate::core::error::{ApiError, ApiResult};
use crate::core::model::{DirListing, FileEntry};
use crate::core::paths::{Sandbox, sanitize_filename};
use crate::core::state::Shared;
use crate::core::transfer::{receive_upload, serve_path};

fn require_write(session: &Session) -> ApiResult<()> {
    if session.can_write() { Ok(()) } else { Err(ApiError::forbidden("这台设备只有读取权限")) }
}

/// Names never shown to clients.
fn is_hidden(name: &str, meta: &std::fs::Metadata) -> bool {
    if name.starts_with('.') || name.starts_with('~') && name.contains('$') {
        return true;
    }
    let lower = name.to_ascii_lowercase();
    if matches!(lower.as_str(), "desktop.ini" | "thumbs.db" | "$recycle.bin" | "system volume information" | "icon\r") {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const HIDDEN: u32 = 0x2;
        const SYSTEM: u32 = 0x4;
        if meta.file_attributes() & (HIDDEN | SYSTEM) != 0 {
            return true;
        }
    }
    let _ = meta;
    false
}

fn modified_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64).unwrap_or(0)
}

pub fn entry_for(sandbox: &Sandbox, path: &FsPath) -> Option<FileEntry> {
    let meta = std::fs::metadata(path).ok()?;
    Some(FileEntry {
        name: path.file_name()?.to_string_lossy().into_owned(),
        path: sandbox.relative(path)?,
        is_dir: meta.is_dir(),
        size: if meta.is_dir() { 0 } else { meta.len() },
        modified: modified_ms(&meta),
    })
}

fn list_dir(sandbox: &Sandbox, dir: &FsPath) -> std::io::Result<Vec<FileEntry>> {
    let mut entries = Vec::new();
    for item in std::fs::read_dir(dir)? {
        let Ok(item) = item else { continue };
        let name = item.file_name().to_string_lossy().into_owned();
        let Ok(link_meta) = item.metadata() else { continue };
        if is_hidden(&name, &link_meta) {
            continue;
        }
        let mut path = item.path();
        if link_meta.file_type().is_symlink() {
            // Only list links that stay inside the shared folder.
            match dunce::canonicalize(&path) {
                Ok(target) if target.starts_with(sandbox.root()) => path = item.path(),
                _ => continue,
            }
        }
        let Ok(meta) = std::fs::metadata(&path) else { continue };
        let Some(rel) = sandbox.relative(&item.path()) else { continue };
        entries.push(FileEntry {
            name,
            path: rel,
            is_dir: meta.is_dir(),
            size: if meta.is_dir() { 0 } else { meta.len() },
            modified: modified_ms(&meta),
        });
    }
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(entries)
}

#[derive(Deserialize)]
pub struct PathQuery {
    #[serde(default)]
    path: String,
    inline: Option<u8>,
}

pub async fn list(State(state): State<Shared>, Extension(session): Extension<Session>, Query(q): Query<PathQuery>) -> ApiResult<Json<DirListing>> {
    let sandbox = state.sandbox();
    let dir = sandbox.resolve_dir(&q.path)?;
    let rel = sandbox.relative(&dir).unwrap_or_default();
    let sb = sandbox.clone();
    let entries = tokio::task::spawn_blocking(move || list_dir(&sb, &dir)).await.map_err(ApiError::internal)??;
    Ok(Json(DirListing { path: rel, entries, can_write: session.can_write() }))
}

pub async fn download(State(state): State<Shared>, Query(q): Query<PathQuery>, req: Request) -> ApiResult<Response> {
    let path = state.sandbox().resolve_file(&q.path)?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
    let (parts, _) = req.into_parts();
    Ok(serve_path(&path, &name, q.inline == Some(1), &parts).await)
}

#[derive(Deserialize)]
pub struct UploadQuery {
    #[serde(default)]
    dir: String,
    name: String,
}

pub async fn upload(
    State(state): State<Shared>,
    Extension(session): Extension<Session>,
    Query(q): Query<UploadQuery>,
    body: Body,
) -> ApiResult<Json<FileEntry>> {
    require_write(&session)?;
    let sandbox = state.sandbox();
    let dir = sandbox.resolve_dir(&q.dir)?;
    let name = sanitize_filename(&q.name).ok_or_else(|| ApiError::bad_request("文件名无效"))?;
    let (path, _) = receive_upload(&state, body, &dir, &name).await?;
    let entry = entry_for(&sandbox, &path).ok_or_else(|| ApiError::internal("uploaded file vanished"))?;
    state.emit_files_changed(&sandbox.relative(&dir).unwrap_or_default());
    Ok(Json(entry))
}

#[derive(Deserialize)]
pub struct MkdirBody {
    #[serde(default)]
    dir: String,
    name: String,
}

pub async fn mkdir(State(state): State<Shared>, Extension(session): Extension<Session>, Json(b): Json<MkdirBody>) -> ApiResult<Json<FileEntry>> {
    require_write(&session)?;
    let sandbox = state.sandbox();
    let dir = sandbox.resolve_dir(&b.dir)?;
    let name = sanitize_filename(&b.name).ok_or_else(|| ApiError::bad_request("名称无效"))?;
    let target = dir.join(&name);
    if target.exists() {
        return Err(ApiError::conflict("已存在同名的文件或文件夹"));
    }
    std::fs::create_dir(&target)?;
    state.emit_files_changed(&sandbox.relative(&dir).unwrap_or_default());
    Ok(Json(entry_for(&sandbox, &target).ok_or_else(|| ApiError::internal("folder vanished"))?))
}

#[derive(Deserialize)]
pub struct RenameBody {
    path: String,
    name: String,
}

pub async fn rename(State(state): State<Shared>, Extension(session): Extension<Session>, Json(b): Json<RenameBody>) -> ApiResult<Json<FileEntry>> {
    require_write(&session)?;
    let sandbox = state.sandbox();
    let src = sandbox.resolve_existing(&b.path)?;
    if src == sandbox.root() {
        return Err(ApiError::bad_request("不能重命名共享文件夹本身"));
    }
    let parent = src.parent().ok_or_else(|| ApiError::bad_request("路径无效"))?.to_path_buf();
    let name = sanitize_filename(&b.name).ok_or_else(|| ApiError::bad_request("名称无效"))?;
    let dst = parent.join(&name);
    // Allow case-only renames on case-insensitive disks (same file).
    let same_file = dunce::canonicalize(&dst).ok().as_deref() == Some(src.as_path());
    if dst.exists() && !same_file {
        return Err(ApiError::conflict("已存在同名的文件或文件夹"));
    }
    std::fs::rename(&src, &dst)?;
    state.emit_files_changed(&sandbox.relative(&parent).unwrap_or_default());
    Ok(Json(entry_for(&sandbox, &dst).ok_or_else(|| ApiError::internal("renamed item vanished"))?))
}

#[derive(Deserialize)]
pub struct DeleteBody {
    path: String,
}

pub fn move_to_trash(path: &FsPath) -> Result<(), trash::Error> {
    #[allow(unused_mut)]
    let mut ctx = trash::TrashContext::default();
    #[cfg(target_os = "macos")]
    {
        // The default (Finder via AppleScript) asks for Automation permission.
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        ctx.set_delete_method(DeleteMethod::NsFileManager);
    }
    ctx.delete(path)
}

pub async fn delete(State(state): State<Shared>, Extension(session): Extension<Session>, Json(b): Json<DeleteBody>) -> ApiResult<Json<Value>> {
    require_write(&session)?;
    let sandbox = state.sandbox();
    let target = sandbox.resolve_existing(&b.path)?;
    if target == sandbox.root() {
        return Err(ApiError::bad_request("不能删除共享文件夹本身"));
    }
    let parent_rel = target.parent().and_then(|p| sandbox.relative(p)).unwrap_or_default();
    // Trash APIs block (and need their own COM apartment on Windows).
    tokio::task::spawn_blocking(move || move_to_trash(&target))
        .await
        .map_err(ApiError::internal)?
        .map_err(|e| ApiError::internal(format!("move to trash failed: {e}")))?;
    state.emit_files_changed(&parent_rel);
    Ok(Json(json!({ "ok": true })))
}
