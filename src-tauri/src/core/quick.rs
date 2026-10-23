//! What the desktop shell's menu bar, global shortcut and notification
//! buttons do. The work is here, behind the `HostBridge`, so it is tested
//! without a window; the shell only reads the clipboard and posts results.

use std::path::{Path, PathBuf};

use serde_json::json;

use super::auth::Session;
use super::chat;
use super::error::{ApiError, ApiResult};
use super::events::Audience;
use super::model::MessageOut;
use super::state::Shared;

/// The identifier of a notification's own button (`HostBridge::notify_message`).
pub const ACTION_COPY: &str = "copy";
pub const ACTION_OPEN: &str = "open";
/// A click on the notification itself.
pub const ACTION_SHOW: &str = "show";
pub const ACTION_DISMISS: &str = "dismiss";

/// Things the OS would *run*, mount or install rather than show (apps, scripts,
/// installers, disk images, profiles, shortcuts). These are only revealed.
pub(crate) fn is_launchable(path: &Path) -> bool {
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
        // macOS disk images mount; profiles, certificates, screen savers and shortcuts install; Java Web Start runs.
        "dmg", "sparseimage", "sparsebundle", "cdr", "smi", "mobileconfig", "provisionprofile", "cer", "crt", "p12", "pfx", "saver",
        "shortcut", "jnlp",
        // Windows remote desktop files connect out; the rest run or install.
        "rdp", "xll", "wsc", "sct", "shb", "shs", "psc1", "msh", "mst", "xbap",
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
pub(crate) fn shell_failed() -> ApiError {
    ApiError::internal("the system could not open the item")
}

/// Opens with the default app; launchable files are revealed in the folder
/// instead. Returns "opened" or "revealed".
pub(crate) async fn open_or_reveal(state: &Shared, path: PathBuf) -> ApiResult<&'static str> {
    let bridge = state.bridge.clone();
    // Covers app bundles too (directories named *.app).
    let reveal = is_launchable(&path);
    tokio::task::spawn_blocking(move || if reveal { bridge.reveal_path(&path) } else { bridge.open_path(&path) })
        .await
        .map_err(ApiError::internal)?
        .map_err(|_| shell_failed())?;
    Ok(if reveal { "revealed" } else { "opened" })
}

/// Where a message's attachment was saved, which may be an earlier shared
/// folder (see `AppState::attachment_path`).
fn attachment(state: &Shared, message_id: i64) -> ApiResult<PathBuf> {
    let msg = state.db.message(message_id)?.ok_or_else(|| ApiError::not_found("这条消息已被删除"))?;
    msg.file_path.as_ref().ok_or_else(|| ApiError::not_found("这条消息没有文件"))?;
    state.attachment_path(&msg).ok_or_else(|| ApiError::not_found("文件已被移动或删除"))
}

/// Opens a message's attachment (launchable files are revealed instead).
pub async fn open_attachment(state: &Shared, message_id: i64) -> ApiResult<&'static str> {
    let path = attachment(state, message_id)?;
    open_or_reveal(state, path).await
}

/// Shows a message's attachment in its folder.
pub async fn reveal_attachment(state: &Shared, message_id: i64) -> ApiResult<()> {
    let path = attachment(state, message_id)?;
    let bridge = state.bridge.clone();
    tokio::task::spawn_blocking(move || bridge.reveal_path(&path)).await.map_err(ApiError::internal)?.map_err(|_| shell_failed())
}

/// Puts a message's full text on the clipboard (a notification shows 80 characters).
pub fn copy_message(state: &Shared, message_id: i64) -> Result<(), String> {
    let msg = state.db.message(message_id).map_err(|e| e.to_string())?.ok_or("这条消息已被删除")?;
    let text = msg.text.filter(|_| msg.kind == "text").ok_or("这条消息不是文字")?;
    state.bridge.write_clipboard_text(&text)
}

/// Puts the newest text any phone sent on the clipboard; returns that phone's name.
pub fn copy_latest_received(state: &Shared) -> Result<String, String> {
    let msg = state.db.latest_received_text().map_err(|e| e.to_string())?.ok_or("还没有收到过文字消息")?;
    let text = msg.text.unwrap_or_default();
    state.bridge.write_clipboard_text(&text)?;
    Ok(state.db.device(&msg.device_id).ok().flatten().map(|d| d.name).unwrap_or_default())
}

