//! Pairing state machine (no I/O, so it is easy to test).
//!
//! The host opens a short-lived pairing session that a phone can use once,
//! either through the QR token or the 6-digit code. The resulting request
//! waits until the host approves it after comparing the confirm code.

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;

use super::model::PairRequestOut;
use super::util::{ct_eq, random_digits, random_token};

pub const SESSION_TTL_MS: i64 = 120_000;
pub const REQUEST_TTL_MS: i64 = 180_000;
const MAX_CODE_FAILURES: u32 = 10;
const MAX_PENDING: usize = 5;
const RATE_WINDOW_MS: i64 = 60_000;
const RATE_MAX: usize = 5;

#[derive(Debug, PartialEq, Eq)]
pub enum PairError {
    NoSession,
    BadCredential,
    TooMany,
    RateLimited,
    CodeDisabled,
}

impl PairError {
    pub fn message(&self) -> &'static str {
        match self {
            PairError::NoSession => "二维码或配对码已失效，请在电脑上重新点「添加设备」",
            PairError::BadCredential => "配对码不正确",
            PairError::TooMany => "等待确认的请求太多了，请先在电脑上处理",
            PairError::RateLimited => "尝试太频繁，请稍等一分钟再试",
            PairError::CodeDisabled => "配对码错误次数太多已停用，请扫描二维码，或在电脑上重新生成",
        }
    }
}

struct Session {
    token: String,
    code: String,
    expires_at: i64,
    code_failures: u32,
}

impl Session {
    /// After too many wrong guesses the 6-digit code stops working, but the
    /// unguessable QR token stays valid so an attacker cannot block pairing.
    fn code_disabled(&self) -> bool {
        self.code_failures >= MAX_CODE_FAILURES
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Pending,
    /// Approved; the cookie value is handed out exactly once.
    Approved { cookie: String },
    Claimed,
    Rejected,
}

#[derive(Clone, Debug)]
pub struct Request {
    pub id: String,
    pub name: String,
    pub confirm_code: String,
    pub ip: IpAddr,
    pub created_at: i64,
    pub status: Status,
}

impl Request {
    pub fn to_out(&self) -> PairRequestOut {
        PairRequestOut {
            id: self.id.clone(),
            name: self.name.clone(),
            confirm_code: self.confirm_code.clone(),
            ip: self.ip.to_string(),
            created_at: self.created_at,
        }
    }
}

/// What a polling phone is told.
#[derive(Debug, PartialEq, Eq)]
pub enum Poll {
    Pending,
    Approved { cookie: String },
    Rejected,
    Expired,
}

#[derive(Default)]
pub struct Pairing {
    session: Option<Session>,
    requests: HashMap<String, Request>,
    attempts: HashMap<IpAddr, VecDeque<i64>>,
}

impl Pairing {
    /// Opens a new pairing session (replacing any previous one).
    pub fn start(&mut self, now: i64) -> (String, String, i64) {
        let s = Session { token: random_token(), code: random_digits(6), expires_at: now + SESSION_TTL_MS, code_failures: 0 };
        let out = (s.token.clone(), s.code.clone(), s.expires_at);
        self.session = Some(s);
        out
    }

    pub fn cancel(&mut self) {
        self.session = None;
    }

    fn rate_limited(&mut self, ip: IpAddr, now: i64) -> bool {
        let q = self.attempts.entry(ip).or_default();
        while q.front().is_some_and(|t| now - t > RATE_WINDOW_MS) {
            q.pop_front();
        }
        if q.len() >= RATE_MAX {
            return true;
        }
        q.push_back(now);
        false
    }

    /// A phone presents the QR token or the 6-digit code.
    pub fn request(&mut self, token: Option<&str>, code: Option<&str>, name: &str, ip: IpAddr, now: i64) -> Result<Request, PairError> {
        self.gc(now);
        if self.rate_limited(ip, now) {
            return Err(PairError::RateLimited);
        }
        if !self.session.as_ref().is_some_and(|s| s.expires_at > now) {
            self.session = None;
            return Err(PairError::NoSession);
        }
        let session = self.session.as_mut().expect("checked above");
        let ok = match (token, code) {
            (Some(t), _) if !t.is_empty() => ct_eq(t.as_bytes(), session.token.as_bytes()),
            (_, Some(_)) if session.code_disabled() => return Err(PairError::CodeDisabled),
            (_, Some(c)) if !c.is_empty() => {
                let ok = ct_eq(c.as_bytes(), session.code.as_bytes());
                if !ok {
                    session.code_failures += 1;
                }
                ok
            }
            _ => false,
        };
        if !ok {
            return Err(PairError::BadCredential);
        }
        if self.requests.values().filter(|r| r.status == Status::Pending).count() >= MAX_PENDING {
            return Err(PairError::TooMany);
        }
        // One session, one device.
        self.session = None;
        let req = Request {
            id: random_token(),
            name: name.to_string(),
            confirm_code: random_digits(4),
            ip,
            created_at: now,
            status: Status::Pending,
        };
        self.requests.insert(req.id.clone(), req.clone());
        Ok(req)
    }

