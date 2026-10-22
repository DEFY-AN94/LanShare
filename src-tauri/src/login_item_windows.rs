//! Launch at login on Windows: the value `LanShare` under
//! HKCU\Software\Microsoft\Windows\CurrentVersion\Run, with the path in
//! quotes and `--background`, so LanShare starts in the tray. Task Manager's
//! Startup apps page can turn it off (StartupApproved); Settings then says so.
//!
//! Tauri's uninstaller removes the Run value, also when an upgrade or a
//! reinstall runs it first; the first start after an install puts it back
//! (`restore`). At any other start a missing value was removed on purpose
//! and stays removed.
//! Only the installed copy (with uninstall.exe beside it) sets the value, so
//! a development build never takes it over.

use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_BINARY, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW};

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
const NAME: &str = "LanShare";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// What the Run value should hold for this copy of LanShare.
fn command() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!("\"{}\" {}", exe.display(), crate::BACKGROUND_ARG))
}

fn uninstaller() -> Option<std::path::PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.join("uninstall.exe"))
}

/// This copy was installed by LanShare's installer.
fn installed() -> bool {
    uninstaller().is_some_and(|u| u.is_file())
}

/// Changes with every install (the installer writes a new uninstall.exe);
/// `None` for a copy that was not installed.
pub fn install_stamp() -> Option<String> {
    let modified = std::fs::metadata(uninstaller()?).ok()?.modified().ok()?;
    let ms = modified.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis();
    Some(ms.to_string())
}

fn run_value() -> Option<String> {
    let (sub, name) = (wide(RUN), wide(NAME));
    let mut len: u32 = 0;
    // SAFETY: NUL-terminated strings; a null buffer asks for the size.
    let rc = unsafe { RegGetValueW(HKEY_CURRENT_USER, sub.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, null_mut(), null_mut(), &mut len) };
    if rc != ERROR_SUCCESS || len == 0 {
        return None;
    }
    let mut buf = vec![0u16; (len as usize).div_ceil(2)];
    // SAFETY: `buf` holds `len` bytes.
    let rc = unsafe { RegGetValueW(HKEY_CURRENT_USER, sub.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, null_mut(), buf.as_mut_ptr().cast(), &mut len) };
    if rc != ERROR_SUCCESS {
        return None;
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..end]))
}

/// Task Manager's Startup apps page turned LanShare off (the first byte of
/// its StartupApproved value is odd: 0x03 off, 0x02 on).
fn turned_off_in_task_manager() -> bool {
    let (sub, name) = (wide(APPROVED), wide(NAME));
    let mut data = [0u8; 12];
    let mut len = data.len() as u32;
    // SAFETY: `data` holds `len` bytes.
    let rc = unsafe { RegGetValueW(HKEY_CURRENT_USER, sub.as_ptr(), name.as_ptr(), RRF_RT_REG_BINARY, null_mut(), data.as_mut_ptr().cast(), &mut len) };
    rc == ERROR_SUCCESS && len > 0 && data[0] & 1 == 1
}

fn write_run_value(value: &str) -> Result<(), String> {
    let (sub, name, data) = (wide(RUN), wide(NAME), wide(value));
    // SAFETY: NUL-terminated strings; `data` holds the given number of bytes.
    let rc = unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, sub.as_ptr(), name.as_ptr(), REG_SZ, data.as_ptr().cast(), (data.len() * 2) as u32) };
    if rc == ERROR_SUCCESS { Ok(()) } else { Err(format!("注册表没有接受这个设置（错误 {rc}）")) }
}

fn delete(sub: &str) -> Result<(), String> {
    let (sub, name) = (wide(sub), wide(NAME));
    // SAFETY: NUL-terminated strings.
    let rc = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, sub.as_ptr(), name.as_ptr()) };
    if rc == ERROR_SUCCESS || rc == ERROR_FILE_NOT_FOUND { Ok(()) } else { Err(format!("注册表没有接受这个设置（错误 {rc}）")) }
}

/// "on", "off", "needsApproval" (turned off in Task Manager) or
/// "notInstalled". A value that points to an older copy of LanShare (moved
/// or reinstalled) is updated.
pub fn state() -> &'static str {
    if !installed() {
        return "notInstalled";
    }
    let Some(value) = run_value() else { return "off" };
    if let Some(cmd) = command()
        && value != cmd
    {
        let _ = write_run_value(&cmd);
    }
    if turned_off_in_task_manager() { "needsApproval" } else { "on" }
}

/// At a start while Settings has launch at login on: puts a missing Run
/// value back on the first start after an install (see the top of this
/// file). Returns false when it stays off, so the choice is forgotten. A
/// Task Manager "off" is kept: it is a separate value.
pub fn restore(installed_since: bool) -> bool {
    if !installed() || run_value().is_some() {
        return true;
    }
    if !installed_since {
        return false;
    }
    let restored = command().is_some_and(|cmd| write_run_value(&cmd).is_ok());
    tracing::info!("launch at login restored after an upgrade: {}", if restored { "ok" } else { "failed" });
    restored
}

pub fn set(on: bool) -> Result<&'static str, String> {
    if !installed() {
        return Err("只有安装好的 LanShare 才能设置".into());
    }
    if on {
        write_run_value(&command().ok_or("找不到 LanShare 的程序文件")?)?;
        // Turning it on here also undoes "off" in Task Manager.
        delete(APPROVED)?;
    } else {
        delete(RUN)?;
    }
    Ok(state())
}

/// Opens Settings › Apps › Startup.
pub fn settings_url() -> &'static str {
    "ms-settings:startupapps"
}
