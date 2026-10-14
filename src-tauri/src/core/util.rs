//! Small helpers: randomness, time, hashing.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut buf = [0u8; N];
    // Failing to read OS randomness is unrecoverable for a security tool.
    getrandom::fill(&mut buf).expect("operating system randomness is unavailable");
    buf
}

/// 256-bit secret, URL-safe base64 (43 chars).
pub fn random_token() -> String {
    URL_SAFE_NO_PAD.encode(random_bytes::<32>())
}

/// `n` random bytes as lowercase hex.
pub fn random_hex(n: usize) -> String {
    let mut buf = vec![0u8; n];
    getrandom::fill(&mut buf).expect("operating system randomness is unavailable");
    hex::encode(buf)
}

/// Uniformly random decimal digits (rejection sampling avoids modulo bias).
pub fn random_digits(n: usize) -> String {
    let mut out = String::with_capacity(n);
    while out.len() < n {
        let [b] = random_bytes::<1>();
        if b < 250 {
            out.push(char::from(b'0' + b % 10));
        }
    }
    out
}

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

pub fn sha256_hex(data: impl AsRef<[u8]>) -> String {
    hex::encode(Sha256::digest(data))
}

/// Constant-time comparison for secrets.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// First `max` characters of `s` on one line, for notification previews.
pub fn preview(s: &str, max: usize) -> String {
    let one_line: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = one_line.chars().take(max).collect();
    if one_line.chars().count() > max {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_and_digits() {
        assert_eq!(random_token().len(), 43);
        assert_ne!(random_token(), random_token());
        let d = random_digits(6);
        assert_eq!(d.len(), 6);
        assert!(d.chars().all(|c| c.is_ascii_digit()));
        assert_eq!(random_hex(4).len(), 8);
    }

    #[test]
    fn constant_time_eq() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"ab"));
    }

    #[test]
    fn previews() {
        assert_eq!(preview("hello\n  world", 20), "hello world");
        assert_eq!(preview("一二三四五六", 3), "一二三…");
    }
}
