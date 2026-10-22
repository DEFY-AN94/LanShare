//! The menu bar (macOS) / tray (Windows) icon and its menu:
//! 打开 LanShare, 复制最近收到的文字, 发送剪贴板到「…」 with a submenu per
//! device, 暂停共享 and 退出 LanShare.

use std::time::Duration;

use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Wry};
use tokio::sync::broadcast::error::RecvError;

use crate::core::quick;
use crate::core::state::Shared;

pub const TRAY_ID: &str = "main-tray";
const OPEN: &str = "open";
const COPY_LATEST: &str = "copy-latest";
const CLIP_RECENT: &str = "clip-recent";
const CLIP_TO: &str = "clip-to";
const CLIP_DEVICE: &str = "clip:";
const PAUSE: &str = "pause";
const QUIT: &str = "quit";

/// Events after which the device list or the clipboard target may differ.
const REFRESH_ON: &[&str] = &["devices", "msg", "pairing_done", "cleared", "pruned", "settings"];

#[derive(Clone)]
struct Items {
    recent: MenuItem<Wry>,
    to: Submenu<Wry>,
    pause: CheckMenuItem<Wry>,
}

/// Menu labels show device names, which phones choose (at most 40 characters).
fn label(name: &str) -> String {
    // Menus take "&" as an access key marker (muda drops it on macOS too).
    name.replace('&', "&&")
}

fn refresh(app: &AppHandle, items: &Items, state: &Shared) {
    let devices = quick::recent_devices(state);
    match quick::clipboard_target(state) {
        Some((_, name)) => {
            let _ = items.recent.set_text(format!("发送剪贴板到「{}」", label(&name)));
            let _ = items.recent.set_enabled(true);
        }
        None => {
            let _ = items.recent.set_text("发送剪贴板（还没有配对的设备）");
            let _ = items.recent.set_enabled(false);
        }
    }
    if let Ok(old) = items.to.items() {
        for item in old {
            let _ = items.to.remove(&item);
        }
    }
    for (id, name) in &devices {
        if let Ok(item) = MenuItem::with_id(app, format!("{CLIP_DEVICE}{id}"), label(name), true, None::<&str>) {
            let _ = items.to.append(&item);
        }
    }
    let _ = items.to.set_enabled(!devices.is_empty());
    let _ = items.pause.set_checked(state.is_paused());
}

pub fn create(app: &tauri::App, state: Shared) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, OPEN, "打开 LanShare", true, None::<&str>)?;
    let copy_latest = MenuItem::with_id(app, COPY_LATEST, "复制最近收到的文字", true, None::<&str>)?;
    let recent = MenuItem::with_id(app, CLIP_RECENT, "发送剪贴板", false, None::<&str>)?;
    let to = Submenu::with_id(app, CLIP_TO, "发送剪贴板到", false)?;
    let pause = CheckMenuItem::with_id(app, PAUSE, "暂停共享", true, state.is_paused(), None::<&str>)?;
    let quit = MenuItem::with_id(app, QUIT, "退出 LanShare", true, None::<&str>)?;
    let (sep1, sep2, sep3) = (PredefinedMenuItem::separator(app)?, PredefinedMenuItem::separator(app)?, PredefinedMenuItem::separator(app)?);
    let entries: [&dyn IsMenuItem<Wry>; 9] = [&open, &sep1, &copy_latest, &recent, &to, &sep2, &pause, &sep3, &quit];
    let menu = Menu::with_items(app, &entries)?;
    let items = Items { recent, to, pause };

    let menu_state = state.clone();
    let pause_item = items.pause.clone();
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("LanShare")
        .menu(&menu)
        .show_menu_on_left_click(cfg!(target_os = "macos"))
        .on_menu_event(move |app, event| {
            let id = event.id().as_ref().to_string();
            let state = menu_state.clone();
            match id.as_str() {
                OPEN => crate::show_main_window(app),
                // The check mark has already flipped when this fires.
                PAUSE => state.set_paused(pause_item.is_checked().unwrap_or(false)),
                QUIT => app.exit(0),
                COPY_LATEST => {
                    tauri::async_runtime::spawn_blocking(move || quick::copy_latest_and_tell(&state));
                }
                CLIP_RECENT => {
                    tauri::async_runtime::spawn(async move { quick::send_clipboard_and_tell(&state, None).await });
                }
                other => {
                    if let Some(device) = other.strip_prefix(CLIP_DEVICE).map(str::to_string) {
                        tauri::async_runtime::spawn(async move { quick::send_clipboard_and_tell(&state, Some(&device)).await });
                    }
                }
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                crate::show_main_window(tray.app_handle());
            }
        });
    // macOS menu bars want a monochrome "template" image; Windows trays use the color icon.
    #[cfg(target_os = "macos")]
    {
        builder = builder.icon(tauri::include_image!("./icons/tray.png")).icon_as_template(true);
    }
    #[cfg(not(target_os = "macos"))]
    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }
    builder.build(app)?;

    let handle = app.handle().clone();
    refresh(&handle, &items, &state);
    // Follows pausing from the window, new devices and conversations. A burst
    // of changes is handled once; events missed under load mean a full refresh.
    let mut rx = state.events.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(ev) if ev.name == "paused" => {
                    let _ = items.pause.set_checked(ev.data.contains("true"));
                    continue;
                }
                Ok(ev) if !REFRESH_ON.contains(&ev.name) => continue,
                Ok(_) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => return,
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
            while let Ok(ev) = rx.try_recv() {
                if ev.name == "paused" {
                    let _ = items.pause.set_checked(ev.data.contains("true"));
                }
            }
            refresh(&handle, &items, &state);
        }
    });
    Ok(())
}
