//! Persistent settings stored as `config.json` in the app data directory.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Below 49152 so it stays clear of the dynamic/ephemeral range that
/// Windows (Hyper-V/WSL) carves exclusion blocks out of.
pub const DEFAULT_PORT: u16 = 39210;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    pub shared_root: PathBuf,
    #[serde(default = "default_port")]
    pub port: u16,
}

fn default_port() -> u16 {
    DEFAULT_PORT
}

impl Config {
    fn file(data_dir: &Path) -> PathBuf {
        data_dir.join("config.json")
    }

    /// Loads `config.json`, falling back to `~/LanShare` on port 39210.
    pub fn load_or_default(data_dir: &Path, home: &Path) -> Config {
        let fallback = Config { shared_root: home.join("LanShare"), port: DEFAULT_PORT };
        match std::fs::read(Self::file(data_dir)) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                tracing::warn!("config.json is unreadable ({e}); using defaults");
                fallback
            }),
            Err(_) => fallback,
        }
    }

    pub fn save(&self, data_dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(data_dir)?;
        let tmp = data_dir.join("config.json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(tmp, Self::file(data_dir))
    }
}