/// Paired devices (id, name), the most recent conversation first.
pub fn recent_devices(state: &Shared) -> Vec<(String, String)> {
    state.db.devices_by_recent().unwrap_or_else(|e| {
        tracing::warn!("could not list devices: {e}");
        Vec::new()
    })
}

/// Where the clipboard goes: the device chosen in Settings while it is
/// still paired, otherwise the most recent conversation.
pub fn clipboard_target(state: &Shared) -> Option<(String, String)> {
    let devices = recent_devices(state);
    let chosen = state.clipboard_target();
    chosen.and_then(|id| devices.iter().find(|(d, _)| *d == id).cloned()).or_else(|| devices.into_iter().next())
}

/// What the clipboard may name as files and still be sent: pictures. Other
/// files go through the chat window (drag and drop), which streams them.
const PICTURE_TYPES: &[&str] = &["png", "jpg", "jpeg", "heic", "heif", "gif", "webp", "tif", "tiff", "bmp"];
const MAX_CLIPBOARD_PICTURES: usize = 20;
const MAX_CLIPBOARD_PICTURE_BYTES: u64 = 100 * 1024 * 1024;
const FILES_NOT_SENT: &str = "剪贴板里是文件：请把文件拖进 LanShare 的聊天窗口发送";

fn is_picture(path: &Path) -> bool {
    path.extension().is_some_and(|e| PICTURE_TYPES.contains(&e.to_string_lossy().to_ascii_lowercase().as_str()))
}

async fn read_picture(path: &Path) -> Result<Vec<u8>, String> {
    let meta = tokio::fs::metadata(path).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound { "复制的图片已经不在了" } else { "没能读取复制的图片" }.to_string()
    })?;
    if !meta.is_file() {
        return Err(FILES_NOT_SENT.into());
    }
    if meta.len() > MAX_CLIPBOARD_PICTURE_BYTES {
        return Err("图片太大，请把它拖进 LanShare 的聊天窗口发送".into());
    }
    tokio::fs::read(path).await.map_err(|_| "没能读取复制的图片".to_string())
}

/// Sends picture files from the clipboard as they are (a picture copied in
/// QQ or WeChat, or pictures copied in Finder); returns the last message. A
/// single picture LanShare may not read goes as the PNG the clipboard holds.
async fn send_picture_files(state: &Shared, device_id: &str, paths: Vec<PathBuf>, mut image: Option<Vec<u8>>) -> Result<MessageOut, String> {
    if paths.is_empty() || !paths.iter().all(|p| is_picture(p)) {
        return Err(FILES_NOT_SENT.into());
    }
    if paths.len() > MAX_CLIPBOARD_PICTURES {
        return Err(format!("一次最多发送 {MAX_CLIPBOARD_PICTURES} 张图片"));
    }
    let single = paths.len() == 1;
    let mut last = None;
    for path in paths {
        let (name, bytes) = match read_picture(&path).await {
            Ok(bytes) => (path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(picture_name), bytes),
            Err(e) => match image.take().filter(|_| single) {
                Some(png) => (path.file_stem().map(|s| format!("{}.png", s.to_string_lossy())).unwrap_or_else(picture_name), png),
                None => return Err(e),
            },
        };
        last = Some(chat::send_bytes(state, device_id, &name, bytes).await.map_err(|e| e.message)?);
    }
    last.ok_or_else(|| FILES_NOT_SENT.to_string())
}

/// A name for a picture from the clipboard, like 截图 2026-10-22 14.03.05.png.
fn picture_name() -> String {
    format!("截图 {}.png", chrono::Local::now().format("%Y-%m-%d %H.%M.%S"))
}

/// Sends what is on the clipboard to `device_id` (or the clipboard target) as
/// the computer: text as a message, a picture as a PNG in 已发送, picture
/// files as they are. Reads the clipboard only now. Returns the device's name
/// and the (last) message.
pub async fn send_clipboard(state: &Shared, device_id: Option<&str>) -> Result<(String, MessageOut), String> {
    let (id, name) = match device_id {
        Some(id) => state.db.device(id).ok().flatten().map(|d| (d.id, d.name)).ok_or("这台设备已经不在了")?,
        None => clipboard_target(state).ok_or("还没有配对的设备")?,
    };
    let bridge = state.bridge.clone();
    let clip = tokio::task::spawn_blocking(move || bridge.read_clipboard()).await.map_err(|e| e.to_string())?;
    let sent = match clip {
        super::bridge::Clip::Empty => return Err("剪贴板是空的，没有发送".into()),
        super::bridge::Clip::Unsupported(why) => return Err(why),
        super::bridge::Clip::Files { paths, image } => return send_picture_files(state, &id, paths, image).await.map(|m| (name, m)),
        super::bridge::Clip::Text(text) => chat::send_text(state, &Session::Host, Some(&id), None, &text),
        super::bridge::Clip::Png(bytes) => chat::send_bytes(state, &id, &picture_name(), bytes).await,
    };
    // Kinds and lengths only: the clipboard's content never reaches the log.
    sent.map(|m| (name, m)).map_err(|e| e.message)
}

