//! Desktop shell: starts the server, shows it in a window, adds a tray icon.

pub mod core;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::webview::{DownloadEvent, NewWindowResponse};
use tauri::{AppHandle, Manager, RunEvent, Url, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;
use tracing_subscriber::EnvFilter;

use crate::core::bridge::HostBridge;
use crate::core::state::Shared;
use crate::core::{ServerHandle, StartOptions};

const MAIN: &str = "main";
const MENU_OPEN: &str = "open";
const MENU_PAUSE: &str = "pause";
const MENU_QUIT: &str = "quit";

pub fn init_logging() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,lanshare_lib=debug"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

fn show_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(MAIN) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

struct TauriBridge {
    app: AppHandle,
}

impl HostBridge for TauriBridge {
    fn notify(&self, title: &str, body: &str) {
        // No banner while the user is already looking at the window.
        let looking = self
            .app
            .get_webview_window(MAIN)
            .map(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false))
            .unwrap_or(false);
        if !looking {
            let _ = self.app.notification().builder().title(title).body(body).show();
        }
    }

    fn open_path(&self, path: &Path) -> Result<(), String> {
        self.app.opener().open_path(path.to_string_lossy(), None::<&str>).map_err(|e| e.to_string())
    }

    fn reveal_path(&self, path: &Path) -> Result<(), String> {
        self.app.opener().reveal_item_in_dir(path).map_err(|e| e.to_string())
    }

    fn open_url(&self, url: &str) -> Result<(), String> {
        self.app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
    }

    fn pick_folder(&self) -> Option<PathBuf> {
        self.app.dialog().file().set_title("选择共享文件夹").blocking_pick_folder().and_then(|p| p.into_path().ok())
    }

    fn show_window(&self) {
        show_main_window(&self.app);
    }
}

fn create_window(app: &AppHandle, server: &ServerHandle) -> tauri::Result<()> {
    let url: Url = server.login_url().parse().expect("valid loopback url");
    let origin = url.origin();
    let nav_app = app.clone();
    let popup_app = app.clone();
    WebviewWindowBuilder::new(app, MAIN, WebviewUrl::External(url))
        .title("LanShare")
        .inner_size(1080.0, 720.0)
        .min_inner_size(820.0, 560.0)
        .center()
        // Let the page receive HTML5 file drops (drag files into chat / folders).
        .disable_drag_drop_handler()
        // The window only ever shows our own loopback page; links go to the browser.
        .on_navigation(move |u: &Url| {
            if u.origin() == origin || u.scheme() == "about" {
                return true;
            }
            if matches!(u.scheme(), "http" | "https") {
                let _ = nav_app.opener().open_url(u.as_str(), None::<&str>);
            }
            false
        })
        .on_new_window(move |u, _features| {
            if matches!(u.scheme(), "http" | "https") {
                let _ = popup_app.opener().open_url(u.as_str(), None::<&str>);
            }
            NewWindowResponse::Deny
        })
        // The host UI opens files with the OS instead of downloading them.
        .on_download(|_webview, event| !matches!(event, DownloadEvent::Requested { .. }))
        .build()?;
    Ok(())
}

fn create_tray(app: &tauri::App, state: Shared) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, MENU_OPEN, "打开 LanShare", true, None::<&str>)?;
    let pause = CheckMenuItem::with_id(app, MENU_PAUSE, "暂停共享", true, state.is_paused(), None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, MENU_QUIT, "退出 LanShare", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &pause, &sep, &quit])?;

    let menu_state = state.clone();
    let pause_item = pause.clone();
    let mut builder = TrayIconBuilder::with_id("main-tray")
        .tooltip("LanShare")
        .menu(&menu)
        .show_menu_on_left_click(cfg!(target_os = "macos"))
        .on_menu_event(move |app, event| match event.id().as_ref() {
            MENU_OPEN => show_main_window(app),
            // The check mark has already flipped when this fires.
            MENU_PAUSE => menu_state.set_paused(pause_item.is_checked().unwrap_or(false)),
            MENU_QUIT => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }
    builder.build(app)?;

    // Keep the tray check mark in sync when pausing from the window.
    let mut rx = state.events.subscribe();
    tauri::async_runtime::spawn(async move {
        while let Ok(ev) = rx.recv().await {
            if ev.name == "paused" {
                let _ = pause.set_checked(ev.data.contains("true"));
            }
        }
    });
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_logging();
    let app = tauri::Builder::default()
        // Must be first: a second launch just focuses the running window.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| show_main_window(app)))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        // The page has no IPC rights, so the plugin's JS link interception must stay off.
        .plugin(tauri_plugin_opener::Builder::new().open_js_links_on_click(false).build())
        .on_window_event(|window, event| {
            // Closing the window keeps LanShare running in the tray / Dock.
            if let WindowEvent::CloseRequested { api, .. } = event
                && window.label() == MAIN
            {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let home_dir = app.path().home_dir()?;
            let bridge = Arc::new(TauriBridge { app: app.handle().clone() });
            let opts = StartOptions { data_dir, home_dir, shared_root: None, port: None, local_port: 0 };
            let server = tauri::async_runtime::block_on(crate::core::start(opts, bridge))?;
            create_window(app.handle(), &server)?;
            create_tray(app, server.state.clone())?;
            app.manage(server);
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to start LanShare");

    app.run(|app, event| match event {
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => show_main_window(app),
        RunEvent::ExitRequested { code: None, api, .. } => api.prevent_exit(),
        _ => {}
    });
}
