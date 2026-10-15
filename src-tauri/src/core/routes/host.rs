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
    Json(HostOverview {
        host_name: state.host_name.clone(),
        port: state.lan_port,
        addresses: state.addresses(),
        shared_root: crate::core::paths::shell_path(state.sandbox().root()).display().to_string(),
        ca_fingerprint: state.tls.ca_fingerprint(),
        ca_name: state.tls.ca_name(),
        paused: state.is_paused(),
        version: env!("CARGO_PKG_VERSION").to_string(),
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
    Json(PairingSession { code, expires_at, urls, ca_fingerprint: state.tls.ca_fingerprint() })
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

/// Opens with the default app; launchable files are revealed in the folder instead.
pub async fn open(State(state): State<Shared>, Json(b): Json<PathBody>) -> ApiResult<Json<Value>> {
    let path = state.sandbox().resolve_existing(&b.path)?;
    let bridge = state.bridge.clone();
    // Covers app bundles too (directories named *.app).
    let reveal = is_launchable(&path);
    tokio::task::spawn_blocking(move || if reveal { bridge.reveal_path(&path) } else { bridge.open_path(&path) })
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::internal)?;
    Ok(Json(json!({ "action": if reveal { "revealed" } else { "opened" } })))
}

pub async fn reveal(State(state): State<Shared>, Json(b): Json<PathBody>) -> ApiResult<StatusCode> {
    let path = state.sandbox().resolve_existing(&b.path)?;
    let bridge = state.bridge.clone();
    tokio::task::spawn_blocking(move || bridge.reveal_path(&path)).await.map_err(ApiError::internal)?.map_err(ApiError::internal)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct UrlBody {
    url: String,
}

/// Opens a link from a chat message in the system browser (http/https only).
pub async fn open_url(State(state): State<Shared>, Json(b): Json<UrlBody>) -> ApiResult<StatusCode> {
    let url = b.url.trim();
    let lower = url.to_ascii_lowercase();
    let ok = (lower.starts_with("http://") || lower.starts_with("https://"))
        && url.len() <= 4096
        && !url.chars().any(|c| c.is_whitespace() || c.is_control());
    if !ok {
        return Err(ApiError::bad_request("只能打开 http/https 链接"));
    }
    state.bridge.open_url(url).map_err(ApiError::internal)?;
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
    tracing::info!("shared folder changed to {}", dir.display());
    Ok(overview(State(state)).await.into_response())
}

pub async fn reset_cert(State(state): State<Shared>) -> ApiResult<StatusCode> {
    let ips = state.address_ips();
    let host = state.host_name.clone();
    let st = state.clone();
    tokio::task::spawn_blocking(move || st.tls.reset(&host, &ips)).await.map_err(ApiError::internal)??;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn clear_messages(State(state): State<Shared>) -> ApiResult<StatusCode> {
    state.db.clear_messages()?;
    state.emit(Audience::Everyone, "cleared", &json!({}));
    Ok(StatusCode::NO_CONTENT)
}