/// Says how a menu, shortcut or notification action went: in the window
/// when it is in front (a notification would not show there), otherwise as
/// a notification.
pub fn tell(state: &Shared, text: &str, error: bool) {
    if state.bridge.window_focused() {
        state.emit(Audience::Host, "notice", &json!({ "text": text, "error": error }));
    } else {
        state.bridge.notify("LanShare", text);
    }
}

/// Shows an error from a menu, shortcut or notification action.
pub fn report<T>(state: &Shared, result: Result<T, String>) {
    if let Err(msg) = result {
        tell(state, &msg, true);
    }
}

/// Sends the clipboard and says how it went.
pub async fn send_clipboard_and_tell(state: &Shared, device_id: Option<&str>) {
    match send_clipboard(state, device_id).await {
        Ok((name, msg)) => {
            let what = if msg.kind == "file" { "图片" } else { "文字" };
            tell(state, &format!("已把剪贴板里的{what}发送到「{name}」"), false);
        }
        Err(e) => tell(state, &e, true),
    }
}

/// 复制最近收到的文字 from the menu.
pub fn copy_latest_and_tell(state: &Shared) {
    match copy_latest_received(state) {
        Ok(name) => tell(state, &format!("已复制「{name}」发来的文字"), false),
        Err(e) => tell(state, &e, true),
    }
}

/// The global shortcut was pressed. While Settings records a new one, this
/// is the current shortcut pressed again: Settings hears it, nothing is sent.
pub async fn shortcut_pressed(state: &Shared) {
    if state.shortcut_recording() {
        state.emit(Audience::Host, "shortcut_pressed", &json!({}));
        return;
    }
    send_clipboard_and_tell(state, None).await;
}

const MODIFIERS: [&str; 4] = ["Control", "Alt", "Shift", "Super"];

/// Combinations the system itself uses (screenshots, lock screen, emoji).
const SYSTEM_SHORTCUTS: &[&str] = &[
    "Shift+Super+Digit3",
    "Shift+Super+Digit4",
    "Shift+Super+Digit5",
    "Shift+Super+Digit6",
    "Control+Shift+Super+Digit3",
    "Control+Shift+Super+Digit4",
    "Control+Super+KeyQ",
    "Control+Super+Space",
    "Control+Super+KeyF",
    "Shift+Super+KeyS",
];

/// Checks a shortcut as Settings records it ("Control+Alt+KeyV"): at least
/// two modifiers, so a slip such as ⌘V or Ctrl+C can never take over pasting
/// or copying in every app, and one of them Control or ⌘ / Win, so it never
/// takes a character away from typing: Option(+Shift) types symbols on a
/// Mac, and on Windows Ctrl+Alt is AltGr, which types them on many layouts.
pub fn check_shortcut(accelerator: &str, windows: bool) -> Result<(), &'static str> {
    let parts: Vec<&str> = accelerator.split('+').collect();
    let (key, modifiers) = parts.split_last().ok_or("快捷键无效")?;
    let known = modifiers.iter().all(|m| MODIFIERS.contains(m));
    let distinct = modifiers.iter().enumerate().all(|(i, m)| !modifiers[..i].contains(m));
    let key_ok = key.len() <= 16 && !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric());
    if !known || !distinct || !key_ok || MODIFIERS.contains(key) {
        return Err("快捷键无效");
    }
    if modifiers.len() < 2 {
        return Err("快捷键至少要包含两个修饰键");
    }
    let has = |m: &str| modifiers.contains(&m);
    if !has("Control") && !has("Super") {
        return Err(if windows { "快捷键要包含 Ctrl 或 Win" } else { "快捷键要包含 ⌃ 或 ⌘" });
    }
    if windows && has("Control") && has("Alt") && !has("Super") {
        return Err("Ctrl+Alt 在 Windows 上等同于 AltGr，会挡住打字，请换一个组合");
    }
    let mut sorted = modifiers.to_vec();
    sorted.sort_by_key(|m| MODIFIERS.iter().position(|x| x == m));
    let normal = format!("{}+{key}", sorted.join("+"));
    if SYSTEM_SHORTCUTS.contains(&normal.as_str()) {
        return Err("这个组合是系统自己的快捷键，请换一个");
    }
    Ok(())
}

