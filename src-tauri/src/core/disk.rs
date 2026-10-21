//! Free space and file-system type of the disk an upload goes to, read
//! straight from the OS (no extra crates).

use std::io;
use std::path::Path;

#[derive(Clone, Copy, Debug)]
pub struct DiskInfo {
    /// Bytes this process may still write.
    pub available: u64,
    pub total: u64,
    /// FAT12/16/32, as on many USB sticks: one file must stay under 4 GiB.
    pub fat: bool,
}

/// The largest file a FAT file system can hold.
pub const FAT_MAX_FILE: u64 = u32::MAX as u64;

const GB: u64 = 1 << 30;

/// Space uploads must always leave free (section 3 of PLAN.md): 5% of the
/// disk, at least 2 GB and at most 20 GB. On a small disk such as a USB stick
/// it is capped at half the disk, so uploads are not refused outright.
pub fn reserve_for(total: u64) -> u64 {
    (total / 20).clamp(2 * GB, 20 * GB).min(total / 2)
}

/// "1.4 GB", "730 MB", "12 KB": sizes for messages shown to the user.
pub fn human_size(n: u64) -> String {
    const MB: u64 = 1 << 20;
    match n {
        n if n >= GB => format!("{:.1} GB", n as f64 / GB as f64),
        n if n >= 10 * MB => format!("{} MB", n / MB),
        n if n >= MB => format!("{:.1} MB", n as f64 / MB as f64),
        n => format!("{} KB", n.div_ceil(1024)),
    }
}

#[cfg(test)]
static FAKE: std::sync::Mutex<Vec<(std::path::PathBuf, DiskInfo)>> = std::sync::Mutex::new(Vec::new());

/// Tests: pretend every path under `root` is on a disk like `info`.
#[cfg(test)]
pub fn fake_disk(root: &Path, info: DiskInfo) {
    FAKE.lock().unwrap().push((root.to_path_buf(), info));
}

/// `path` is a folder or a file; for a file, the folder it is in is asked
/// (Windows answers only for folders).
pub fn disk_info(path: &Path) -> io::Result<DiskInfo> {
    let path = if path.is_file() { path.parent().unwrap_or(path) } else { path };
    #[cfg(test)]
    if let Some((_, info)) = FAKE.lock().unwrap().iter().rev().find(|(root, _)| path.starts_with(root)) {
        return Ok(*info);
    }
    os_disk_info(path)
}

#[cfg(target_os = "macos")]
fn os_disk_info(path: &Path) -> io::Result<DiskInfo> {
    use std::ffi::{CStr, CString};
    use std::os::unix::ffi::OsStrExt;
    let p = CString::new(path.as_os_str().as_bytes()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    // SAFETY: a plain C struct that statfs fills in.
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: a NUL-terminated path and a valid out-pointer.
    if unsafe { libc::statfs(p.as_ptr(), &mut st) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let block = u64::from(st.f_bsize);
    // SAFETY: statfs NUL-terminates f_fstypename.
    let fstype = unsafe { CStr::from_ptr(st.f_fstypename.as_ptr()) }.to_string_lossy().to_ascii_lowercase();
    // FAT and exFAT volumes answer 0 here, so it only ever adds to statfs.
    let statfs_available = st.f_bavail * block;
    let available = important_usage_available(path).map_or(statfs_available, |n| n.max(statfs_available));
    // "msdos" is FAT12/16/32; exFAT ("exfat") has no 4 GB limit.
    Ok(DiskInfo { available, total: st.f_blocks * block, fat: fstype == "msdos" })
}

/// What macOS can free up for something the user asked for: free space plus
/// purgeable space (caches, local copies of iCloud files), as Finder counts
/// it. statfs alone leaves purgeable space out and can report far less.
#[cfg(target_os = "macos")]
fn important_usage_available(path: &Path) -> Option<u64> {
    use objc2_foundation::{NSNumber, NSString, NSURL, NSURLVolumeAvailableCapacityForImportantUsageKey};
    objc2::rc::autoreleasepool(|_| {
        let url = NSURL::fileURLWithPath(&NSString::from_str(path.to_str()?));
        let mut value = None;
        // SAFETY: the extern key constant is valid for the process lifetime, and
        // its value is an NSNumber, which is checked by the downcast below.
        unsafe { url.getResourceValue_forKey_error(&mut value, NSURLVolumeAvailableCapacityForImportantUsageKey) }.ok()?;
        let n = value?.downcast::<NSNumber>().ok()?;
        u64::try_from(n.longLongValue()).ok()
    })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn os_disk_info(path: &Path) -> io::Result<DiskInfo> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let p = CString::new(path.as_os_str().as_bytes()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    // SAFETY: a plain C struct that statvfs fills in.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: a NUL-terminated path and a valid out-pointer.
    if unsafe { libc::statvfs(p.as_ptr(), &mut st) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let block = st.f_frsize as u64;
    Ok(DiskInfo { available: st.f_bavail as u64 * block, total: st.f_blocks as u64 * block, fat: false })
}

#[cfg(windows)]
fn os_disk_info(path: &Path) -> io::Result<DiskInfo> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetVolumeInformationW, GetVolumePathNameW};
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let (mut available, mut total, mut free) = (0u64, 0u64, 0u64);
    // SAFETY: a NUL-terminated path and valid out-pointers.
    if unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut available, &mut total, &mut free) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut root = [0u16; 1024];
    let mut fs_name = [0u16; 64];
    // SAFETY: each buffer is passed with its own length.
    let named = unsafe {
        GetVolumePathNameW(wide.as_ptr(), root.as_mut_ptr(), root.len() as u32) != 0
            && GetVolumeInformationW(
                root.as_ptr(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                fs_name.as_mut_ptr(),
                fs_name.len() as u32,
            ) != 0
    };
    let len = fs_name.iter().position(|&c| c == 0).unwrap_or(fs_name.len());
    let name = String::from_utf16_lossy(&fs_name[..len]);
    let fat = named && (name.eq_ignore_ascii_case("FAT32") || name.eq_ignore_ascii_case("FAT"));
    Ok(DiskInfo { available, total, fat })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserve_follows_the_plan() {
        assert_eq!(reserve_for(1000 * GB), 20 * GB, "capped at 20 GB");
        assert_eq!(reserve_for(200 * GB), 10 * GB, "5% of the disk");
        assert_eq!(reserve_for(20 * GB), 2 * GB, "at least 2 GB");
        assert_eq!(reserve_for(2 * GB), GB, "never more than half of a small disk");
    }

    #[test]
    fn reads_this_disk() {
        let info = os_disk_info(&std::env::temp_dir()).unwrap();
        assert!(info.total > 0 && info.available <= info.total);
        let file = tempfile::NamedTempFile::new().unwrap();
        assert!(disk_info(file.path()).unwrap().total > 0, "a file's disk is its folder's");
        assert_eq!(human_size(3 * GB / 2), "1.5 GB");
        assert_eq!(human_size(730 << 20), "730 MB");
        assert_eq!(human_size(1_468_006), "1.4 MB");
    }
}
