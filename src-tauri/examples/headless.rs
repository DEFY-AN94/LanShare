//! Runs the LanShare server without a window (for development and testing).
//!
//! cargo run --example headless -- [--data DIR] [--shared DIR] [--port N] [--local-port N]

use std::path::PathBuf;
use std::sync::Arc;

use lanshare_lib::core::bridge::HeadlessBridge;
use lanshare_lib::core::{self, StartOptions};

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    lanshare_lib::init_logging();
    let args: Vec<String> = std::env::args().collect();
    let cwd = std::env::current_dir()?;
    // Relative paths are resolved against the current directory.
    let data_dir = arg(&args, "--data").map(|p| cwd.join(p)).unwrap_or_else(|| cwd.join(".dev-data/app"));
    let shared_root = Some(arg(&args, "--shared").map(|p| cwd.join(p)).unwrap_or_else(|| cwd.join(".dev-data/shared")));
    let home_dir = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_else(|| cwd.clone());
    let port = arg(&args, "--port").and_then(|p| p.parse().ok());
    let local_port = arg(&args, "--local-port").and_then(|p| p.parse().ok()).unwrap_or(0);

    let handle = core::start(StartOptions { data_dir, home_dir, shared_root, port, local_port }, Arc::new(HeadlessBridge)).await?;
    println!("LanShare (headless) is running");
    for url in handle.lan_urls() {
        println!("  phone:   {url}");
    }
    println!("  host UI: {}", handle.login_url());
    println!("Press Ctrl+C to stop.");
    tokio::signal::ctrl_c().await?;
    Ok(())
}