/// A notification's button or a click on it (`action`), for the message it showed.
pub async fn on_notification(state: &Shared, action: &str, message_id: Option<i64>, device_id: Option<&str>) {
    match action {
        ACTION_DISMISS => {}
        ACTION_COPY => report(state, message_id.ok_or_else(|| "这条消息已被删除".to_string()).and_then(|id| copy_message(state, id))),
        ACTION_OPEN => {
            let result = match message_id {
                Some(id) => open_attachment(state, id).await.map_err(|e| e.message),
                None => Err("这条消息已被删除".into()),
            };
            report(state, result);
        }
        _ => {
            let device = device_id
                .map(str::to_string)
                .or_else(|| message_id.and_then(|id| state.db.message(id).ok().flatten()).map(|m| m.device_id));
            match device.filter(|d| state.db.device(d).ok().flatten().is_some()) {
                Some(d) => state.bridge.show_conversation(&d),
                None => state.bridge.show_window(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{check_shortcut, is_launchable};
    use std::path::Path;

    #[test]
    fn shortcuts_need_two_modifiers_and_stay_off_the_system_ones() {
        for ok in ["Control+Alt+KeyV", "Control+Alt+Shift+Super+KeyV", "Alt+Super+Digit5", "Control+Shift+F9", "Super+Alt+KeyC"] {
            assert_eq!(check_shortcut(ok, false), Ok(()), "{ok}");
        }
        for slip in ["Super+KeyV", "Control+KeyC", "Alt+F4", "Super+KeyQ", "KeyV"] {
            assert_eq!(check_shortcut(slip, false), Err("快捷键至少要包含两个修饰键"), "{slip}");
        }
        assert!(check_shortcut("Super+Shift+Digit4", false).is_err(), "screenshots");
        assert!(check_shortcut("Super+Control+KeyQ", false).is_err(), "lock screen, in either order");
        for bad in ["", "+", "Control+Alt+", "Control+Control+KeyV", "Ctrl+Alt+KeyV", "Control+Alt+Shift", "Control+Alt+Key V"] {
            assert_eq!(check_shortcut(bad, false), Err("快捷键无效"), "{bad:?}");
        }
    }

    #[test]
    fn shortcuts_never_take_a_character_away_from_typing() {
        assert_eq!(check_shortcut("Alt+Shift+Digit7", false), Err("快捷键要包含 ⌃ 或 ⌘"), "⌥⇧7 types a symbol on a Mac");
        assert_eq!(check_shortcut("Alt+Shift+KeyV", true), Err("快捷键要包含 Ctrl 或 Win"));
        for altgr in ["Control+Alt+KeyV", "Control+Alt+Shift+KeyE"] {
            assert!(check_shortcut(altgr, true).unwrap_err().contains("AltGr"), "{altgr}");
        }
        for ok in ["Control+Shift+KeyV", "Control+Alt+Super+KeyV", "Shift+Super+KeyY"] {
            assert_eq!(check_shortcut(ok, true), Ok(()), "{ok}");
        }
    }

    #[test]
    fn files_that_run_or_mount_are_only_revealed() {
        for name in [
            "setup.exe", "run.ps1", "link.lnk", "help.chm", "HELP.CHM", "app.msix", "pkg.appx", "x.appinstaller", "disk.iso", "disk.img",
            "disk.vhd", "disk.vhdx", "Tool.app", "x.sh", "x.pyw", "fix.diagcab", "update.msu", "x.scf", "dark.theme", "disk.dmg",
            "DISK.DMG", "wifi.mobileconfig", "x.sparseimage", "ca.crt", "id.p12", "x.saver", "x.jnlp", "x.rdp",
        ] {
            assert!(is_launchable(Path::new(name)), "{name}");
        }
        for name in ["photo.jpg", "doc.pdf", "notes.txt", "movie.mov", "archive.zip"] {
            assert!(!is_launchable(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn a_disk_image_folder_is_only_revealed() {
        // A phone can build one with mkdir and uploads; `open` would mount it.
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("Disk.sparsebundle");
        std::fs::create_dir(&bundle).unwrap();
        assert!(is_launchable(&bundle));
    }
}
