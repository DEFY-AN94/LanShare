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
    /// Kept across restarts, so a pause set on an untrusted network is not
    /// quietly undone by relaunching (or, later, by launch at login).
    #[serde(default)]
    pub paused: bool,
    /// Chat messages older than this many days are deleted; 0 keeps them.
    #[serde(default)]
    pub chat_retention_days: u32,
    /// Launch at login was suggested once (after the first pairing) and is not suggested again.
    #[serde(default)]
    pub login_item_suggested: bool,
    /// Cutoff (ms) of the last automatic cleanup: phones that were away, also
    /// across a restart, drop what they still show from before it.
    #[serde(default)]
    pub pruned_before: i64,
    /// Highest message id the last "clear all chats" removed, for the same
    /// purpose. Ids, not times, so a clock that was off cannot hide later messages.
    #[serde(default)]
    pub cleared_through: i64,
}

/// The choices Settings offers for automatic chat cleanup.
pub const RETENTION_CHOICES: [u32; 3] = [0, 30, 90];

fn default_port() -> u16 {
    DEFAULT_PORT
}

impl Config {
    fn file(data_dir: &Path) -> PathBuf {
        data_dir.join("config.json")
    }

    /// Loads `config.json`, falling back to `~/LanShare` on port 39210.
    pub fn load_or_default(data_dir: &Path, home: &Path) -> Config {
        let fallback = Config { shared_root: home.join("LanShare"), port: DEFAULT_PORT, paused: false, chat_retention_days: 0, login_item_suggested: false, pruned_before: 0, cleared_through: 0 };
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
