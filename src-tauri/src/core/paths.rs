//! Keeps every client-supplied path inside the shared folder.
//!
//! Clients always send paths relative to the shared root using `/` separators.
//! Each segment is validated, the joined path is canonicalized (resolving
//! symlinks), and the result must still live under the canonical root.

use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum PathError {
    /// The path is malformed or names something we never expose.
    Invalid,
    NotFound,
    /// Resolves outside the shared folder (e.g. through a symlink).
    Outside,
    Io(io::Error),
}

impl std::fmt::Display for PathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PathError::Invalid => write!(f, "路径无效"),
            PathError::NotFound => write!(f, "文件或文件夹不存在"),
            PathError::Outside => write!(f, "不允许访问共享文件夹以外的位置"),
            PathError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl From<io::Error> for PathError {
    fn from(e: io::Error) -> Self {
        if e.kind() == io::ErrorKind::NotFound {
            PathError::NotFound
        } else {
            PathError::Io(e)
        }
    }
}

#[derive(Clone, Debug)]
pub struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    /// Creates the folder if needed and remembers its canonical form.
    pub fn new(root: &Path) -> io::Result<Self> {
        std::fs::create_dir_all(root)?;
        Ok(Self { root: dunce::canonicalize(root)? })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolves an existing file or directory.
    pub fn resolve_existing(&self, rel: &str) -> Result<PathBuf, PathError> {
        let mut joined = self.root.clone();
        for seg in segments(rel)? {
            joined.push(seg);
        }
        let canon = dunce::canonicalize(&joined)?;
        if !canon.starts_with(&self.root) {
            return Err(PathError::Outside);
        }
        Ok(canon)
    }

    pub fn resolve_dir(&self, rel: &str) -> Result<PathBuf, PathError> {
        let p = self.resolve_existing(rel)?;
        if p.is_dir() { Ok(p) } else { Err(PathError::Invalid) }
    }

    pub fn resolve_file(&self, rel: &str) -> Result<PathBuf, PathError> {
        let p = self.resolve_existing(rel)?;
        if p.is_file() { Ok(p) } else { Err(PathError::Invalid) }
    }

    /// Ensures `<root>/<name>` exists as a directory (used for 收件箱 / 已发送).
    pub fn ensure_subdir(&self, name: &str) -> Result<PathBuf, PathError> {
        validate_segment(name)?;
        let p = self.root.join(name);
        std::fs::create_dir_all(&p)?;
        let canon = dunce::canonicalize(&p)?;
        if !canon.starts_with(&self.root) || !canon.is_dir() {
            return Err(PathError::Outside);
        }
        Ok(canon)
    }

    /// Converts an absolute path under the root back to the client form.
    pub fn relative(&self, abs: &Path) -> Option<String> {
        let rest = abs.strip_prefix(&self.root).ok()?;
        let parts: Vec<String> = rest
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        Some(parts.join("/"))
    }
}

/// Splits `a/b/c` into validated segments; empty segments are ignored so
/// leading/trailing slashes and the empty string (the root) are accepted.
pub fn segments(rel: &str) -> Result<Vec<&str>, PathError> {
    let mut out = Vec::new();
    for seg in rel.split('/') {
        if seg.is_empty() {
            continue;
        }
        validate_segment(seg)?;
        out.push(seg);
    }
    Ok(out)
}

pub fn validate_segment(seg: &str) -> Result<(), PathError> {
    // `.`-prefixed names cover `.`/`..`, dotfiles and our own upload temp files.
    if seg.is_empty() || seg.starts_with('.') {
        return Err(PathError::Invalid);
    }
    if seg.chars().any(|c| c == '\\' || c == ':' || c.is_control()) {
        return Err(PathError::Invalid);
    }
    if cfg!(windows) && (is_windows_reserved(seg) || seg.ends_with(' ') || seg.ends_with('.')) {
        return Err(PathError::Invalid);
    }
    Ok(())
}

/// Names Windows refuses to create regardless of extension (`NUL.txt` included).
pub fn is_windows_reserved(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit()
            && stem.as_bytes()[3] != b'0')
}

/// Turns an arbitrary client-provided file name into one that is valid on
/// both macOS and Windows. Returns `None` if nothing usable is left.
pub fn sanitize_filename(raw: &str) -> Option<String> {
    // Some clients send a full path; keep only the last component.
    let base = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let replaced: String = base
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let trimmed = replaced
        .trim()
        .trim_start_matches('.')
        .trim_end_matches(['.', ' '])
        .trim();
    if trimmed.is_empty() {
        return None;
    }
    let name = if is_windows_reserved(trimmed) { format!("_{trimmed}") } else { trimmed.to_string() };
    Some(truncate_keep_extension(&name, 180))
}

