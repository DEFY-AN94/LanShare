//! The optional global shortcut that sends the clipboard to a phone. None is
//! set by default; Settings records one. A new one is registered before the
//! old one is let go, so a refusal keeps the shortcut that worked.

use std::sync::Mutex;

use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::core::ServerHandle;

static CURRENT: Mutex<Option<Shortcut>> = Mutex::new(None);

/// Replaces the registered shortcut with `accelerator` (e.g.
/// "Control+Alt+Super+KeyV"), or removes it. Blocking: the plugin does its
/// work on the main thread, so never call this from the main thread itself.
pub fn set(app: &AppHandle, accelerator: Option<&str>) -> Result<(), String> {
    let mut current = CURRENT.lock().unwrap_or_else(|p| p.into_inner());
    let wanted = accelerator.map(|a| a.parse::<Shortcut>().map_err(|e| format!("快捷键无效（{e}）"))).transpose()?;
    if wanted == *current {
        return Ok(());
    }
    let gs = app.global_shortcut();
    if let Some(next) = wanted {
        gs.on_shortcut(next, |app, _shortcut, event| {
            if event.state == ShortcutState::Pressed {
                send_clipboard(app);
            }
        })
        .map_err(|e| e.to_string())?;
    }
    if let Some(old) = current.take() {
        let _ = gs.unregister(old);
    }
    *current = wanted;
    Ok(())
}

fn send_clipboard(app: &AppHandle) {
    let Some(server) = app.try_state::<ServerHandle>() else { return };
    let state = server.state.clone();
    tauri::async_runtime::spawn(async move { crate::core::quick::shortcut_pressed(&state).await });
}
