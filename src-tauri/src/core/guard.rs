//! Connection-level protection for the LAN listener, applied before TLS:
//! non-LAN peers are dropped and each address gets a bounded number of
//! concurrent connections, so idle sockets cannot exhaust file descriptors.

use std::collections::HashMap;
use std::future::{Ready, ready};
use std::io;
use std::net::IpAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use axum_server::accept::Accept;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;

use super::net::is_lan_peer;

const MAX_PER_IP: usize = 32;
const MAX_TOTAL: usize = 256;

#[derive(Default)]
struct Counts {
    per_ip: HashMap<IpAddr, usize>,
    total: usize,
}

#[derive(Clone, Default)]
pub struct LanAcceptor {
    counts: Arc<Mutex<Counts>>,
}

/// Releases the connection slot when the connection closes.
pub struct Slot {
    counts: Arc<Mutex<Counts>>,
    ip: IpAddr,
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut c = self.counts.lock().unwrap_or_else(|p| p.into_inner());
        c.total = c.total.saturating_sub(1);
        if let Some(n) = c.per_ip.get_mut(&self.ip) {
            *n -= 1;
            if *n == 0 {
                c.per_ip.remove(&self.ip);
            }
        }
    }
}

impl LanAcceptor {
    fn take_slot(&self, ip: IpAddr) -> Option<Slot> {
        let mut c = self.counts.lock().unwrap_or_else(|p| p.into_inner());
        let n = c.per_ip.get(&ip).copied().unwrap_or(0);
        if n >= MAX_PER_IP || c.total >= MAX_TOTAL {
            return None;
        }
        c.per_ip.insert(ip, n + 1);
        c.total += 1;
        Some(Slot { counts: self.counts.clone(), ip })
    }
}

/// A TCP stream that holds its connection slot for as long as it lives.
pub struct GuardedStream {
    inner: TcpStream,
    _slot: Slot,
}

impl<S> Accept<TcpStream, S> for LanAcceptor {
    type Stream = GuardedStream;
    type Service = S;
    type Future = Ready<io::Result<(GuardedStream, S)>>;

    fn accept(&self, stream: TcpStream, service: S) -> Self::Future {
        let ip = match stream.peer_addr() {
            Ok(a) => a.ip().to_canonical(),
            Err(e) => return ready(Err(e)),
        };
        if !is_lan_peer(ip) {
            return ready(Err(io::Error::new(io::ErrorKind::PermissionDenied, "not a LAN peer")));
        }
        match self.take_slot(ip) {
            Some(slot) => ready(Ok((GuardedStream { inner: stream, _slot: slot }, service))),
            None => {
                tracing::warn!("too many connections from {ip}; dropping");
                ready(Err(io::Error::new(io::ErrorKind::ConnectionRefused, "connection limit")))
            }
        }
    }
}

impl AsyncRead for GuardedStream {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for GuardedStream {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
    fn poll_write_vectored(mut self: Pin<&mut Self>, cx: &mut Context<'_>, bufs: &[io::IoSlice<'_>]) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write_vectored(cx, bufs)
    }
    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }
}

/// GUI apps on macOS start with a soft limit of 256 open files.
pub fn raise_fd_limit() {
    #[cfg(unix)]
    // SAFETY: plain libc calls with a properly initialised struct.
    unsafe {
        let mut lim = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim) == 0 {
            let want: libc::rlim_t = 8192;
            let target = if lim.rlim_max < want { lim.rlim_max } else { want };
            if lim.rlim_cur < target {
                lim.rlim_cur = target;
                let _ = libc::setrlimit(libc::RLIMIT_NOFILE, &lim);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_are_bounded_and_released() {
        let a = LanAcceptor::default();
        let ip: IpAddr = "192.168.1.2".parse().unwrap();
        let slots: Vec<Slot> = (0..MAX_PER_IP).map(|_| a.take_slot(ip).unwrap()).collect();
        assert!(a.take_slot(ip).is_none());
        assert!(a.take_slot("192.168.1.3".parse().unwrap()).is_some());
        drop(slots);
        assert!(a.take_slot(ip).is_some());
    }
}
