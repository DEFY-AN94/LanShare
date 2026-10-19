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
    /// Friendly form (no `\\?\` prefix on Windows) for display and relative paths.
    root: PathBuf,
    /// Form produced by `std::fs::canonicalize`; containment checks compare
    /// against this so long Windows paths (kept verbatim) still match.
    root_canon: PathBuf,
    /// Places inside the root that must never be served (LanShare's own data).
    deny: Vec<PathBuf>,
}

impl Sandbox {
    /// Creates the folder if needed and remembers its canonical form.
    pub fn new(root: &Path) -> io::Result<Self> {
        std::fs::create_dir_all(root)?;
        Self::existing(root)
    }

    /// Opens a folder that must already exist (never creates it).
    pub fn existing(root: &Path) -> io::Result<Self> {
        let root_canon = std::fs::canonicalize(root)?;
        if !root_canon.is_dir() {
            return Err(io::Error::new(io::ErrorKind::NotADirectory, "not a folder"));
        }
        Ok(Self { root: friendly(&root_canon), root_canon, deny: Vec::new() })
    }

    /// Adds paths (e.g. the app data dir) that stay off-limits even if they
    /// happen to live under the root.
    pub fn with_deny(mut self, paths: &[PathBuf]) -> Self {
        self.deny = paths.iter().map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone())).collect();
        self
    }

    /// `canon` must come from `std::fs::canonicalize`. Besides staying under
    /// the root, every component below it must be a name we would list:
    /// this also catches Windows 8.3 aliases (`ENV~1` → `.env`) and items
    /// with the hidden/system attribute.
    fn check(&self, canon: &Path) -> Result<(), PathError> {
        let rest = canon.strip_prefix(&self.root_canon).map_err(|_| PathError::Outside)?;
        if self.deny.iter().any(|d| canon.starts_with(d)) {
            return Err(PathError::Outside);
        }
        let mut cur = self.root_canon.clone();
        for comp in rest.components() {
            let name = comp.as_os_str().to_string_lossy();
            if name.starts_with('.') {
                return Err(PathError::Outside);
            }
            cur.push(comp);
            if hidden_by_os(&cur) {
                return Err(PathError::Outside);
            }
        }
        Ok(())
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
        let canon = std::fs::canonicalize(&joined)?;
        self.check(&canon)?;
        Ok(friendly(&canon))
    }

    /// Resolves an entry *without* following a symlink in its last component,
    /// so rename/delete act on the link itself rather than its target.
    pub fn resolve_entry(&self, rel: &str) -> Result<PathBuf, PathError> {
        let segs = segments(rel)?;
        let (last, parents) = segs.split_last().ok_or(PathError::Invalid)?;
        let parent = std::fs::canonicalize(self.resolve_dir(&parents.join("/"))?)?;
        let entry = parent.join(last);
        // A real file or folder is checked under its on-disk name, so a
        // Windows 8.3 alias (`XXXX~1`) cannot stand in for a protected name.
        let entry = if std::fs::symlink_metadata(&entry)?.file_type().is_symlink() { entry } else { std::fs::canonicalize(&entry)? };
        self.check(&entry)?;
        Ok(friendly(&entry))
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
        let canon = std::fs::canonicalize(&p)?;
        self.check(&canon)?;
        if !canon.is_dir() {
            return Err(PathError::Invalid);
        }
        Ok(friendly(&canon))
    }

    /// Converts an absolute path under the root back to the client form.
    pub fn relative(&self, abs: &Path) -> Option<String> {
        let rest = abs.strip_prefix(&self.root).or_else(|_| abs.strip_prefix(&self.root_canon)).ok()?;
        let parts: Vec<String> = rest
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        Some(parts.join("/"))
    }
}

/// Drops the `\\?\` prefix where Windows allows it (paths under MAX_PATH);
/// long paths stay verbatim so they keep working.
fn friendly(p: &Path) -> PathBuf {
    dunce::simplified(p).to_path_buf()
}