/// Limits a name to `max` bytes, keeping the extension and char boundaries.
fn truncate_keep_extension(name: &str, max: usize) -> String {
    if name.len() <= max {
        return name.to_string();
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 && name.len() - i <= 16 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    let budget = max.saturating_sub(ext.len());
    let mut end = budget.min(stem.len());
    while !stem.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}", &stem[..end], ext)
}

/// Picks `name`, or `name (1).ext`, `name (2).ext`… so nothing is overwritten.
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    for n in 1..10_000 {
        let candidate = dir.join(format!("{stem} ({n}){ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{stem} ({}){ext}", crate::core::util::random_hex(4)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sandbox() -> (tempfile::TempDir, Sandbox) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("share");
        let sb = Sandbox::new(&root).unwrap();
        std::fs::create_dir_all(sb.root().join("docs/sub")).unwrap();
        std::fs::write(sb.root().join("docs/a.txt"), "hi").unwrap();
        std::fs::write(dir.path().join("secret.txt"), "nope").unwrap();
        (dir, sb)
    }

    #[test]
    fn resolves_inside_root() {
        let (_d, sb) = sandbox();
        assert!(sb.resolve_file("docs/a.txt").is_ok());
        assert!(sb.resolve_file("/docs//a.txt/").is_ok());
        assert!(sb.resolve_dir("").is_ok());
        assert_eq!(sb.relative(&sb.resolve_file("docs/a.txt").unwrap()).unwrap(), "docs/a.txt");
    }

    #[test]
    fn rejects_traversal_and_hidden() {
        let (_d, sb) = sandbox();
        for bad in ["..", "../secret.txt", "docs/../../secret.txt", "./docs", ".hidden", "docs/.x", "a\\b", "C:/x", "a\u{0}b"] {
            assert!(sb.resolve_existing(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn missing_is_not_found() {
        let (_d, sb) = sandbox();
        assert!(matches!(sb.resolve_existing("nope.txt"), Err(PathError::NotFound)));
        assert!(matches!(sb.resolve_dir("docs/a.txt"), Err(PathError::Invalid)));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape() {
        let (d, sb) = sandbox();
        std::os::unix::fs::symlink(d.path().join("secret.txt"), sb.root().join("link.txt")).unwrap();
        std::os::unix::fs::symlink(d.path(), sb.root().join("linkdir")).unwrap();
        assert!(matches!(sb.resolve_existing("link.txt"), Err(PathError::Outside)));
        assert!(matches!(sb.resolve_existing("linkdir/secret.txt"), Err(PathError::Outside)));
        // A symlink that stays inside the root is fine.
        std::os::unix::fs::symlink(sb.root().join("docs"), sb.root().join("inner")).unwrap();
        assert!(sb.resolve_file("inner/a.txt").is_ok());
    }

    #[test]
    fn sanitizes_names() {
        assert_eq!(sanitize_filename("photo.jpg").unwrap(), "photo.jpg");
        assert_eq!(sanitize_filename("../../etc/passwd").unwrap(), "passwd");
        assert_eq!(sanitize_filename("C:\\Users\\x\\a:b?.txt").unwrap(), "a_b_.txt");
        assert_eq!(sanitize_filename("  .bashrc  ").unwrap(), "bashrc");
        assert_eq!(sanitize_filename("name. . ").unwrap(), "name");
        assert_eq!(sanitize_filename("NUL.txt").unwrap(), "_NUL.txt");
        assert_eq!(sanitize_filename("com1").unwrap(), "_com1");
        assert_eq!(sanitize_filename("会议记录.docx").unwrap(), "会议记录.docx");
        assert!(sanitize_filename("...").is_none());
        assert!(sanitize_filename("").is_none());
        let long = format!("{}.mp4", "长".repeat(100));
        let s = sanitize_filename(&long).unwrap();
        assert!(s.len() <= 180 && s.ends_with(".mp4"));
    }

    #[test]
    fn windows_reserved_names() {
        assert!(is_windows_reserved("CON"));
        assert!(is_windows_reserved("aux.log"));
        assert!(is_windows_reserved("LPT9"));
        assert!(!is_windows_reserved("COM0"));
        assert!(!is_windows_reserved("console"));
        assert!(!is_windows_reserved("COM10"));
    }

    #[test]
    fn unique_path_appends_counter() {
        let (_d, sb) = sandbox();
        let dir = sb.root().join("docs");
        assert_eq!(unique_path(&dir, "b.txt"), dir.join("b.txt"));
        assert_eq!(unique_path(&dir, "a.txt"), dir.join("a (1).txt"));
        std::fs::write(dir.join("a (1).txt"), "").unwrap();
        assert_eq!(unique_path(&dir, "a.txt"), dir.join("a (2).txt"));
    }
}
