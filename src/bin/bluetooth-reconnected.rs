use anyhow::{Context, Result};
use bluer::{Adapter, Session};
use bluwofi::is_ignored;
use std::process::{Command, Stdio};
use std::time::Duration;

/// Background daemon: every poll interval (from config), look for devices
/// that are paired + trusted but not currently connected, and try to
/// reconnect them. Meant to run as a systemd --user service (see
/// ../systemd/). Config comes from ~/.config/bluetooth-wofi/config.toml,
/// shared with the interactive bluetooth-wofi binary.
#[tokio::main]
async fn main() -> Result<()> {
    let config = bluwofi::load_config();

    let session = Session::new()
        .await
        .context("failed to connect to bluetoothd (is bluetooth.service running?)")?;
    let adapter = session
        .default_adapter()
        .await
        .context("no bluetooth adapter found")?;

    eprintln!(
        "bluetooth-reconnectd started, polling every {}s",
        config.daemon_poll_interval_secs
    );

    loop {
        if let Err(e) = reconnect_pass(&adapter, config.daemon_connect_timeout_secs).await {
            eprintln!("reconnect pass failed: {:#}", e);
        }
        tokio::time::sleep(Duration::from_secs(config.daemon_poll_interval_secs)).await;
    }
}

async fn reconnect_pass(adapter: &Adapter, connect_timeout_secs: u64) -> Result<()> {
    if !adapter.is_powered().await.unwrap_or(false) {
        return Ok(());
    }

    for addr in adapter.device_addresses().await? {
        let device = adapter.device(addr)?;

        let paired = device.is_paired().await.unwrap_or(false);
        let trusted = device.is_trusted().await.unwrap_or(false);
        let connected = device.is_connected().await.unwrap_or(false);

        if !paired || !trusted || connected {
            continue;
        }

        if is_ignored(addr) {
            continue;
        }

        let name = device
            .name()
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| addr.to_string());

        let attempt =
            tokio::time::timeout(Duration::from_secs(connect_timeout_secs), device.connect()).await;

        match attempt {
            Ok(Ok(())) => {
                eprintln!("reconnected to {}", name);
                notify(&format!("Reconnected to {}", name));
            }
            Ok(Err(e)) => {
                // Expected most of the time (device just out of range) —
                // keep it quiet, this isn't worth a desktop notification.
                eprintln!("could not reconnect to {}: {}", name, e);
            }
            Err(_) => {
                eprintln!("timed out reconnecting to {}", name);
            }
        }
    }

    Ok(())
}

/// Best-effort desktop notification — silently does nothing if
/// notify-send isn't installed.
fn notify(body: &str) {
    let _ = Command::new("notify-send")
        .arg("--app-name=bluetooth-reconnectd")
        .args(["Bluetooth", body])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}
