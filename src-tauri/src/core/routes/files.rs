//! Browsing and managing the shared folder.

use std::path::Path as FsPath;
use std::time::UNIX_EPOCH;

use axum::body::Body;
use axum::extract::{Query, Request, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::core::auth::Session;
use crate::core::error::{ApiError, ApiResult};
use crate::core::model::{DirListing, FileEntry};
use crate::core::paths::{Sandbox, sanitize_filename, segments};
use crate::core::state::{AppState, INBOX_DIR, SENT_DIR, Shared, is_chat_path};
use crate::core::transfer::{
    REJECTED_UPLOAD_DRAIN, check_fits, declared_len, discard_body, preflight, receive_upload, serve_path, unless_root_gone, upload_length,
};

fn require_write(session: &Session) -> ApiResult<()> {
    if session.can_write() { Ok(()) } else { Err(ApiError::forbidden("这台设备只有读取权限")) }
}

/// Chat attachments of every conversation live in 收件箱/已发送, so phones
/// reach them only through their own messages, never by browsing.
fn require_visible(session: &Session, rel: &str) -> ApiResult<()> {
    if !session.is_host() && is_chat_path(rel) {
        return Err(ApiError::forbidden("聊天收发的文件只在电脑上可见"));
    }
    Ok(())
}

/// The same check on where a path really leads: a symlink or a Windows 8.3
/// alias can name a chat folder without spelling it, and an earlier shared
/// folder's chat folders may now sit inside this one.
fn require_visible_at(session: &Session, state: &AppState, resolved: &FsPath) -> ApiResult<()> {
    if !session.is_host() && state.is_chat_location(resolved) {
        return Err(ApiError::forbidden("聊天收发的文件只在电脑上可见"));
    }
    Ok(())
}

/// Phones may not create items named like the chat folders at the top level.
fn require_unreserved(session: &Session, sandbox: &Sandbox, dir: &FsPath, name: &str) -> ApiResult<()> {
    if !session.is_host() && dir == sandbox.root() && matches!(name, INBOX_DIR | SENT_DIR) {
        return Err(ApiError::conflict("这个名称留给聊天文件夹使用"));
    }
    Ok(())
}

/// A listed entry that is, or (through a symlink) leads into, a chat folder.
fn leads_into_chat_folder(state: &AppState, sandbox: &Sandbox, entry: &FileEntry) -> bool {
    is_chat_path(&entry.path) || sandbox.resolve_existing(&entry.path).is_ok_and(|p| state.is_chat_location(&p))
}

/// Names never shown to clients.
fn is_hidden(name: &str, meta: &std::fs::Metadata) -> bool {
    if name.starts_with('.') || (name.starts_with('~') && name.contains('$')) {
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
        let path = item.path();
        // Only list links that stay inside the shared folder.
        if link_meta.file_type().is_symlink() && sandbox.resolve_existing(&format!("{}/{name}", sandbox.relative(dir).unwrap_or_default())).is_err() {
            continue;
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
    require_visible(&session, &q.path)?;
    let sandbox = state.usable_sandbox()?;
    let dir = sandbox.resolve_dir(&q.path)?;
    require_visible_at(&session, &state, &dir)?;
    let rel = sandbox.relative(&dir).unwrap_or_default();
    let (sb, st) = (sandbox.clone(), state.clone());
    let hide_chat = !session.is_host();
    let entries = tokio::task::spawn_blocking(move || {
        let mut entries = list_dir(&sb, &dir)?;
        if hide_chat {
            entries.retain(|e| !leads_into_chat_folder(&st, &sb, e));
        }
        std::io::Result::Ok(entries)
    })
    .await
    .map_err(ApiError::internal)??;
    Ok(Json(DirListing { path: rel, entries, can_write: session.can_write() }))
}

pub async fn download(State(state): State<Shared>, Extension(session): Extension<Session>, Query(q): Query<PathQuery>, req: Request) -> ApiResult<Response> {
    require_visible(&session, &q.path)?;
    let sandbox = state.usable_sandbox()?;
    let path = sandbox.resolve_file(&q.path)?;
    require_visible_at(&session, &state, &path)?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
    let (parts, _) = req.into_parts();
    Ok(serve_path(&state, &session, &path, &name, q.inline == Some(1), &parts).await)
}

#[derive(Deserialize)]
pub struct UploadQuery {
    #[serde(default)]
    dir: String,
    name: String,
    /// Chosen by the client per file; a retry sends the same one.
    upload_id: Option<String>,
    /// The file's size as the phone sees it (see `upload_length`).
    size: Option<u64>,
}

pub async fn upload(
    State(state): State<Shared>,
    Extension(session): Extension<Session>,
    Query(q): Query<UploadQuery>,
    headers: HeaderMap,
    body: Body,
) -> ApiResult<Json<FileEntry>> {
    let mut body = Some(body);
    let result = store_upload(&state, &session, &q, declared_len(&headers), &mut body).await.map_err(|e| unless_root_gone(&state, e));
    // Left unread by a retry that already went through, or by a rejection.
    if let Some(rest) = body {
        discard_body(&headers, rest, if result.is_ok() { u64::MAX } else { REJECTED_UPLOAD_DRAIN }).await;
    }
    result.map(Json)
}

async fn store_upload(state: &Shared, session: &Session, q: &UploadQuery, declared: Option<u64>, body: &mut Option<Body>) -> ApiResult<FileEntry> {
    require_write(session)?;
    require_visible(session, &q.dir)?;
    let sandbox = state.usable_sandbox()?;
    let dir = sandbox.resolve_dir(&q.dir)?;
    require_visible_at(session, state, &dir)?;
    let name = sanitize_filename(&q.name).ok_or_else(|| ApiError::bad_request("文件名无效"))?;
    require_unreserved(session, &sandbox, &dir, &name)?;
    let upload_id = crate::core::chat::valid_client_id(q.upload_id.as_deref())?;
    let who = session.device_id().unwrap_or("host");
    // A retry whose first attempt already landed (only the response was lost).
    if let Some(id) = upload_id
        && let Some(entry) = state.recent_upload(who, id).and_then(|p| entry_for(&sandbox, &p))
    {
        return Ok(entry);
    }
    let length = upload_length(declared, q.size)?;
    let mut reservation = preflight(state, &dir, length)?;
    let body = body.take().ok_or_else(|| ApiError::internal("upload body already used"))?;
    let (path, _) = receive_upload(state, session, body, &dir, &name, length, reservation.as_mut()).await?;
    if let Some(id) = upload_id {
        state.remember_upload(who, id, path.clone());
    }
    let entry = entry_for(&sandbox, &path).ok_or_else(|| ApiError::internal("uploaded file vanished"))?;
    state.emit_files_changed(&sandbox.relative(&dir).unwrap_or_default());
    Ok(entry)
}

#[derive(Deserialize)]
pub struct UploadCheckQuery {
    /// A folder in the shared folder (Files); left out for a chat attachment.
    dir: Option<String>,
    size: u64,
}

/// "Would a file of this size fit?", asked before sending it: 204, or the
/// same 507 / 413 / 503 the upload itself would get. Reserves nothing.
pub async fn upload_check(State(state): State<Shared>, Extension(session): Extension<Session>, Query(q): Query<UploadCheckQuery>) -> ApiResult<axum::http::StatusCode> {
    let sandbox = state.usable_sandbox()?;
    let dir = match q.dir.as_deref() {
        Some(rel) => {
            require_write(&session)?;
            require_visible(&session, rel)?;
            let dir = sandbox.resolve_dir(rel)?;
            require_visible_at(&session, &state, &dir)?;
            dir
        }
        None => sandbox.root().to_path_buf(),
    };
    check_fits(&state, &dir, q.size)?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct MkdirBody {
    #[serde(default)]
    dir: String,
    name: String,
}

pub async fn mkdir(State(state): State<Shared>, Extension(session): Extension<Session>, Json(b): Json<MkdirBody>) -> ApiResult<Json<FileEntry>> {
    require_write(&session)?;
    require_visible(&session, &b.dir)?;
    let sandbox = state.usable_sandbox()?;
    let dir = sandbox.resolve_dir(&b.dir)?;
    require_visible_at(&session, &state, &dir)?;
    let name = sanitize_filename(&b.name).ok_or_else(|| ApiError::bad_request("名称无效"))?;
    require_unreserved(&session, &sandbox, &dir, &name)?;
    let target = dir.join(&name);
    if std::fs::symlink_metadata(&target).is_ok() {
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
    require_visible(&session, &b.path)?;
    let sandbox = state.usable_sandbox()?;
    // resolve_entry: renaming a symlink renames the link, not its target.
    let src = sandbox.resolve_entry(&b.path)?;
    let parent = src.parent().ok_or_else(|| ApiError::bad_request("路径无效"))?.to_path_buf();
    // The item itself too: a phone may not rename a chat folder away.
    require_visible_at(&session, &state, &src)?;
    require_visible_at(&session, &state, &parent)?;
    let name = sanitize_filename(&b.name).ok_or_else(|| ApiError::bad_request("名称无效"))?;
    require_unreserved(&session, &sandbox, &parent, &name)?;
    let dst = parent.join(&name);
    // Allow case-only renames on case-insensitive disks (same file).
    let same_file = src.file_name().map(|n| n.to_string_lossy().to_lowercase()) == Some(name.to_lowercase())
        && dunce::canonicalize(&dst).ok() == dunce::canonicalize(&src).ok();
    if std::fs::symlink_metadata(&dst).is_ok() && !same_file {
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

/// Windows only keeps a Recycle Bin on fixed disks. On USB sticks and network
/// shares the shell would ask on the desktop to delete permanently instead.
#[cfg(windows)]
fn has_recycle_bin(path: &FsPath) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Component, Prefix};
    let Some(Component::Prefix(prefix)) = path.components().next() else { return false };
    let root = match prefix.kind() {
        Prefix::Disk(d) | Prefix::VerbatimDisk(d) => format!("{}:\\", d as char),
        _ => return false, // UNC paths: network shares
    };
    let wide: Vec<u16> = std::ffi::OsStr::new(&root).encode_wide().chain(std::iter::once(0)).collect();
    // SAFETY: `wide` is a NUL-terminated UTF-16 string that outlives the call.
    let kind = unsafe { windows_sys::Win32::Storage::FileSystem::GetDriveTypeW(wide.as_ptr()) };
    kind == 3 // DRIVE_FIXED
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

/// Moves `target` to the Trash / Recycle Bin; refused where Windows keeps no
/// Recycle Bin, so nothing is ever deleted for good.
pub(super) async fn trash(target: std::path::PathBuf) -> ApiResult<()> {
    #[cfg(windows)]
    if !has_recycle_bin(&target) {
        return Err(ApiError::conflict("这个位置（U 盘或网络共享）没有回收站，为避免永久删除，请在电脑上处理这个文件"));
    }
    // A fresh thread each time: on Windows the trash crate sets up COM per
    // thread and aborts if a pooled thread was already initialised differently.
    tokio::task::spawn_blocking(move || std::thread::spawn(move || move_to_trash(&target)).join())
        .await
        .map_err(ApiError::internal)?
        .map_err(|_| ApiError::internal("trash thread panicked"))?
        .map_err(|e| ApiError::internal(format!("move to trash failed ({})", trash_failure(&e))))
}

/// The trash crate's messages quote the path; keep only what kind of failure it was.
fn trash_failure(e: &trash::Error) -> String {
    match e {
        trash::Error::Os { code, .. } => format!("os error {code}"),
        trash::Error::TargetedRoot => "targeted a root".into(),
        _ => "other".into(),
    }
}

pub async fn delete(State(state): State<Shared>, Extension(session): Extension<Session>, Json(b): Json<DeleteBody>) -> ApiResult<Json<Value>> {
    require_write(&session)?;
    require_visible(&session, &b.path)?;
    let sandbox = state.usable_sandbox()?;
    // Deleting a symlink removes the link only (trash keeps the last component).
    let target = sandbox.resolve_entry(&b.path).map_err(|e| match e {
        crate::core::paths::PathError::Invalid if segments(&b.path).is_ok_and(|s| s.is_empty()) => ApiError::bad_request("不能删除共享文件夹本身"),
        other => other.into(),
    })?;
    require_visible_at(&session, &state, &target)?;
    if let Some(parent) = target.parent() {
        require_visible_at(&session, &state, parent)?;
    }
    let parent_rel = target.parent().and_then(|p| sandbox.relative(p)).unwrap_or_default();
    trash(target).await?;
    state.emit_files_changed(&parent_rel);
    Ok(Json(json!({ "ok": true })))
}
