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
use crate::core::events::Audience;
use crate::core::model::{DeviceOut, HostOverview, PairRequestOut, PairingSession, PairingUrl};
use crate::core::paths::{Sandbox, unsafe_root_reason};
use crate::core::state::Shared;
use crate::core::util::{now_ms, random_hex, random_token, sha256_hex};

pub async fn overview(State(state): State<Shared>) -> Json<HostOverview> {
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
    })
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
    DeviceOut {
        online: state.is_online(&d.id),
        id: d.id,
        name: d.name,
        can_write: d.can_write,
        created_at: d.created_at,
        last_seen_at: d.last_seen_at,
        last_message,
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

/// Things the OS would *run* rather than show (apps, scripts, installers,
/// shortcuts). Files from other devices of this kind are only revealed.
fn is_launchable(path: &std::path::Path) -> bool {
    const RISKY: &[&str] = &[
        "app", "command", "terminal", "tool", "sh", "bash", "zsh", "csh", "fish", "pkg", "mpkg", "jar", "py", "rb", "pl", "php", "scpt",
        "scptd", "applescript", "workflow", "action", "fileloc", "inetloc", "webloc", "prefpane", "kext", "plugin", "osax", "dylib", "exe",
        "bat", "cmd", "com", "scr", "msi", "msp", "ps1", "psm1", "vbs", "vbe", "js", "jse", "wsf", "wsh", "hta", "lnk", "url", "reg",
        "cpl", "msc", "appref-ms", "settingcontent-ms", "library-ms", "inf", "pif", "application", "gadget", "desktop", "appimage",
        // Windows help files can run script; packages install; disk images mount.
        "chm", "msix", "msixbundle", "appx", "appxbundle", "appinstaller", "iso", "img", "vhd", "vhdx",
        // More Windows types that run, install or reach out when opened.
        "pyw", "pyz", "pyzw", "msu", "ppkg", "searchconnector-ms", "search-ms", "website", "scf", "theme", "themepack", "deskthemepack",
        "diagcab",
    ];
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if RISKY.contains(&ext.as_str()) {
        return true;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(m) = std::fs::metadata(path)
            && m.is_file()
            && m.permissions().mode() & 0o111 != 0
        {
            return true;
        }
    }
    false
}

/// The shell's error text quotes the path or link, which must not reach the log.
fn shell_failed() -> ApiError {
    ApiError::internal("the system could not open the item")
}

#[cfg(test)]
mod tests {
    use super::is_launchable;
    use std::path::Path;

    #[test]
    fn files_that_run_or_mount_are_only_revealed() {
        for name in [
            "setup.exe", "run.ps1", "link.lnk", "help.chm", "HELP.CHM", "app.msix", "pkg.appx", "x.appinstaller", "disk.iso", "disk.img",
            "disk.vhd", "disk.vhdx", "Tool.app", "x.sh", "x.pyw", "fix.diagcab", "update.msu", "x.scf", "dark.theme",
        ] {
            assert!(is_launchable(Path::new(name)), "{name}");
        }
        for name in ["photo.jpg", "doc.pdf", "notes.txt", "movie.mov", "archive.zip"] {
            assert!(!is_launchable(Path::new(name)), "{name}");
        }
    }
}

/// Opens with the default app; launchable files are revealed in the folder instead.
pub async fn open(State(state): State<Shared>, Json(b): Json<PathBody>) -> ApiResult<Json<Value>> {
    let path = state.usable_sandbox()?.resolve_existing(&b.path)?;
    let bridge = state.bridge.clone();
    // Covers app bundles too (directories named *.app).
    let reveal = is_launchable(&path);
    tokio::task::spawn_blocking(move || if reveal { bridge.reveal_path(&path) } else { bridge.open_path(&path) })
        .await
        .map_err(ApiError::internal)?
        .map_err(|_| shell_failed())?;
    Ok(Json(json!({ "action": if reveal { "revealed" } else { "opened" } })))
}

pub async fn reveal(State(state): State<Shared>, Json(b): Json<PathBody>) -> ApiResult<StatusCode> {
    let path = state.usable_sandbox()?.resolve_existing(&b.path)?;
    let bridge = state.bridge.clone();
    tokio::task::spawn_blocking(move || bridge.reveal_path(&path)).await.map_err(ApiError::internal)?.map_err(|_| shell_failed())?;
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

/// Launch at login (macOS 13+). `suggest`: offer it once, after the first pairing.
pub async fn login_item(State(state): State<Shared>) -> ApiResult<Json<Value>> {
    let bridge = state.bridge.clone();
    let current = tokio::task::spawn_blocking(move || bridge.login_item()).await.map_err(ApiError::internal)?;
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
        ApiError::conflict("macOS 没有接受这个设置，请到「系统设置 › 通用 › 登录项」里查看")
    })?;
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
    Ok(StatusCode::NO_CONTENT)
}
