//! The LanShare server: everything except the desktop window.

pub mod auth;
pub mod bridge;
pub mod config;
pub mod db;
pub mod error;
pub mod events;
mod guard;
pub mod model;
pub mod net;
pub mod pairing;
pub mod paths;
mod routes;
pub mod state;
pub mod tls;
mod transfer;
pub mod util;

#[cfg(test)]
mod tests;

use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum_server::tls_rustls::RustlsAcceptor;
use hyper_util::rt::TokioTimer;
use notify::Watcher;
use serde_json::json;

use bridge::HostBridge;
use config::Config;
use db::Db;
use events::Audience;
use paths::{Sandbox, unsafe_root_reason};
use state::{AppState, Shared, StateParts};
use tls::Tls;

pub struct StartOptions {
    pub data_dir: PathBuf,
    pub home_dir: PathBuf,
    /// Overrides the configured shared folder (used by the headless runner).
    pub shared_root: Option<PathBuf>,
    /// Overrides the configured LAN port.
    pub port: Option<u16>,
    /// Fixed loopback port (0 = any free port).
    pub local_port: u16,
}

pub struct ServerHandle {
    pub state: Shared,
}

impl ServerHandle {
    /// URL the desktop window opens to sign in as the host.
    pub fn login_url(&self) -> String {
        format!("http://127.0.0.1:{}/__local_login?token={}", self.state.local_port, self.state.local_token)
    }

    pub fn local_origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.state.local_port)
    }

    pub fn lan_urls(&self) -> Vec<String> {
        self.state.addresses().iter().map(|a| format!("https://{}:{}", a.ip, self.state.lan_port)).collect()
    }
}

fn bind_lan(preferred: u16) -> std::io::Result<TcpListener> {
    // Nearby ports first, then jump past whole reserved blocks, then any port.
    let near = (0..10).map(move |i| preferred.saturating_add(i));
    let far = (1..10).map(move |i| preferred.saturating_add(i * 100));
    let candidates = near.chain(far).chain(std::iter::once(0));
    let mut last_err = None;
    for port in candidates {
        match TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)) {
            Ok(l) => {
                l.set_nonblocking(true)?;
                if port != preferred {
                    tracing::warn!("port {preferred} is busy; using {}", l.local_addr()?.port());
                }
                return Ok(l);
            }
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| std::io::Error::other("no free port")))
}

/// The name phones and new CA certificates show for this computer.
pub fn host_name() -> String {
    pick_host_name(net::computer_name().as_deref(), &gethostname::gethostname().to_string_lossy())
}

/// The macOS Computer Name is what the user picked ("MyMac"); the hostname
/// may be one a DHCP server handed out ("Mac"). Elsewhere the hostname is the
/// computer's name.
fn pick_host_name(computer_name: Option<&str>, hostname: &str) -> String {
    let name = match computer_name.map(str::trim) {
        Some(n) if !n.is_empty() => n,
        _ => hostname.trim().trim_end_matches(".local").trim(),
    };
    if name.is_empty() { "我的电脑".to_string() } else { name.to_string() }
}

/// Opens the configured shared folder, falling back to ~/LanShare when it is
/// missing (unplugged drive, unmounted share) or unsafe to expose.
fn open_shared_folder(config: &mut Config, home: &std::path::Path, data_dir: &std::path::Path) -> Result<Sandbox> {
    let default = home.join("LanShare");
    let configured = config.shared_root.clone();
    let usable = if configured == default {
        None
    } else if let Some(reason) = unsafe_root_reason(&configured, home, data_dir) {
        tracing::warn!("shared folder {} is not allowed ({reason}); using {}", configured.display(), default.display());
        None
    } else {
        match Sandbox::existing(&configured) {
            Ok(sb) => Some(sb),
            // Folder deleted (or never created) but its parent is there: recreate it.
            Err(_) if configured.is_absolute() && configured.parent().is_some_and(|p| p.is_dir()) => match Sandbox::new(&configured) {
                Ok(sb) => Some(sb),
                Err(e) => {
                    tracing::warn!("cannot create shared folder {} ({e}); using {} for now", configured.display(), default.display());
                    None
                }
            },
            Err(e) => {
                // e.g. an unplugged drive. Not saved over: the folder comes back once it is reconnected.
                tracing::warn!("shared folder {} is unavailable ({e}); using {} for now", configured.display(), default.display());
                None
            }
        }
    };
    let sandbox = match usable {
        Some(sb) => sb,
        None => {
            let sb = Sandbox::new(&default).with_context(|| format!("create {}", default.display()))?;
            if configured == default || unsafe_root_reason(&configured, home, data_dir).is_some() {
                config.shared_root = sb.root().to_path_buf();
            }
            sb
        }
    };
    Ok(sandbox.with_deny(&[data_dir.to_path_buf()]))
}

