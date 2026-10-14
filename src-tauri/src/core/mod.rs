//! The LanShare server: everything except the desktop window.

pub mod auth;
pub mod bridge;
pub mod config;
pub mod db;
pub mod error;
pub mod events;
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
use serde_json::json;

use bridge::HostBridge;
use config::Config;
use db::Db;
use events::Audience;
use paths::Sandbox;
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
    let candidates = (0..10).map(|i| preferred.saturating_add(i)).chain(std::iter::once(0));
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

pub fn host_name() -> String {
    let raw = gethostname::gethostname().to_string_lossy().into_owned();
    let name = raw.trim_end_matches(".local").trim();
    if name.is_empty() { "我的电脑".to_string() } else { name.to_string() }
}

/// Starts both listeners. Must run inside a tokio runtime.
pub async fn start(opts: StartOptions, bridge: Arc<dyn HostBridge>) -> Result<ServerHandle> {
    std::fs::create_dir_all(&opts.data_dir).with_context(|| format!("create {}", opts.data_dir.display()))?;
    let mut config = Config::load_or_default(&opts.data_dir, &opts.home_dir);
    if let Some(root) = opts.shared_root {
        config.shared_root = root;
    }
    if let Some(port) = opts.port {
        config.port = port;
    }
    let sandbox = Sandbox::new(&config.shared_root).with_context(|| format!("open shared folder {}", config.shared_root.display()))?;
    config.shared_root = sandbox.root().to_path_buf();
    if let Err(e) = config.save(&opts.data_dir) {
        tracing::warn!("could not save config: {e}");
    }

    let db = Db::open(&opts.data_dir.join("lanshare.db")).context("open database")?;
    let host_name = host_name();
    let addresses = net::lan_addresses();
    let ips: Vec<IpAddr> = addresses.iter().filter_map(|a| a.ip.parse().ok()).collect();
    let tls = Tls::init(&opts.data_dir.join("tls"), &host_name, tls::local_dns_name(&host_name), &ips).context("prepare certificates")?;

    let lan_listener = bind_lan(config.port).context("bind LAN port")?;
    let lan_port = lan_listener.local_addr()?.port();
    let local_listener = TcpListener::bind((Ipv4Addr::LOCALHOST, opts.local_port)).context("bind loopback port")?;
    local_listener.set_nonblocking(true)?;
    let local_port = local_listener.local_addr()?.port();

    let state = AppState::new(StateParts {
        db,
        data_dir: opts.data_dir.clone(),
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
    let server = axum_server::from_tcp_rustls(lan_listener, state.tls.rustls_config())?;
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
    tracing::info!("LanShare listening on port {lan_port} (LAN, HTTPS) and 127.0.0.1:{local_port} (window)");
    Ok(ServerHandle { state })
}

/// Follows Wi-Fi changes: refreshes the address list and the certificate.
async fn watch_network(state: Shared) {
    let mut tick = tokio::time::interval(Duration::from_secs(20));
    tick.tick().await;
    loop {
        tick.tick().await;
        let Ok(addresses) = tokio::task::spawn_blocking(net::lan_addresses).await else { continue };
        if !state.set_addresses(addresses) {
            continue;
        }
        let st = state.clone();
        let ips = state.address_ips();
        match tokio::task::spawn_blocking(move || st.tls.ensure_ips(&ips)).await {
            Ok(Err(e)) => tracing::error!("could not refresh certificate: {e:#}"),
            Err(e) => tracing::error!("certificate task failed: {e}"),
            Ok(Ok(_)) => {}
        }
        state.emit(Audience::Host, "network", &json!({}));
    }
}
