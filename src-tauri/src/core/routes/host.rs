//! Host-only endpoints (desktop window on the loopback listener).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use qrcode::QrCode;
use qrcode::render::svg;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::core::auth::Session;
use crate::core::db::DeviceRow;
use crate::core::error::{ApiError, ApiResult};
use crate::core::bridge::Withdraw;
use crate::core::events::Audience;
use crate::core::model::{DeviceOut, HostOverview, PairRequestOut, PairingSession, PairingUrl};
use crate::core::paths::{Sandbox, unsafe_root_reason};
use crate::core::state::Shared;
use crate::core::transfer::checked_blocking;
use crate::core::quick::{self, shell_failed};
use crate::core::util::{now_ms, random_hex, random_token, sha256_hex};

pub async fn overview(State(state): State<Shared>) -> Json<HostOverview> {
    let bridge = state.bridge.clone();
    let notifications = tokio::task::spawn_blocking(move || bridge.notifications_state()).await.unwrap_or("unknown");
    let shown = |p: &std::path::Path| crate::core::paths::shell_path(p).display().to_string();
    let sandbox = state.sandbox();
    Json(HostOverview {
        host_name: state.host_name.clone(),
        port: state.lan_port,
        addresses: state.addresses(),
        shared_root: shown(sandbox.root()),
        shared_root_unavailable: sandbox.check_root().err(),
        preferred_root: state.preferred_root().as_deref().map(shown),
        ca_fingerprint: state.tls.ca_fingerprint(),
        ca_name: state.tls.ca_name(),
        paused: state.is_paused(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        network_warning: state.network_warning(),
        chat_retention_days: state.chat_retention_days(),
        verbose_logging: crate::core::logging::verbose(),
        platform: std::env::consts::OS,
        notifications,
        notify_hide_text: state.notify_hide_text(),
        clipboard_shortcut: state.clipboard_shortcut(),
        clipboard_shortcut_error: state.shortcut_error(),
        clipboard_target: state.clipboard_target(),
    })
}

#[derive(Deserialize)]
pub struct ViewingBody {
    device: Option<String>,
}

/// The window shows this conversation in front (or none): it counts as read.
pub async fn set_viewing(State(state): State<Shared>, Json(b): Json<ViewingBody>) -> ApiResult<StatusCode> {
    let device = b.device.filter(|d| !d.is_empty());
    if let Some(d) = &device
        && state.db.device(d)?.is_none()
    {
        return Err(ApiError::not_found("设备不存在"));
    }
    state.set_viewing(device);
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct NotificationsBody {
    hide_text: bool,
}

pub async fn set_notifications(State(state): State<Shared>, Json(b): Json<NotificationsBody>) -> ApiResult<StatusCode> {
    state.set_notify_hide_text(b.hide_text)?;
    if b.hide_text {
        // Texts already on screen go as well.
        state.notifier.forget_all();
        state.bridge.withdraw_notifications(&Withdraw::All);
    }
    state.emit(Audience::Host, "settings", &json!({}));
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct ShortcutBody {
    shortcut: Option<String>,
}

/// Registers the new global shortcut first; if the system refuses it, the
/// old one stays and Settings shows why.
pub async fn set_clipboard_shortcut(State(state): State<Shared>, Json(b): Json<ShortcutBody>) -> ApiResult<StatusCode> {
    let shortcut = b.shortcut.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    if let Some(s) = &shortcut {
        if s.len() > 64 {
            return Err(ApiError::bad_request("快捷键无效"));
        }
        quick::check_shortcut(s, cfg!(windows)).map_err(ApiError::bad_request)?;
    }
    let bridge = state.bridge.clone();
    let wanted = shortcut.clone();
    tokio::task::spawn_blocking(move || bridge.set_shortcut(wanted.as_deref()))
        .await
        .map_err(ApiError::internal)?
        .map_err(|e| ApiError::conflict(format!("这个快捷键没能设置：{e}")))?;
    state.set_clipboard_shortcut(shortcut)?;
    state.emit(Audience::Host, "settings", &json!({}));
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct RecordingBody {
    on: bool,
}

/// Settings is recording a new shortcut: pressing the current one then
/// tells Settings instead of sending the clipboard.
pub async fn set_shortcut_recording(State(state): State<Shared>, Json(b): Json<RecordingBody>) -> StatusCode {
    state.set_shortcut_recording(b.on);
    StatusCode::NO_CONTENT
}

#[derive(Deserialize)]
pub struct TargetBody {
    device: Option<String>,
}

/// Where the clipboard goes; `None` is the most recent conversation.
pub async fn set_clipboard_target(State(state): State<Shared>, Json(b): Json<TargetBody>) -> ApiResult<StatusCode> {
    let device = b.device.filter(|d| !d.is_empty());
    if let Some(d) = &device
        && state.db.device(d)?.is_none()
    {
        return Err(ApiError::not_found("设备不存在"));
    }
    state.set_clipboard_target(device)?;
    state.emit(Audience::Host, "settings", &json!({}));
    Ok(StatusCode::NO_CONTENT)
}

fn qr_svg(text: &str) -> String {
    match QrCode::new(text.as_bytes()) {
        Ok(code) => {
            let svg = code
                .render::<svg::Color>()
                .quiet_zone(true)
                .min_dimensions(240, 240)
                .dark_color(svg::Color("#000000"))
                .light_color(svg::Color("#ffffff"))
                .build();
            // Drop the XML prolog; the UI embeds it as an image.
            svg.split_once("?>").map(|(_, rest)| rest.trim_start().to_string()).unwrap_or(svg)
        }
        Err(_) => String::new(),
    }
}

pub async fn start_pairing(State(state): State<Shared>) -> Json<PairingSession> {
    let (token, code, expires_at) = state.pairing.lock().unwrap().start(now_ms());
    let urls = state
        .addresses()
        .into_iter()
        .map(|a| {
            let url = format!("https://{}:{}/pair?t={}", a.ip, state.lan_port, token);
            PairingUrl { qr_svg: qr_svg(&url), url, ip: a.ip, iface: a.iface }
        })
        .collect();
    Json(PairingSession { code, expires_at, urls, ca_fingerprint: state.tls.ca_fingerprint(), network_warning: state.network_warning() })
}

pub async fn cancel_pairing(State(state): State<Shared>) -> StatusCode {
    state.pairing.lock().unwrap().cancel();
    StatusCode::NO_CONTENT
}

pub async fn pairing_requests(State(state): State<Shared>) -> Json<Vec<PairRequestOut>> {
    Json(state.pairing.lock().unwrap().pending(now_ms()).iter().map(|r| r.to_out()).collect())
}

pub async fn approve(State(state): State<Shared>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let req = state.pairing.lock().unwrap().get_pending(&id, now_ms()).ok_or_else(|| ApiError::not_found("请求已过期"))?;
    let device_id = random_hex(12);
    let secret = random_token();
    state.db.insert_device(&DeviceRow {
        id: device_id.clone(),
        name: req.name.clone(),
        token_hash: sha256_hex(&secret),
        can_write: true,
        created_at: now_ms(),
        last_seen_at: None,
    })?;
    if !state.pairing.lock().unwrap().approve(&id, format!("{device_id}.{secret}")) {
        let _ = state.db.delete_device(&device_id);
        return Err(ApiError::not_found("请求已过期"));
    }
    tracing::info!("paired new device {} ({})", req.name, req.ip);
    state.emit(Audience::Host, "pairing_done", &json!({ "id": id, "status": "approved", "name": req.name }));
    state.emit(Audience::Host, "devices", &json!({}));
    Ok(Json(json!({ "ok": true, "device_id": device_id })))
}

pub async fn reject(State(state): State<Shared>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    if !state.pairing.lock().unwrap().reject(&id) {
        return Err(ApiError::not_found("请求已过期"));
    }
    state.emit(Audience::Host, "pairing_done", &json!({ "id": id, "status": "rejected" }));
    Ok(Json(json!({ "ok": true })))
}

fn device_out(state: &Shared, d: DeviceRow) -> DeviceOut {
    let last_message = state.db.last_message(&d.id).ok().flatten().map(|m| state.message_out(m));
    let unread = state.unread_counts().get(&d.id).copied().unwrap_or(0);
    DeviceOut {
        online: state.is_online(&d.id),
        id: d.id,
        name: d.name,
        can_write: d.can_write,
        created_at: d.created_at,
        last_seen_at: d.last_seen_at,
        last_message,
        unread,
    }
}

pub async fn devices(State(state): State<Shared>) -> ApiResult<Json<Vec<DeviceOut>>> {
    let rows = state.db.devices()?;
    Ok(Json(rows.into_iter().map(|d| device_out(&state, d)).collect()))
}

#[derive(Deserialize)]
pub struct DevicePatch {
    name: Option<String>,
    can_write: Option<bool>,
}

pub async fn update_device(State(state): State<Shared>, Path(id): Path<String>, Json(p): Json<DevicePatch>) -> ApiResult<Json<DeviceOut>> {
    let name = match p.name.as_deref().map(str::trim) {
        Some("") => return Err(ApiError::bad_request("名称不能为空")),
        Some(n) => Some(n.chars().filter(|c| !c.is_control()).take(40).collect::<String>()),
        None => None,
    };
    if !state.db.update_device(&id, name.as_deref(), p.can_write)? {
        return Err(ApiError::not_found("设备不存在"));
    }
    state.emit(Audience::Host, "devices", &json!({}));
    let d = state.db.device(&id)?.ok_or_else(|| ApiError::not_found("设备不存在"))?;
    Ok(Json(device_out(&state, d)))
}

pub async fn delete_device(State(state): State<Shared>, Path(id): Path<String>) -> ApiResult<StatusCode> {
    if !state.db.delete_device(&id)? {
        return Err(ApiError::not_found("设备不存在"));
    }
    state.revoke(&id);
    state.device_removed(&id);
    state.emit(Audience::Device(id), "revoked", &json!({}));
    state.emit(Audience::Host, "devices", &json!({}));
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct PausedBody {
    paused: bool,
}

pub async fn set_paused(State(state): State<Shared>, Json(b): Json<PausedBody>) -> Json<Value> {
    state.set_paused(b.paused);
    Json(json!({ "paused": b.paused }))
}

#[derive(Deserialize)]
pub struct PathBody {
    #[serde(default)]
    path: String,
}

/// Opens with the default app; launchable files are revealed in the folder instead.
pub async fn open(State(state): State<Shared>, Json(b): Json<PathBody>) -> ApiResult<Json<Value>> {
    let check = state.path_check().await;
    let path = state.usable_sandbox()?.resolve_existing(&b.path)?;
    let action = quick::open_or_reveal(&state, path, check).await?;
    Ok(Json(json!({ "action": action })))
}

/// Opens a chat attachment where it was saved, also in an earlier shared folder.
pub async fn open_message_file(State(state): State<Shared>, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let action = quick::open_attachment(&state, id).await?;
    Ok(Json(json!({ "action": action })))
}

/// Shows a chat attachment in its folder, also in an earlier shared folder.
pub async fn reveal_message_file(State(state): State<Shared>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    quick::reveal_attachment(&state, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn reveal(State(state): State<Shared>, Json(b): Json<PathBody>) -> ApiResult<StatusCode> {
    let check = state.path_check().await;
    let path = state.usable_sandbox()?.resolve_existing(&b.path)?;
    let bridge = state.bridge.clone();
    checked_blocking(check, move || bridge.reveal_path(&path)).await?.map_err(|_| shell_failed())?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct UrlBody {
    url: String,
}

/// Opens a link from a chat message or Settings in the system browser (http/https only).
pub async fn open_url(State(state): State<Shared>, Json(b): Json<UrlBody>) -> ApiResult<StatusCode> {
    let url = b.url.trim();
    let lower = url.to_ascii_lowercase();
    let ok = (lower.starts_with("http://") || lower.starts_with("https://"))
        && url.len() <= 4096
        && !url.chars().any(|c| c.is_whitespace() || c.is_control());
    if !ok {
        return Err(ApiError::bad_request("只能打开 http/https 链接"));
    }
    state.bridge.open_url(url).map_err(|_| shell_failed())?;
    Ok(StatusCode::NO_CONTENT)
}

/// Lets the user choose a different shared folder with the native picker.
pub async fn pick_shared_root(State(state): State<Shared>, Extension(_s): Extension<Session>) -> ApiResult<Response> {
    let bridge = state.bridge.clone();
    let picked = tokio::task::spawn_blocking(move || bridge.pick_folder()).await.map_err(ApiError::internal)?;
    let Some(dir) = picked else {
        return Ok(StatusCode::NO_CONTENT.into_response());
    };
    if let Some(reason) = unsafe_root_reason(&dir, &state.home_dir, &state.data_dir) {
        return Err(ApiError::bad_request(reason));
    }
    let sandbox = Sandbox::new(&dir)
        .map_err(|e| ApiError::bad_request(format!("无法使用这个文件夹：{e}")))?
        .with_deny(std::slice::from_ref(&state.data_dir));
    {
        let mut cfg = state.config.lock().unwrap();
        cfg.shared_root = sandbox.root().to_path_buf();
        cfg.save(&state.data_dir).map_err(ApiError::internal)?;
    }
    state.set_sandbox(sandbox);
    crate::core::watch_shared_folder(&state);
    state.emit_files_changed("");
    tracing::info!("shared folder changed");
    Ok(overview(State(state)).await.into_response())
}

pub async fn reset_cert(State(state): State<Shared>) -> ApiResult<StatusCode> {
    let ips = state.address_ips();
    let host = state.host_name.clone();
    let st = state.clone();
    tokio::task::spawn_blocking(move || st.tls.reset(&host, &ips)).await.map_err(ApiError::internal)??;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct RetentionBody {
    days: u32,
}

/// Automatic chat cleanup: 0 (off), 30 or 90 days. Applies at once.
pub async fn set_chat_retention(State(state): State<Shared>, Json(b): Json<RetentionBody>) -> ApiResult<Json<Value>> {
    if !crate::core::config::RETENTION_CHOICES.contains(&b.days) {
        return Err(ApiError::bad_request("只能选择不自动删除、30 天或 90 天"));
    }
    state.set_chat_retention_days(b.days).map_err(ApiError::internal)?;
    let st = state.clone();
    let deleted = tokio::task::spawn_blocking(move || st.prune_old_messages()).await.map_err(ApiError::internal)??;
    Ok(Json(json!({ "days": b.days, "deleted": deleted })))
}

/// Opens `<data dir>/logs` in Finder / File Explorer.
pub async fn open_logs(State(state): State<Shared>) -> ApiResult<StatusCode> {
    let dir = crate::core::logging::dir().ok_or_else(|| ApiError::conflict("日志文件还没有创建"))?;
    let bridge = state.bridge.clone();
    tokio::task::spawn_blocking(move || bridge.open_path(&dir)).await.map_err(ApiError::internal)?.map_err(|_| shell_failed())?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct VerboseBody {
    on: bool,
}

/// Debug-level logging for troubleshooting; off again after a restart.
pub async fn set_verbose_logging(Json(b): Json<VerboseBody>) -> Json<Value> {
    crate::core::logging::set_verbose(b.on);
    tracing::info!("verbose logging {}", if b.on { "on" } else { "off" });
    Json(json!({ "on": b.on }))
}

/// Launch at login (macOS 13+, Windows). `suggest`: offer it once, after the first pairing.
pub async fn login_item(State(state): State<Shared>) -> ApiResult<Json<Value>> {
    let bridge = state.bridge.clone();
    let current = tokio::task::spawn_blocking(move || bridge.login_item()).await.map_err(ApiError::internal)?;
    if current == "off" {
        // Turned off elsewhere (System Settings, another tool): not put back later.
        state.set_login_item_chosen(false);
    }
    Ok(Json(json!({ "state": current, "suggest": current == "off" && !state.login_item_suggested() })))
}

#[derive(Deserialize)]
pub struct LoginItemBody {
    on: bool,
}

pub async fn set_login_item(State(state): State<Shared>, Json(b): Json<LoginItemBody>) -> ApiResult<Json<Value>> {
    state.mark_login_item_suggested();
    let bridge = state.bridge.clone();
    let current = tokio::task::spawn_blocking(move || bridge.set_login_item(b.on)).await.map_err(ApiError::internal)?.map_err(|e| {
        tracing::warn!("launch at login could not be changed: {e}");
        ApiError::conflict(if cfg!(windows) {
            "Windows 没有接受这个设置，请到「设置 › 应用 › 启动」里查看"
        } else {
            "macOS 没有接受这个设置，请到「系统设置 › 通用 › 登录项」里查看"
        })
    })?;
    state.set_login_item_chosen(b.on);
    Ok(Json(json!({ "state": current })))
}

/// "Not now" on the one-time suggestion.
pub async fn dismiss_login_item_suggestion(State(state): State<Shared>) -> StatusCode {
    state.mark_login_item_suggested();
    StatusCode::NO_CONTENT
}

/// Opens System Settings › General › Login Items (to allow LanShare there).
pub async fn open_login_item_settings(State(state): State<Shared>) -> ApiResult<StatusCode> {
    let bridge = state.bridge.clone();
    tokio::task::spawn_blocking(move || bridge.open_login_item_settings()).await.map_err(ApiError::internal)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn clear_messages(State(state): State<Shared>) -> ApiResult<StatusCode> {
    let (_, through) = state.db.clear_messages()?;
    state.mark_cleared(through);
    state.emit(Audience::Everyone, "cleared", &json!({ "through": through }));
    state.notifier.forget_all();
    state.bridge.withdraw_notifications(&Withdraw::All);
    state.unread_changed();
    Ok(StatusCode::NO_CONTENT)
}