/// Starts both listeners. Must run inside a tokio runtime.
pub async fn start(opts: StartOptions, bridge: Arc<dyn HostBridge>) -> Result<ServerHandle> {
    std::fs::create_dir_all(&opts.data_dir).with_context(|| format!("create {}", opts.data_dir.display()))?;
    guard::raise_fd_limit();
    let mut config = Config::load_or_default(&opts.data_dir, &opts.home_dir);
    if let Some(root) = opts.shared_root {
        config.shared_root = root;
    }
    if let Some(port) = opts.port {
        config.port = port;
    }
    let sandbox = open_shared_folder(&mut config, &opts.home_dir, &opts.data_dir)?;
    if let Err(e) = config.save(&opts.data_dir) {
        tracing::warn!("could not save config: {e}");
    }

    let db = Db::open(&opts.data_dir.join("lanshare.db")).context("open database")?;
    let host_name = host_name();
    let addresses = net::lan_addresses();
    let ips: Vec<IpAddr> = addresses.iter().filter_map(|a| a.ip.parse().ok()).collect();
    let tls = Tls::init(&opts.data_dir.join("tls"), &host_name, net::local_dns_name(), &ips).context("prepare certificates")?;

    let lan_listener = bind_lan(config.port).context("bind LAN port")?;
    let lan_port = lan_listener.local_addr()?.port();
    if lan_port != config.port {
        // Keep using the port that worked so phone bookmarks survive restarts.
        config.port = lan_port;
        let _ = config.save(&opts.data_dir);
    }
    let local_listener = TcpListener::bind((Ipv4Addr::LOCALHOST, opts.local_port)).context("bind loopback port")?;
    local_listener.set_nonblocking(true)?;
    let local_port = local_listener.local_addr()?.port();

    let state = AppState::new(StateParts {
        db,
        data_dir: opts.data_dir.clone(),
        home_dir: opts.home_dir.clone(),
        config,
        sandbox,
        local_token: util::random_token(),
        local_port,
        lan_port,
        tls,
        bridge,
        host_name,
        addresses,
    });

    let lan_app = routes::lan_router(state.clone());
    // Non-LAN peers and connection floods are cut off before the TLS handshake.
    let acceptor =
        guard::LogHandshakeFailures(RustlsAcceptor::new(state.tls.rustls_config()).acceptor(guard::LanAcceptor::default()));
    let mut server = axum_server::from_tcp(lan_listener)?.acceptor(acceptor);
    server.http_builder().http1().timer(TokioTimer::new()).header_read_timeout(Duration::from_secs(20));
    server
        .http_builder()
        .http2()
        .timer(TokioTimer::new())
        .keep_alive_interval(Duration::from_secs(30))
        .keep_alive_timeout(Duration::from_secs(20));
    tokio::spawn(async move {
        if let Err(e) = server.serve(lan_app.into_make_service_with_connect_info::<SocketAddr>()).await {
            tracing::error!("LAN server stopped: {e}");
        }
    });

    let local = tokio::net::TcpListener::from_std(local_listener)?;
    let local_app = routes::local_router(state.clone());
    tokio::spawn(async move {
        if let Err(e) = axum::serve(local, local_app.into_make_service_with_connect_info::<SocketAddr>()).await {
            tracing::error!("loopback server stopped: {e}");
        }
    });

    tokio::spawn(watch_network(state.clone()));
    watch_shared_folder(&state);
    let root = state.sandbox().root().to_path_buf();
    tokio::task::spawn_blocking(move || transfer::sweep_tree(&root, 4));
    tracing::info!("LanShare listening on port {lan_port} (LAN, HTTPS) and 127.0.0.1:{local_port} (window)");
    Ok(ServerHandle { state })
}

/// (Re)starts watching the shared folder so edits made on the computer —
/// in Finder or any other app — refresh open file lists on every device.
pub fn watch_shared_folder(state: &Shared) {
    let root = state.sandbox().root().to_path_buf();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<std::path::PathBuf>();
    let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            for p in ev.paths {
                let _ = tx.send(p);
            }
        }
    })
    .and_then(|mut w| w.watch(&root, notify::RecursiveMode::Recursive).map(|_| w));
    let watcher = match watcher {
        Ok(w) => w,
        Err(e) => {
            tracing::warn!("cannot watch shared folder: {e}");
            return;
        }
    };
    // Replacing the old watcher drops it, which closes its channel and ends its task.
    *state.watcher.lock().unwrap() = Some(watcher);
    let st = state.clone();
    tokio::spawn(async move {
        while let Some(first) = rx.recv().await {
            // Coalesce bursts (copying a folder fires many events).
            let mut changed = vec![first];
            tokio::time::sleep(Duration::from_millis(300)).await;
            while let Ok(p) = rx.try_recv() {
                changed.push(p);
            }
            let sandbox = st.sandbox();
            let mut dirs: Vec<String> = changed
                .iter()
                .filter(|p| !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')))
                .filter_map(|p| p.parent().and_then(|d| sandbox.relative(d)))
                .collect();
            dirs.sort();
            dirs.dedup();
            for dir in dirs {
                st.emit_files_changed(&dir);
            }
        }
    });
}

/// Follows Wi-Fi changes and renames of the computer (address list +
/// certificate) and renews the certificate before it expires.
async fn watch_network(state: Shared) {
    let mut tick = tokio::time::interval(Duration::from_secs(20));
    let mut ticks: u64 = 0;
    tick.tick().await;
    loop {
        tick.tick().await;
        ticks += 1;
        if ticks.is_multiple_of(180) {
            let st = state.clone();
            if let Ok(Err(e)) = tokio::task::spawn_blocking(move || st.tls.ensure_fresh()).await {
                tracing::error!("certificate renewal failed: {e:#}");
            }
        }
        let Ok((addresses, dns)) = tokio::task::spawn_blocking(|| (net::lan_addresses(), net::local_dns_name())).await else { continue };
        let moved = state.set_addresses(addresses);
        if !moved && dns.as_ref().is_none_or(|d| state.tls.leaf_dns().as_ref() == Some(d)) {
            continue;
        }
        let st = state.clone();
        let ips = state.address_ips();
        match tokio::task::spawn_blocking(move || st.tls.ensure_names(dns, &ips)).await {
            Ok(Err(e)) => tracing::error!("could not refresh certificate: {e:#}"),
            Err(e) => tracing::error!("certificate task failed: {e}"),
            Ok(Ok(_)) => {}
        }
        if moved {
            state.emit(Audience::Host, "network", &json!({}));
        }
    }
}
