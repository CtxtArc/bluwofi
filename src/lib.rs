use serde::Deserialize;
use std::path::PathBuf;

/// Config file lives at ~/.config/bluwofi/config.toml. Every field
/// is optional in the file itself — anything left out falls back to the
/// Default impl below, so an empty or missing file is totally fine.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// How long `scan_devices()` keeps discovery open, in seconds.
    pub scan_duration_secs: u64,
    /// After a manual disconnect via the menu, how long
    /// bluetooth-reconnectd should leave that device alone, in seconds.
    pub reconnect_cooldown_secs: u64,
    /// How often bluetooth-reconnectd sweeps for disconnected trusted
    /// devices, in seconds.
    pub daemon_poll_interval_secs: u64,
    /// Per-device connect() timeout during a daemon sweep, in seconds.
    pub daemon_connect_timeout_secs: u64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            scan_duration_secs: 5,
            reconnect_cooldown_secs: 120,
            daemon_poll_interval_secs: 20,
            daemon_connect_timeout_secs: 8,
        }
    }
}

fn config_path() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
            PathBuf::from(home).join(".config")
        });
    base.join("bluetooth-wofi").join("config.toml")
}

/// Loads config.toml if present, falling back to defaults for anything
/// missing or if the file doesn't exist at all. Malformed TOML is
/// reported to stderr and treated as "use defaults" rather than a hard
/// failure — a typo in your config shouldn't stop the whole tool from
/// working.
pub fn load_config() -> Config {
    let path = config_path();
    match std::fs::read_to_string(&path) {
        Ok(contents) => match toml::from_str(&contents) {
            Ok(cfg) => cfg,
            Err(e) => {
                eprintln!(
                    "warning: failed to parse {} ({e}), using defaults",
                    path.display()
                );
                Config::default()
            }
        },
        Err(_) => Config::default(), // no config file — defaults are fine
    }
}

/// Directory used to hand off "leave this device alone for a while"
/// markers between bluetooth-wofi and bluetooth-reconnectd. One file per
/// device address, containing a unix timestamp (seconds) up to which the
/// daemon should skip that device.
pub fn ignore_dir() -> PathBuf {
    let base = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(base).join("bluwofi-ignore")
}

/// Called by bluetooth-wofi after a successful manual disconnect.
pub fn mark_ignored(addr: bluer::Address, cooldown_secs: u64) {
    let dir = ignore_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let _ = std::fs::write(
        dir.join(addr.to_string()),
        (now + cooldown_secs).to_string(),
    );
}

/// Called by bluetooth-reconnectd before attempting a reconnect. True if
/// this device has an unexpired marker. Cleans up expired markers as it
/// finds them.
pub fn is_ignored(addr: bluer::Address) -> bool {
    let path = ignore_dir().join(addr.to_string());
    let Ok(content) = std::fs::read_to_string(&path) else {
        return false;
    };
    let Ok(until) = content.trim().parse::<u64>() else {
        let _ = std::fs::remove_file(&path);
        return false;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if now >= until {
        let _ = std::fs::remove_file(&path);
        false
    } else {
        true
    }
}
