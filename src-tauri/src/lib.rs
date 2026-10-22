//! Desktop shell: starts the server, shows it in a window, adds a tray icon.

mod clipboard;
pub mod core;
#[cfg(target_os = "macos")]
mod login_item;
#[cfg(windows)]
mod login_item_windows;
#[cfg(target_os = "macos")]
mod mac_notify;
mod plugin_notify;
mod shortcut;
mod tray;

use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(target_os = "macos")]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicU32, Ordering};

use tauri::webview::{DownloadEvent, NewWindowResponse};
use tauri::{AppHandle, Manager, RunEvent, Url, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::core::bridge::{Clip, HostBridge, MessageNotice, Withdraw};
use crate::core::{ServerHandle, StartOptions};

const MAIN: &str = "main";

pub fn init_logging() {
    crate::core::logging::init();
}

/// Command-line flag for a start without a window (also for the Windows Run key).
const BACKGROUND_ARG: &str = "--background";

/// Why LanShare starts with only its menu bar / tray icon, or `None` for a
/// normal start. On macOS this must run at the top of `setup`, while the
/// launch's Apple event is still current.
fn hidden_start_reason() -> Option<&'static str> {
    if std::env::args().any(|a| a == BACKGROUND_ARG) {
        return Some(BACKGROUND_ARG);
    }
    #[cfg(target_os = "macos")]
    {
        if login_item::launched_at_login() {
            return Some("login item");
        }
        if login_item::opened_soon_after_login() {
            return Some("soon after login");
        }
    }
    None
}

/// LanShare has a Dock icon (macOS). After a hidden start it has none until
/// the window opens, and the unread count shows next to the menu bar icon.
#[cfg(target_os = "macos")]
static DOCK_VISIBLE: AtomicBool = AtomicBool::new(true);
static UNREAD: AtomicU32 = AtomicU32::new(0);

