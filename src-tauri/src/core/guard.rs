//! Connection-level protection for the LAN listener. Before TLS, non-LAN
//! peers are dropped and each address gets a bounded number of concurrent
//! connections, so idle sockets cannot exhaust file descriptors. After it, a
//! connection that never starts a request, or whose peer stops reading
//! what it is sent, is closed, so it cannot hold its slot for good.

use std::collections::HashMap;
use std::future::{Future, Ready, ready};
use std::io;
use std::net::IpAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use axum_server::accept::Accept;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio::time::Sleep;

use super::net::is_lan_peer;

const MAX_PER_IP: usize = 32;
const MAX_TOTAL: usize = 256;
/// From the end of the TLS handshake until the connection has said which
/// HTTP version it speaks. Then hyper's own timers take over: 20 s to read
/// each HTTP/1 request head, HTTP/2 pings answered.
const FIRST_REQUEST: Duration = Duration::from_secs(20);
/// How long a peer may leave what it is sent unread.
const WRITE_STALL: Duration = Duration::from_secs(60);
const H2_PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

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

/// Wraps the TLS acceptor (whose handshake has 10 s): logs failed
/// handshakes, e.g. a phone that does not trust the local CA, which rustls
/// drops silently otherwise, and times the connection once it is up.
#[derive(Clone)]
pub struct Handshake<A>(pub A);

impl<A, S> Accept<TcpStream, S> for Handshake<A>
where
    A: Accept<TcpStream, S>,
    A::Future: Send + 'static,
{
    type Stream = TimedStream<A::Stream>;
    type Service = A::Service;
    type Future = Pin<Box<dyn Future<Output = io::Result<(Self::Stream, A::Service)>> + Send>>;

    fn accept(&self, stream: TcpStream, service: S) -> Self::Future {
        let peer = stream.peer_addr().map(|a| a.ip().to_canonical()).ok();
        let fut = self.0.accept(stream, service);
        Box::pin(async move {
            let res = fut.await;
            if let (Err(e), Some(ip)) = (&res, peer) {
                // Refused and non-LAN peers are LanAcceptor's own decisions.
                if !matches!(e.kind(), io::ErrorKind::PermissionDenied | io::ErrorKind::ConnectionRefused) {
                    tracing::debug!("TLS handshake with {ip} failed: {e}");
                }
            }
            res.map(|(stream, service)| (TimedStream::new(stream, FIRST_REQUEST, WRITE_STALL), service))
        })
    }
}

/// A connection after its TLS handshake. hyper has no timer for the bytes
/// that tell HTTP/1 from HTTP/2, nor for writing to a peer that does not
/// read, so these two are timed here.
pub struct TimedStream<S> {
    inner: S,
    /// Until the HTTP version is known.
    first_request: Option<Pin<Box<Sleep>>>,
    /// How much of the HTTP/2 preface has arrived so far.
    preface: usize,
    write_stall: Duration,
    /// Since a write has been waiting for the peer to read.
    stalled: Option<Pin<Box<Sleep>>>,
}

impl<S> TimedStream<S> {
    fn new(inner: S, first_request: Duration, write_stall: Duration) -> Self {
        TimedStream { inner, first_request: Some(Box::pin(tokio::time::sleep(first_request))), preface: 0, write_stall, stalled: None }
    }

