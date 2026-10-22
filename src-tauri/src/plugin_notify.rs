//! tauri-plugin-notification: Windows toasts with 复制 / 打开, and the plain
//! notification everywhere else when nothing better is available (macOS
//! falls back to it when UNUserNotificationCenter is not allowed).
//!
//! On Windows a button or a click comes back only while the toast is on
//! screen; a later click in Action Center does not (PLAN.md backlog).

use tauri::AppHandle;
use tauri_plugin_notification::{Action, ActionType, NotificationExt};

use crate::core::bridge::{MessageNotice, NoticeKind};
use crate::core::quick;
use crate::core::state::Shared;

const TYPE_TEXT: &str = "lanshare.text";
const TYPE_FILE: &str = "lanshare.file";

/// Windows toasts are silent unless they name a sound; macOS plays its own.
fn builder(app: &AppHandle) -> tauri_plugin_notification::NotificationBuilder<tauri::Wry> {
    let b = app.notification().builder();
    if cfg!(windows) { b.sound("Default") } else { b }
}

pub fn plain(app: &AppHandle, title: &str, body: &str) {
    let _ = builder(app).title(title).body(body).show();
}

/// A phone's message as a toast with its button. A quiet update is skipped:
/// a toast cannot be updated in place here.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn message(app: &AppHandle, n: &MessageNotice) {
    if n.quiet {
        return;
    }
    let kind = match n.kind {
        NoticeKind::Text => TYPE_TEXT,
        NoticeKind::File => TYPE_FILE,
    };
    let _ = builder(app)
        .title(&n.title)
        .body(&n.body)
        .action_type_id(kind)
        .extra("message_id", n.message_id.to_string())
        .extra("device_id", n.device_id.clone())
        .show();
}

/// Registers the buttons and sends their clicks to `quick::on_notification`.
#[cfg_attr(not(windows), allow(dead_code))]
fn register(app: &AppHandle, state: Shared) -> tauri_plugin_notification::Result<()> {
    let n = app.notification();
    n.register_action_types(vec![
        ActionType::builder(TYPE_TEXT).actions(vec![Action::builder(quick::ACTION_COPY, "复制").build()]).build(),
        ActionType::builder(TYPE_FILE).actions(vec![Action::builder(quick::ACTION_OPEN, "打开").build()]).build(),
    ])?;
    n.on_action(move |performed| {
        let action = match performed.action_id() {
            quick::ACTION_COPY => quick::ACTION_COPY,
            quick::ACTION_OPEN => quick::ACTION_OPEN,
            "tap" => quick::ACTION_SHOW,
            _ => quick::ACTION_DISMISS,
        };
        let extra = performed.notification().map(|n| n.extra().clone()).unwrap_or_default();
        let message_id = extra.get("message_id").and_then(|v| v.as_str()).and_then(|s| s.parse().ok());
        let device_id = extra.get("device_id").and_then(|v| v.as_str()).map(str::to_string);
        let state = state.clone();
        tauri::async_runtime::spawn(async move { quick::on_notification(&state, action, message_id, device_id.as_deref()).await });
    })?;
    Ok(())
}

/// Windows only: on macOS the plugin's buttons would use the deprecated API.
pub fn on_action(_app: &AppHandle, _state: Shared) {
    #[cfg(windows)]
    if register(_app, _state).is_err() {
        tracing::warn!("notification buttons could not be registered");
    }
}