fn show_main_window(app: &AppHandle) {
    // After a hidden start the app has no Dock icon and is hidden.
    #[cfg(target_os = "macos")]
    {
        let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
        let _ = app.show();
        if !DOCK_VISIBLE.swap(true, Ordering::SeqCst) {
            // The count moves from the menu bar to the Dock icon.
            show_unread(app, UNREAD.load(Ordering::SeqCst));
        }
    }
    if let Some(w) = app.get_webview_window(MAIN) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn show_unread(app: &AppHandle, total: u32) {
    #[cfg(target_os = "macos")]
    {
        if let Some(w) = app.get_webview_window(MAIN) {
            let _ = w.set_badge_count((total > 0).then_some(i64::from(total)));
        }
        if let Some(tray) = app.tray_by_id(tray::TRAY_ID) {
            let title = (total > 0 && !DOCK_VISIBLE.load(Ordering::SeqCst)).then(|| total.to_string());
            let _ = tray.set_title(title);
        }
    }
    #[cfg(not(target_os = "macos"))]
    if let Some(tray) = app.tray_by_id(tray::TRAY_ID) {
        let tip = if total > 0 { format!("LanShare · {total} 条未读") } else { "LanShare".to_string() };
        let _ = tray.set_tooltip(Some(tip));
    }
}

struct TauriBridge {
    app: AppHandle,
}

impl HostBridge for TauriBridge {
    fn notify(&self, title: &str, body: &str) {
        // No banner while the user is already looking at the window.
        if self.window_focused() {
            return;
        }
        #[cfg(target_os = "macos")]
        if mac_notify::post_plain(title, body) {
            return;
        }
        plugin_notify::plain(&self.app, title, body);
    }

    fn notify_message(&self, n: &MessageNotice) {
        #[cfg(target_os = "macos")]
        if mac_notify::post(n) {
            return;
        }
        #[cfg(windows)]
        {
            plugin_notify::message(&self.app, n);
        }
        #[cfg(not(windows))]
        if !n.quiet {
            plugin_notify::plain(&self.app, &n.title, &n.body);
        }
    }

    fn quiet_updates(&self) -> bool {
        // Windows toasts cannot be updated in place here (plugin_notify::message).
        #[cfg(target_os = "macos")]
        return mac_notify::quiet_updates();
        #[cfg(not(target_os = "macos"))]
        false
    }

    fn withdraw_notifications(&self, _w: &Withdraw) {
        #[cfg(target_os = "macos")]
        mac_notify::withdraw(_w);
    }

    fn window_focused(&self) -> bool {
        self.app
            .get_webview_window(MAIN)
            .map(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false))
            .unwrap_or(false)
    }

    fn set_unread(&self, total: u32) {
        UNREAD.store(total, Ordering::SeqCst);
        show_unread(&self.app, total);
    }

    fn show_conversation(&self, device_id: &str) {
        // The conversation first, then the window: what was open before
        // never comes to the front, where it would count as read.
        if let Some(w) = self.app.get_webview_window(MAIN) {
            // JSON keeps the id a plain string literal in the script.
            let hash = serde_json::to_string(&format!("#/chat/{device_id}")).unwrap_or_default();
            let _ = w.eval(format!("location.hash = {hash}"));
        }
        show_main_window(&self.app);
    }

    fn notifications_state(&self) -> &'static str {
        #[cfg(target_os = "macos")]
        return mac_notify::state();
        #[cfg(not(target_os = "macos"))]
        "unknown"
    }

    fn read_clipboard(&self) -> Clip {
        clipboard::read()
    }

    fn write_clipboard_text(&self, text: &str) -> Result<(), String> {
        clipboard::write_text(text)
    }

    fn set_shortcut(&self, accelerator: Option<&str>) -> Result<(), String> {
        shortcut::set(&self.app, accelerator)
    }

    fn open_path(&self, path: &Path) -> Result<(), String> {
        let path = crate::core::paths::shell_path(path);
        self.app.opener().open_path(path.to_string_lossy(), None::<&str>).map_err(|e| e.to_string())
    }

    fn reveal_path(&self, path: &Path) -> Result<(), String> {
        self.app.opener().reveal_item_in_dir(crate::core::paths::shell_path(path)).map_err(|e| e.to_string())
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

    #[cfg(target_os = "macos")]
    fn login_item(&self) -> &'static str {
        let state = login_item::state();
        // The user may have changed it in System Settings since the last look.
        login_item::sync_relaunch_on_login(state == login_item::LoginItem::On);
        state.as_str()
    }

    #[cfg(target_os = "macos")]
    fn set_login_item(&self, on: bool) -> Result<&'static str, String> {
        let result = login_item::set(on);
        tracing::info!("launch at login set to {on}: {result:?}; {}", login_item::diagnostics());
        result.map(login_item::LoginItem::as_str)
    }

    #[cfg(target_os = "macos")]
    fn open_login_item_settings(&self) {
        login_item::open_system_settings();
    }

    #[cfg(windows)]
    fn login_item(&self) -> &'static str {
        login_item_windows::state()
    }

    #[cfg(windows)]
    fn set_login_item(&self, on: bool) -> Result<&'static str, String> {
        let result = login_item_windows::set(on);
        tracing::info!("launch at login set to {on}: {}", if result.is_ok() { "ok" } else { "failed" });
        result
    }

    #[cfg(windows)]
    fn open_login_item_settings(&self) {
        let _ = self.app.opener().open_url(login_item_windows::settings_url(), None::<&str>);
    }
}