    pub fn get_pending(&mut self, id: &str, now: i64) -> Option<Request> {
        self.gc(now);
        self.requests.get(id).filter(|r| r.status == Status::Pending).cloned()
    }

    pub fn approve(&mut self, id: &str, cookie: String) -> bool {
        match self.requests.get_mut(id) {
            Some(r) if r.status == Status::Pending => {
                r.status = Status::Approved { cookie };
                true
            }
            _ => false,
        }
    }

    pub fn reject(&mut self, id: &str) -> bool {
        match self.requests.get_mut(id) {
            Some(r) if r.status == Status::Pending => {
                r.status = Status::Rejected;
                true
            }
            _ => false,
        }
    }

    pub fn poll(&mut self, id: &str, now: i64) -> Poll {
        self.gc(now);
        let Some(r) = self.requests.get_mut(id) else { return Poll::Expired };
        match std::mem::replace(&mut r.status, Status::Claimed) {
            Status::Pending => {
                r.status = Status::Pending;
                Poll::Pending
            }
            Status::Approved { cookie } => Poll::Approved { cookie },
            Status::Rejected => {
                r.status = Status::Rejected;
                Poll::Rejected
            }
            Status::Claimed => Poll::Expired,
        }
    }

    pub fn pending(&mut self, now: i64) -> Vec<Request> {
        self.gc(now);
        let mut v: Vec<Request> = self.requests.values().filter(|r| r.status == Status::Pending).cloned().collect();
        v.sort_by_key(|r| r.created_at);
        v
    }

    /// Drops requests older than the TTL, whatever their state.
    fn gc(&mut self, now: i64) {
        self.requests.retain(|_, r| now - r.created_at < REQUEST_TTL_MS);
        self.attempts.retain(|_, q| q.back().is_some_and(|t| now - t <= RATE_WINDOW_MS));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(n: u8) -> IpAddr {
        IpAddr::from([192, 168, 1, n])
    }

    #[test]
    fn token_is_single_use_and_flow_completes() {
        let mut p = Pairing::default();
        let (token, _code, _) = p.start(0);
        let req = p.request(Some(&token), None, "iPhone", ip(2), 10).unwrap();
        assert_eq!(req.confirm_code.len(), 4);
        // Same token again is rejected: the session was consumed.
        assert_eq!(p.request(Some(&token), None, "x", ip(3), 11).unwrap_err(), PairError::NoSession);
        assert_eq!(p.poll(&req.id, 20), Poll::Pending);
        assert!(p.approve(&req.id, "dev.secret".into()));
        assert_eq!(p.poll(&req.id, 30), Poll::Approved { cookie: "dev.secret".into() });
        // The cookie is handed out once.
        assert_eq!(p.poll(&req.id, 31), Poll::Expired);
    }

    #[test]
    fn code_failures_and_expiry() {
        let mut p = Pairing::default();
        let (_t, code, exp) = p.start(0);
        assert_eq!(p.request(None, Some("000000x"), "a", ip(2), 1).unwrap_err(), PairError::BadCredential);
        assert!(p.request(None, Some(&code), "a", ip(2), exp + 1).is_err());

        let (token, code, _) = p.start(1000);
        let wrong = if code == "999999" { "999998" } else { "999999" };
        // Spread attempts over different IPs to stay under the per-IP rate limit.
        for i in 0..10 {
            assert_eq!(p.request(None, Some(wrong), "a", ip(10 + i), 2000 + i as i64).unwrap_err(), PairError::BadCredential);
        }
        // The code is now dead, even the right one…
        assert_eq!(p.request(None, Some(&code), "a", ip(50), 3000).unwrap_err(), PairError::CodeDisabled);
        // …but the QR token still pairs.
        assert!(p.request(Some(&token), None, "a", ip(51), 3001).is_ok());
    }

    #[test]
    fn rate_limit_per_ip() {
        let mut p = Pairing::default();
        p.start(0);
        for i in 0..5 {
            assert_eq!(p.request(None, Some("x"), "a", ip(9), i).unwrap_err(), PairError::BadCredential);
        }
        assert_eq!(p.request(None, Some("x"), "a", ip(9), 6).unwrap_err(), PairError::RateLimited);
        // A minute later the window has moved on.
        assert_ne!(p.request(None, Some("x"), "a", ip(9), 70_000).unwrap_err(), PairError::RateLimited);
    }

    #[test]
    fn reject_and_gc() {
        let mut p = Pairing::default();
        let (t, _, _) = p.start(0);
        let req = p.request(Some(&t), None, "a", ip(2), 0).unwrap();
        assert_eq!(p.pending(1).len(), 1);
        assert!(p.reject(&req.id));
        assert_eq!(p.poll(&req.id, 2), Poll::Rejected);
        assert!(p.pending(3).is_empty());
        assert_eq!(p.poll(&req.id, REQUEST_TTL_MS + 5), Poll::Expired);
    }
}