/// A path the Windows shell accepts (it rejects `\\?\` forms): verbatim
/// UNC becomes `\\host\share`, verbatim disk paths lose the prefix.
pub fn shell_path(p: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let s = p.to_string_lossy();
        if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = s.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }
    p.to_path_buf()
}

/// Windows hidden/system attributes (e.g. AppData, desktop.ini).
fn hidden_by_os(_p: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const HIDDEN: u32 = 0x2;
        const SYSTEM: u32 = 0x4;
        if let Ok(m) = std::fs::symlink_metadata(_p) {
            return m.file_attributes() & (HIDDEN | SYSTEM) != 0;
        }
    }
    false
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

/// Invisible formatting characters. Bidi overrides (U+202E…) can make
/// `photo\u{202E}gpj.app` display as `photoppa.jpg`, so they are dropped.
fn is_format_char(c: char) -> bool {
    matches!(c, '\u{00AD}' | '\u{061C}' | '\u{180E}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}')
}

/// Turns an arbitrary client-provided file name into one that is valid on
/// both macOS and Windows. Returns `None` if nothing usable is left.
pub fn sanitize_filename(raw: &str) -> Option<String> {
    // Some clients send a full path; keep only the last component.
    let base = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let replaced: String = base
        .chars()
        .filter(|c| !is_format_char(*c))
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

/// Why a folder must not become the shared folder, if it must not.
/// Sharing the home folder (or anything above it) would expose things like
/// ~/Library and LanShare's own private keys to every paired device.
pub fn unsafe_root_reason(root: &Path, home: &Path, data_dir: &Path) -> Option<&'static str> {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let (root, home, data) = (canon(root), canon(home), canon(data_dir));
    if root.parent().is_none() {
        return Some("不能把整个磁盘设为共享文件夹");
    }
    if home.starts_with(&root) {
        return Some("不能共享用户文件夹本身或它的上级，请选择或新建一个子文件夹");
    }
    if data.starts_with(&root) || root.starts_with(&data) {
        return Some("这个文件夹包含 LanShare 自己的数据，不能共享");
    }
    for private in ["Library", "AppData"] {
        if root.starts_with(home.join(private)) {
            return Some("不能共享系统资料库文件夹");
        }
    }
    None
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
    fn strips_bidi_overrides() {
        assert_eq!(sanitize_filename("IMG\u{202E}gpj.terminal").unwrap(), "IMGgpj.terminal");
        assert_eq!(sanitize_filename("a\u{200B}b.txt").unwrap(), "ab.txt");
    }

    #[test]
    fn deny_list_and_entries() {
        let (_dir, sb) = sandbox();
        std::fs::create_dir_all(sb.root().join("data/tls")).unwrap();
        std::fs::write(sb.root().join("data/tls/key.pem"), "k").unwrap();
        let data = sb.root().join("data");
        let sb = sb.with_deny(&[data]);
        assert!(matches!(sb.resolve_existing("data/tls/key.pem"), Err(PathError::Outside)));
        assert!(sb.resolve_existing("docs/a.txt").is_ok());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(_dir.path().join("secret.txt"), sb.root().join("out")).unwrap();
            // The link itself (not its outside target) is addressable for rename/delete.
            assert_eq!(sb.resolve_entry("out").unwrap(), sb.root().join("out"));
        }
        assert!(sb.resolve_entry("").is_err());
    }

    #[test]
    fn unsafe_roots() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        let data = home.join("Library/Application Support/app");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir_all(home.join("LanShare")).unwrap();
        assert!(unsafe_root_reason(&home, &home, &data).is_some());
        assert!(unsafe_root_reason(d.path(), &home, &data).is_some());
        assert!(unsafe_root_reason(&home.join("Library"), &home, &data).is_some());
        assert!(unsafe_root_reason(&data, &home, &data).is_some());
        assert!(unsafe_root_reason(&home.join("LanShare"), &home, &data).is_none());
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