    /// Notes what was read: anything but the HTTP/2 preface means HTTP/1,
    /// all of it HTTP/2, and nothing the end of the connection.
    fn note_read(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            self.first_request = None;
        }
        for &b in bytes {
            if b != H2_PREFACE[self.preface] {
                self.first_request = None;
                return;
            }
            self.preface += 1;
            if self.preface == H2_PREFACE.len() {
                self.first_request = None;
                return;
            }
        }
    }

    /// A write that cannot go ahead fails once it has waited `write_stall`.
    fn watch_write<T>(&mut self, cx: &mut Context<'_>, res: Poll<io::Result<T>>) -> Poll<io::Result<T>> {
        if res.is_ready() {
            self.stalled = None;
            return res;
        }
        let stall = self.write_stall;
        let stalled = self.stalled.get_or_insert_with(|| Box::pin(tokio::time::sleep(stall)));
        if stalled.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(io::Error::new(io::ErrorKind::TimedOut, "the peer stopped reading")));
        }
        Poll::Pending
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for TimedStream<S> {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        if let Some(deadline) = self.first_request.as_mut()
            && deadline.as_mut().poll(cx).is_ready()
        {
            return Poll::Ready(Err(io::Error::new(io::ErrorKind::TimedOut, "no request after the TLS handshake")));
        }
        let before = buf.filled().len();
        let res = Pin::new(&mut self.inner).poll_read(cx, buf);
        if matches!(res, Poll::Ready(Ok(()))) && self.first_request.is_some() {
            self.note_read(&buf.filled()[before..]);
        }
        res
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for TimedStream<S> {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        let res = Pin::new(&mut self.inner).poll_write(cx, buf);
        self.watch_write(cx, res)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let res = Pin::new(&mut self.inner).poll_flush(cx);
        self.watch_write(cx, res)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let res = Pin::new(&mut self.inner).poll_shutdown(cx);
        self.watch_write(cx, res)
    }
    fn poll_write_vectored(mut self: Pin<&mut Self>, cx: &mut Context<'_>, bufs: &[io::IoSlice<'_>]) -> Poll<io::Result<usize>> {
        let res = Pin::new(&mut self.inner).poll_write_vectored(cx, bufs);
        self.watch_write(cx, res)
    }
    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
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

    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn timed(inner: tokio::io::DuplexStream) -> TimedStream<tokio::io::DuplexStream> {
        TimedStream::new(inner, Duration::from_secs(20), Duration::from_secs(60))
    }

    #[tokio::test(start_paused = true)]
    async fn a_connection_must_start_a_request_soon_after_the_handshake() {
        // Nothing at all, or HTTP/2's preface begun and left unfinished.
        for sent in [b"".as_slice(), b"PRI * HTTP/2.0\r\n".as_slice()] {
            let (mut peer, stream) = tokio::io::duplex(1024);
            let mut conn = timed(stream);
            let mut buf = [0; 1024];
            if !sent.is_empty() {
                peer.write_all(sent).await.unwrap();
                assert_eq!(conn.read(&mut buf).await.unwrap(), sent.len());
            }
            let started = tokio::time::Instant::now();
            assert_eq!(conn.read(&mut buf).await.unwrap_err().kind(), io::ErrorKind::TimedOut);
            assert!(started.elapsed() <= Duration::from_secs(20));
        }
        // Once the version is known, an idle connection is hyper's to time.
        for sent in [b"GET / HTTP/1.1\r\n".as_slice(), H2_PREFACE] {
            let (mut peer, stream) = tokio::io::duplex(1024);
            let mut conn = timed(stream);
            let mut buf = [0; 1024];
            peer.write_all(sent).await.unwrap();
            assert_eq!(conn.read(&mut buf).await.unwrap(), sent.len());
            tokio::time::advance(Duration::from_secs(300)).await;
            peer.write_all(b"more").await.unwrap();
            assert_eq!(conn.read(&mut buf).await.unwrap(), 4);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_peer_that_stops_reading_is_dropped() {
        let (mut peer, stream) = tokio::io::duplex(64);
        let mut conn = timed(stream);
        // Reading now and then keeps it going, however long it takes.
        for _ in 0..3 {
            conn.write_all(&[1; 64]).await.unwrap();
            let write = tokio::spawn(async move {
                conn.write_all(&[2; 64]).await.unwrap();
                conn
            });
            tokio::time::sleep(Duration::from_secs(50)).await;
            let mut buf = [0; 128];
            peer.read_exact(&mut buf[..64]).await.unwrap();
            conn = write.await.unwrap();
            peer.read_exact(&mut buf[..64]).await.unwrap();
        }
        // Not reading at all fails the write after a minute.
        conn.write_all(&[1; 64]).await.unwrap();
        let started = tokio::time::Instant::now();
        assert_eq!(conn.write_all(&[2; 64]).await.unwrap_err().kind(), io::ErrorKind::TimedOut);
        assert_eq!(started.elapsed(), Duration::from_secs(60));
    }
}