fn create_window(app: &AppHandle, server: &ServerHandle, visible: bool) -> tauri::Result<()> {
    let url: Url = server.login_url().parse().expect("valid loopback url");
    let origin = url.origin();
    let nav_app = app.clone();
    let popup_app = app.clone();
    WebviewWindowBuilder::new(app, MAIN, WebviewUrl::External(url))
        .title("LanShare")
        // Hidden after a login launch; the page still loads, so opening it later is instant.
        .visible(visible)
        .inner_size(1080.0, 720.0)
        .min_inner_size(820.0, 560.0)
        .center()
        // Shrink to the monitor's work area (e.g. 1080p laptops at 150% on Windows).
        .prevent_overflow()
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_logging();
    let app = tauri::Builder::default()
        // Must be first: a second launch just focuses the running window.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if !argv.iter().any(|a| a == BACKGROUND_ARG) {
                show_main_window(app);
            }
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
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
            // First: on macOS the login-item flag is only readable right now.
            let hidden_reason = hidden_start_reason();
            let hidden = hidden_reason.is_some();
            // Before the server starts: a click on a notification that relaunched
            // LanShare is delivered right after this returns, and waits until then.
            #[cfg(target_os = "macos")]
            mac_notify::install();
            #[cfg(target_os = "macos")]
            let launch_event = login_item::launch_event_text();
            let data_dir = app.path().app_data_dir()?;
            let home_dir = app.path().home_dir()?;
            let bridge = Arc::new(TauriBridge { app: app.handle().clone() });
            let opts = StartOptions { data_dir, home_dir, shared_root: None, port: None, local_port: 0, upload_reserve: None };
            let server = tauri::async_runtime::block_on(crate::core::start(opts, bridge))?;
            // Before the window can ask: an upgrade's uninstaller removed launch at login.
            #[cfg(windows)]
            if let Some(stamp) = login_item_windows::install_stamp() {
                let installed_since = server.state.note_install(&stamp);
                if server.state.login_item_chosen() && !login_item_windows::restore(installed_since) {
                    server.state.set_login_item_chosen(false);
                }
            }
            create_window(app.handle(), &server, !hidden)?;
            tray::create(app, server.state.clone())?;
            let state = server.state.clone();
            #[cfg(target_os = "macos")]
            {
                let st = state.clone();
                mac_notify::set_handler(move |r| {
                    let st = st.clone();
                    tauri::async_runtime::spawn(async move { core::quick::on_notification(&st, r.action, r.message_id, r.device_id.as_deref()).await });
                });
            }
            plugin_notify::on_action(app.handle(), state.clone());
            // The saved shortcut; the plugin works on the main thread, so not from here.
            let (st, handle) = (state.clone(), app.handle().clone());
            tauri::async_runtime::spawn_blocking(move || {
                if let Some(saved) = st.clipboard_shortcut()
                    && let Err(e) = shortcut::set(&handle, Some(&saved))
                {
                    tracing::warn!("the saved global shortcut could not be registered");
                    st.set_shortcut_error(Some(e));
                }
                st.unread_changed();
            });
            #[cfg(target_os = "macos")]
            {
                if hidden {
                    // Menu bar icon only: no Dock icon until the window opens,
                    // and the app does not keep focus at login.
                    DOCK_VISIBLE.store(false, Ordering::SeqCst);
                    let _ = app.handle().set_activation_policy(tauri::ActivationPolicy::Accessory);
                    let _ = app.handle().hide();
                }
                tauri::async_runtime::spawn_blocking(move || {
                    let state = login_item::state();
                    login_item::sync_relaunch_on_login(state == login_item::LoginItem::On);
                    // e.g. "launch at login: on; hidden start: Some("login item"); launch event: aevt/oapp prdt=lgit"
                    tracing::info!(
                        "launch at login: {}; hidden start: {hidden_reason:?}; launch event: {launch_event}; {}",
                        state.as_str(),
                        login_item::diagnostics()
                    );
                });
            }
            app.manage(server);
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to start LanShare");

    // `_app` is only needed for the macOS Dock "reopen" event.
    app.run(|_app, event| match event {
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => show_main_window(_app),
        RunEvent::ExitRequested { code: None, api, .. } => api.prevent_exit(),
        _ => {}
    });
}
