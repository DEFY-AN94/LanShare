//! What the desktop shell's menu bar, global shortcut and notification
//! buttons do. The work is here, behind the `HostBridge`, so it is tested
//! without a window; the shell only reads the clipboard and posts results.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::json;
use tokio::sync::OwnedRwLockReadGuard;

use super::auth::Session;
use super::chat;
use super::error::{ApiError, ApiResult};
use super::events::Audience;
use super::model::MessageOut;
use super::paths::{Sandbox, opened_file_is};
use super::state::{AppState, Shared, is_chat_path};
use super::transfer::checked_blocking;

/// The identifier of a notification's own button (`HostBridge::notify_message`).
pub const ACTION_COPY: &str = "copy";
pub const ACTION_OPEN: &str = "open";
/// A click on the notification itself.
pub const ACTION_SHOW: &str = "show";
pub const ACTION_DISMISS: &str = "dismiss";

/// The types 打开 opens directly, in lowercase: the formats phones commonly
/// send. Each opens in its own app, and Office and WPS apply their own checks
/// to files that carry the download mark. Macro-enabled and slideshow formats
/// (docm, xlsm, pptm, xlsb, dotm, pps, ppsx) are left out, because they exist
/// to run macros or start a show.
const OPENABLE: &[&str] = &[
    // Pictures, videos and audio.
    "jpg", "jpeg", "png", "gif", "heic", "heif", "webp", "avif", "bmp", "tif", "tiff", "dng", "mov", "mp4", "m4v", "avi", "mkv", "webm",
    "3gp", "wmv", "mpg", "mpeg", "mp3", "m4a", "aac", "wav", "aif", "aiff", "flac", "ogg", "opus", "wma", "amr", "caf",
    // PDF, text and subtitles; Office, iWork, OpenDocument and WPS documents; e-books.
    "pdf", "txt", "md", "csv", "tsv", "log", "json", "rtf", "srt", "vtt", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "pages", "numbers",
    "key", "odt", "ods", "odp", "wps", "et", "dps", "ofd", "epub",
    // Archives.
    "zip", "rar", "7z", "tar", "gz", "tgz",
];

/// Whether 打开 opens `path` with the default app. A file opens only when its
/// type is on `OPENABLE`; any other file is only shown in its folder. The last
/// extension decides, as it does for the system; on Unix a file marked
/// executable is only shown too.
pub(crate) fn opens_directly(path: &Path) -> bool {
    let ext = path.extension();
    // On a Mac an extension can make a folder an app, installer or disk image.
    if std::fs::metadata(path).is_ok_and(|m| m.is_dir()) {
        return ext.is_none();
    }
    if !ext.and_then(|e| e.to_str()).is_some_and(|e| OPENABLE.contains(&e.to_ascii_lowercase().as_str())) {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(m) = std::fs::metadata(path)
            && m.is_file()
            && m.permissions().mode() & 0o111 != 0
        {
            return false;
        }
    }
    true
}

/// The shell's error text quotes the path or link, which must not reach the log.
pub(crate) fn shell_failed() -> ApiError {
    ApiError::internal("the system could not open the item")
}

/// Opens what `opens_directly` allows with the default app (a plain folder in
/// Finder / File Explorer) and shows anything else in its folder. Returns
/// "opened" or "revealed". `check` was held since `path` was checked
/// (`AppState::path_check`) and is let go once the system has it.
pub(crate) async fn open_or_reveal(state: &Shared, path: PathBuf, check: OwnedRwLockReadGuard<()>) -> ApiResult<&'static str> {
    let bridge = state.bridge.clone();
    // Decided under `check` and off the async workers, since it reads the disk.
    let open = move || if opens_directly(&path) { bridge.open_path(&path).map(|()| "opened") } else { bridge.reveal_path(&path).map(|()| "revealed") };
    checked_blocking(check, open).await?.map_err(|_| shell_failed())
}

/// Where a message's attachment was saved, which may be an earlier shared
/// folder (see `AppState::attachment_path`).
fn attachment(state: &Shared, message_id: i64) -> ApiResult<PathBuf> {
    let msg = state.db.message(message_id)?.ok_or_else(|| ApiError::not_found("这条消息已被删除"))?;
    msg.file_path.as_ref().ok_or_else(|| ApiError::not_found("这条消息没有文件"))?;
    state.attachment_path(&msg).ok_or_else(|| ApiError::not_found("文件已被移动或删除"))
}

/// Opens a message's attachment (only what `opens_directly` allows opens;
/// anything else is revealed).
pub async fn open_attachment(state: &Shared, message_id: i64) -> ApiResult<&'static str> {
    let check = state.path_check().await;
    let path = attachment(state, message_id)?;
    open_or_reveal(state, path, check).await
}

/// Shows a message's attachment in its folder.
pub async fn reveal_attachment(state: &Shared, message_id: i64) -> ApiResult<()> {
    let check = state.path_check().await;
    let path = attachment(state, message_id)?;
    let bridge = state.bridge.clone();
    checked_blocking(check, move || bridge.reveal_path(&path)).await?.map_err(|_| shell_failed())
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

/// Paired devices that can be reached (id, name), the most recent
/// conversation first; one that must pair again is left out.
pub fn recent_devices(state: &Shared) -> Vec<(String, String)> {
    state.db.devices_by_recent(&state.tls.ca_fingerprint()).unwrap_or_else(|e| {
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

/// Why a picture file from the clipboard is not sent.
enum PictureError {
    /// It could not be read: the picture the clipboard also holds may go instead.
    Unreadable(String),
    /// It must not go: nothing goes instead.
    Refused(String),
}

fn unreadable(e: std::io::Error) -> PictureError {
    PictureError::Unreadable(if e.kind() == std::io::ErrorKind::NotFound { "复制的图片已经不在了" } else { "没能读取复制的图片" }.into())
}

/// A picture file from the clipboard, read from the very file that was
/// checked: it is opened first, and a path checked afterwards must lead to
/// it, so a phone's rename (an existing link given its name) cannot swap in
/// another file. One in the shared folder goes out only if phones could
/// download it there anyway, never from a chat folder. The reading itself
/// does not hold up renames.
async fn read_picture(state: &Shared, path: &Path) -> Result<Vec<u8>, PictureError> {
    let (st, path) = (state.clone(), path.to_path_buf());
    let file = checked_blocking(state.path_check().await, move || open_checked_picture(&st, &path))
        .await
        .map_err(|_| PictureError::Unreadable("没能读取复制的图片".into()))??;
    tokio::task::spawn_blocking(move || {
        let mut bytes = Vec::new();
        let mut file = file;
        file.read_to_end(&mut bytes).map(|_| bytes)
    })
    .await
    .map_err(|_| PictureError::Unreadable("没能读取复制的图片".into()))?
    .map_err(unreadable)
}

fn open_checked_picture(state: &AppState, path: &Path) -> Result<std::fs::File, PictureError> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    // A pipe given the picture's name would block the open: refused below instead.
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::custom_flags(&mut options, libc::O_NONBLOCK);
    let file = options.open(path).map_err(unreadable)?;
    let meta = file.metadata().map_err(unreadable)?;
    if !meta.is_file() {
        return Err(PictureError::Refused(FILES_NOT_SENT.into()));
    }
    if meta.len() > MAX_CLIPBOARD_PICTURE_BYTES {
        return Err(PictureError::Refused("图片太大，请把它拖进 LanShare 的聊天窗口发送".into()));
    }
    let sandbox = state.sandbox();
    // Where the copied name itself is: a path through a link elsewhere may still lead into the shared folder.
    let at = path.parent().and_then(|p| dunce::canonicalize(p).ok()).zip(path.file_name()).map(|(d, n)| d.join(n));
    let named_in_share = sandbox.relative(path).is_some() || at.as_deref().is_some_and(|a| sandbox.relative(a).is_some());
    match dunce::canonicalize(path) {
        Ok(real) => {
            if !opened_file_is(&file, &real) {
                return Err(PictureError::Refused("复制的图片刚被改动，没有发送，请重新复制".into()));
            }
            if (named_in_share || sandbox.relative(&real).is_some()) && !phones_can_download(state, &sandbox, &real) {
                return Err(PictureError::Refused("这张图片在共享文件夹里手机打不开的位置（例如聊天文件夹），没有发送".into()));
            }
        }
        // Some Windows volumes cannot name an open file's final path: fine
        // for a picture from outside the shared folder, which phones cannot touch.
        Err(_) if !named_in_share => {}
        Err(e) => return Err(unreadable(e)),
    }
    Ok(file)
}

/// Whether a phone browsing the shared folder could download `real`.
fn phones_can_download(state: &AppState, sandbox: &Sandbox, real: &Path) -> bool {
    let Some(rel) = sandbox.relative(real) else { return false };
    !is_chat_path(&rel) && sandbox.resolve_file(&rel).is_ok_and(|p| p == real && !state.is_chat_location(&p))
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
        let (name, bytes) = match read_picture(state, &path).await {
            Ok(bytes) => (path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(picture_name), bytes),
            Err(PictureError::Refused(e)) => return Err(e),
            Err(PictureError::Unreadable(e)) => match image.take().filter(|_| single) {
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
        Some(id) => match state.db.device(id).ok().flatten() {
            Some(d) if state.needs_repair(&d) => return Err(chat::NEEDS_REPAIR.into()),
            Some(d) => (d.id, d.name),
            None => return Err("这台设备已经不在了".into()),
        },
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
            // The button says 打开, so say why only the folder came up.
            match result {
                Ok("revealed") => tell(state, "为了安全，这种文件不会直接打开，已在文件夹中显示", false),
                other => report(state, other),
            }
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
    use super::{OPENABLE, check_shortcut, opens_directly};
    use std::path::Path;

    /// Types the system would run, mount or install rather than show. None
    /// of them may ever open directly.
    const RUNS_OR_INSTALLS: &[&str] = &[
        "app", "command", "terminal", "tool", "sh", "bash", "zsh", "csh", "fish", "pkg", "mpkg", "jar", "py", "rb", "pl", "php", "scpt",
        "scptd", "applescript", "workflow", "action", "fileloc", "inetloc", "webloc", "prefpane", "kext", "plugin", "osax", "dylib", "exe",
        "bat", "cmd", "com", "scr", "msi", "msp", "ps1", "psm1", "vbs", "vbe", "js", "jse", "wsf", "wsh", "hta", "lnk", "url", "reg",
        "cpl", "msc", "appref-ms", "settingcontent-ms", "library-ms", "inf", "pif", "application", "gadget", "desktop", "appimage",
        // Windows help files can run script; packages install; disk images mount.
        "chm", "msix", "msixbundle", "appx", "appxbundle", "appinstaller", "iso", "img", "vhd", "vhdx",
        // More Windows types that run, install or reach out when opened.
        "pyw", "pyz", "pyzw", "msu", "ppkg", "searchconnector-ms", "search-ms", "website", "scf", "theme", "themepack", "deskthemepack",
        "diagcab",
        // macOS disk images mount; profiles, apps, screen savers and shortcuts install; Java Web Start runs.
        "dmg", "udif", "dmgpart", "asif", "toast", "dvdr", "cdr", "sparseimage", "sparsebundle", "backupbundle", "mobileconfig",
        "configprofile", "provisionprofile", "ipa", "saver", "shortcut", "jnlp",
        // Certificates and keychains import on both systems; server locations connect out.
        "cer", "crt", "cert", "der", "pem", "p7b", "p7c", "p12", "pkcs12", "pfx", "keychain", "keychain-db", "vncloc", "afploc",
        "ftploc", "atloc",
        // Windows remote desktop files connect out; sandbox files run a command; the rest run or install.
        "rdp", "wsb", "xll", "wsc", "sct", "shb", "shs", "psc1", "msh", "mst", "xbap",
    ];

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
        for ext in RUNS_OR_INSTALLS {
            for name in [format!("x.{ext}"), format!("X.{}", ext.to_ascii_uppercase())] {
                assert!(!opens_directly(Path::new(&name)), "{name}");
            }
        }
    }

    #[test]
    fn a_type_not_on_the_list_is_only_revealed() {
        // Unknown types, a trailing dot or space, a look-alike letter, an
        // invisible character, the last of two extensions, a web page and a
        // macro-enabled document.
        for name in ["model.blend", "x.abc", "x.exe.", "x.exe ", "x.\u{435}x\u{435}", "x.jpg\u{200b}", "photo.jpg.exe", "page.html", "report.docm"] {
            assert!(!opens_directly(Path::new(name)), "{name:?}");
        }
    }

    #[test]
    fn documents_media_and_archives_open() {
        for name in [
            "photo.jpg", "IMG_0001.HEIC", "clip.MOV", "voice.m4a", "doc.pdf", "notes.txt", "report.docx", "Budget.XLSX", "slides.key",
            "archive.zip", "backup.tar.gz", "x.exe.jpg",
        ] {
            assert!(opens_directly(Path::new(name)), "{name}");
        }
        for ext in OPENABLE {
            for name in [format!("x.{ext}"), format!("X.{}", ext.to_ascii_uppercase())] {
                assert!(opens_directly(Path::new(&name)), "{name}");
            }
        }
    }

    #[test]
    fn the_open_list_can_match_and_holds_nothing_that_runs() {
        for (i, ext) in OPENABLE.iter().enumerate() {
            // The lookup lowercases the extension, so anything else would never match.
            assert!(!ext.is_empty() && ext.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()), "{ext}");
            assert!(!OPENABLE[..i].contains(ext), "{ext} twice");
            assert!(!RUNS_OR_INSTALLS.contains(ext), "{ext}");
        }
    }

    #[test]
    fn a_folder_opens_unless_its_name_has_an_extension() {
        // A phone can build an app or a disk image this way with mkdir and
        // uploads, and `open` would run or mount it.
        let dir = tempfile::tempdir().unwrap();
        for (name, opens) in [("Photos", true), ("Disk.sparsebundle", false), ("Tool.app", false), ("photos.zip", false), ("v1.2", false)] {
            std::fs::create_dir(dir.path().join(name)).unwrap();
            assert_eq!(opens_directly(&dir.path().join(name)), opens, "{name}");
        }
        // A file with no extension is only revealed, whether or not it exists.
        std::fs::write(dir.path().join("tool"), "#!/bin/sh\n").unwrap();
        assert!(!opens_directly(&dir.path().join("tool")));
        assert!(!opens_directly(&dir.path().join("notes")));
    }

    #[cfg(unix)]
    #[test]
    fn a_file_marked_executable_is_only_revealed() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.txt");
        std::fs::write(&file, "hi").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(opens_directly(&file));
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!opens_directly(&file));
    }
}
