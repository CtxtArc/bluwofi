use anyhow::{Context, Result};
use bluer::agent::{Agent, ReqError};
use bluer::{Adapter, Address, Session};
use bluwofi::mark_ignored;
use futures::StreamExt;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Global toggle for desktop notifications, set once in main() from the
/// --no-notify flag. Defaults to enabled.
static NOTIFY_ENABLED: AtomicBool = AtomicBool::new(true);

#[derive(Debug, Clone)]
struct MenuDevice {
    address: Address,
    name: String,
    connected: bool,
    paired: bool,
    battery: Option<u8>,
    rssi: Option<i16>,
}

/// Guards against two instances running at once (e.g. double-pressing
/// the sway keybind). Removes its lock file on drop, best-effort.
struct InstanceLock(std::path::PathBuf);

impl Drop for InstanceLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Returns None if another live instance already holds the lock. A lock
/// file left behind by a crashed/killed process (stale PID that's no
/// longer running) is detected and reclaimed automatically.
fn acquire_single_instance_lock() -> Option<InstanceLock> {
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    let path = std::path::PathBuf::from(dir).join("bluwofi.lock");

    if let Ok(existing) = std::fs::read_to_string(&path) {
        if let Ok(pid) = existing.trim().parse::<u32>() {
            if std::path::Path::new(&format!("/proc/{pid}")).exists() {
                return None; // another instance is genuinely alive
            }
        }
        // Stale lock (process no longer exists, or file was garbage) —
        // fall through and reclaim it.
    }

    let pid = std::process::id();
    // If we can't even write the lock file, don't block the user over
    // it — just proceed without the guard rather than refusing to run.
    let _ = std::fs::write(&path, pid.to_string());
    Some(InstanceLock(path))
}

#[tokio::main]
async fn main() -> Result<()> {
    if let Err(e) = run().await {
        // Fatal startup errors (no bluetoothd, no adapter, wofi missing,
        // etc.) need to be visible even when launched from a sway
        // keybind with no terminal attached — otherwise pressing the
        // key just silently does nothing and you're left guessing why.
        // This bypasses --no-notify deliberately: total silent failure
        // is worse than one unwanted toast.
        notify_fatal("bluetooth-wofi failed to start", &format!("{:#}", e));
        eprintln!("{:#}", e);
        std::process::exit(1);
    }
    Ok(())
}

async fn run() -> Result<()> {
    let config = bluwofi::load_config();
    let status_mode = std::env::args().any(|a| a == "--status");
    if std::env::args().any(|a| a == "--no-notify") {
        NOTIFY_ENABLED.store(false, Ordering::Relaxed);
    }

    let session = Session::new()
        .await
        .context("failed to connect to bluetoothd (is bluetooth.service running?)")?;
    let adapter = session
        .default_adapter()
        .await
        .context("no bluetooth adapter found")?;

    if status_mode {
        // --status is a quick poll from waybar every few seconds — don't
        // gate it behind the single-instance lock, or the menu and the
        // status poll would fight over it.
        return print_status(&adapter).await;
    }

    let _lock = match acquire_single_instance_lock() {
        Some(lock) => lock,
        None => {
            // Not a real error — just don't open a second menu on top
            // of one that's already open.
            notify("Bluetooth", "Menu is already open", false);
            return Ok(());
        }
    };

    // Register a pairing agent so BlueZ has somewhere to send confirmation
    // requests. Without this, pairing with anything that isn't pure
    // "Just Works" fails with a generic authentication error.
    let agent = Agent {
        request_default: true,
        request_pin_code: None,
        display_pin_code: None,
        request_passkey: None,
        request_confirmation: Some(Box::new(|req| {
            Box::pin(async move {
                let prompt = format!(
                    "Confirm pairing with {}? Passkey: {:06}",
                    req.device, req.passkey
                );
                let lines = vec!["Yes, confirm".to_string(), "No, reject".to_string()];
                let choice = tokio::task::spawn_blocking(move || run_wofi(&lines, &prompt))
                    .await
                    .unwrap_or(Ok(None));
                match choice {
                    Ok(Some(c)) if c.starts_with("Yes") => Ok(()),
                    _ => Err(ReqError::Rejected),
                }
            })
        })),
        display_passkey: Some(Box::new(|req| {
            Box::pin(async move {
                notify(
                    "Bluetooth pairing",
                    &format!("Passkey for {}: {:06}", req.device, req.passkey),
                    false,
                );
                Ok(())
            })
        })),
        request_authorization: Some(Box::new(|_req| Box::pin(async move { Ok(()) }))),
        authorize_service: Some(Box::new(|_req| Box::pin(async move { Ok(()) }))),
        ..Default::default()
    };
    // Keep the handle alive for the lifetime of main() — dropping it
    // unregisters the agent.
    let _agent_handle = session
        .register_agent(agent)
        .await
        .context("failed to register bluetooth pairing agent")?;

    loop {
        let powered = adapter.is_powered().await?;
        let devices = list_devices(&adapter).await?;

        let mut lines = Vec::new();
        lines.push(format!(
            "{}  Power: {}",
            if powered { "\u{1F535}" } else { "\u{26AA}" },
            if powered {
                "ON (click to turn off)"
            } else {
                "OFF (click to turn on)"
            }
        ));
        lines.push("\u{1F50D}  Scan for devices".to_string());

        // Build (line, device) pairs so we can match the selection back
        // to a device by exact line equality rather than a substring
        // check — substring matching (e.g. selection.contains(&d.name))
        // silently picks the wrong device whenever one name is a
        // substring of another ("AirPods" vs "AirPods Pro").
        let device_lines: Vec<(String, &MenuDevice)> =
            devices.iter().map(|d| (device_menu_line(d), d)).collect();
        for (line, _) in &device_lines {
            lines.push(line.clone());
        }

        let selection = match run_wofi(&lines, "Bluetooth")? {
            Some(s) => s,
            None => break, // user hit escape
        };

        if selection.contains("Power:") {
            adapter.set_powered(!powered).await?;
            continue;
        }

        if selection.contains("Scan for devices") {
            scan_devices(&adapter, config.scan_duration_secs).await?;
            continue;
        }

        if let Some((_, dev)) = device_lines.iter().find(|(line, _)| *line == selection) {
            if let Err(e) =
                handle_device_action(&adapter, dev, config.reconnect_cooldown_secs).await
            {
                eprintln!("{}", friendly_error(&dev.name, &e));
            }
        }
    }

    Ok(())
}

/// Exact same line format used both when building the wofi menu and when
/// matching the selection back to a device — keeping this in one place
/// means the two can never drift out of sync.
fn device_menu_line(d: &MenuDevice) -> String {
    let status = match (d.connected, d.paired) {
        (true, _) => "connected",
        (false, true) => "paired",
        (false, false) => "available",
    };
    let battery_suffix = match d.battery {
        Some(pct) => format!(", {}%", pct),
        None => String::new(),
    };
    // BlueZ only keeps RSSI fresh from scan/advertising data — once a
    // device is connected it's no longer being actively scanned, so the
    // value goes stale/empty. Only show it for devices we're not
    // already connected to.
    let signal_suffix = match (d.connected, d.rssi) {
        (false, Some(rssi)) => format!(", {} ({}dBm)", signal_label(rssi), rssi),
        _ => String::new(),
    };
    format!(
        "{}  {} [{}{}{}]",
        device_icon(d),
        d.name,
        status,
        battery_suffix,
        signal_suffix
    )
}

fn device_icon(d: &MenuDevice) -> &'static str {
    if d.connected {
        "\u{1F7E2}"
    } else if d.paired {
        "\u{1F7E1}"
    } else {
        "\u{26AB}"
    }
}

/// Rough, commonly-used thresholds for Bluetooth RSSI (dBm). These are
/// approximate — actual usable range varies by device/antenna — but
/// good enough for an at-a-glance label.
fn signal_label(rssi: i16) -> &'static str {
    if rssi >= -50 {
        "Excellent"
    } else if rssi >= -60 {
        "Good"
    } else if rssi >= -70 {
        "Fair"
    } else {
        "Weak"
    }
}

/// Prints a single JSON line in the shape waybar's `custom` module expects
/// ({"text", "tooltip", "class"}), then exits. Meant to be invoked
/// repeatedly by waybar's polling interval, not the interactive loop.
async fn print_status(adapter: &Adapter) -> Result<()> {
    let powered = adapter.is_powered().await.unwrap_or(false);

    let (text, tooltip, class) = if !powered {
        (
            "\u{f294}".to_string(),
            "Bluetooth off".to_string(),
            "disabled",
        )
    } else {
        let devices = list_devices(adapter).await.unwrap_or_default();
        let connected: Vec<&MenuDevice> = devices.iter().filter(|d| d.connected).collect();

        if connected.is_empty() {
            (
                "\u{f293}".to_string(),
                "Bluetooth on, nothing connected".to_string(),
                "on",
            )
        } else {
            let names: Vec<String> = connected
                .iter()
                .map(|d| match d.battery {
                    Some(pct) => format!("{} ({}%)", d.name, pct),
                    None => d.name.clone(),
                })
                .collect();
            let text = if connected.len() == 1 {
                format!("\u{f293} {}", connected[0].name)
            } else {
                format!("\u{f293} {}", connected.len())
            };
            (text, names.join("\n"), "connected")
        }
    };

    println!(
        "{}",
        serde_json::json!({ "text": text, "tooltip": tooltip, "class": class })
    );
    Ok(())
}

async fn list_devices(adapter: &Adapter) -> Result<Vec<MenuDevice>> {
    let mut result = Vec::new();
    for addr in adapter.device_addresses().await? {
        let device = adapter.device(addr)?;
        let name = device
            .name()
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| addr.to_string());
        // Bluetooth device names are attacker-controlled (any nearby
        // device can advertise whatever name it wants) and we build the
        // wofi menu by joining lines with "\n" — so a name containing a
        // newline could inject a fake extra menu entry. Strip control
        // characters defensively.
        let name: String = name.chars().filter(|c| !c.is_control()).collect();
        let connected = device.is_connected().await.unwrap_or(false);
        let paired = device.is_paired().await.unwrap_or(false);
        // battery_percentage() returns Ok(None) for devices that don't
        // expose org.bluez.Battery1 (most things that aren't headsets/
        // earbuds/some mice), so this is safe to call unconditionally.
        let battery = device.battery_percentage().await.unwrap_or(None);
        // rssi() is usually only populated for devices seen recently
        // during a scan — BlueZ doesn't keep it fresh for a device
        // that's just sitting connected without an active discovery
        // session, so this will often be None and that's expected.
        let rssi = device.rssi().await.unwrap_or(None);
        result.push(MenuDevice {
            address: addr,
            name,
            connected,
            paired,
            battery,
            rssi,
        });
    }
    result.sort_by(|a, b| b.connected.cmp(&a.connected).then(a.name.cmp(&b.name)));
    Ok(result)
}

async fn scan_devices(adapter: &Adapter, scan_duration_secs: u64) -> Result<()> {
    adapter.set_powered(true).await.ok();

    // Keep a transient "Scanning..." wofi window open for the duration of
    // the scan so the menu doesn't appear to close and reopen.
    let mut indicator = Command::new("wofi")
        .args([
            "--dmenu",
            "--prompt",
            "Bluetooth",
            "--insensitive",
            "--cache-file",
            "/dev/null",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .ok();

    if let Some(child) = indicator.as_mut() {
        if let Some(stdin) = child.stdin.as_mut() {
            let _ = stdin.write_all("\u{1F50D}  Scanning...".as_bytes());
        }
    }

    let mut events = adapter.discover_devices().await?;
    let deadline = tokio::time::sleep(Duration::from_secs(scan_duration_secs));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => break,
            ev = events.next() => {
                if ev.is_none() {
                    break;
                }
            }
        }
    }

    if let Some(mut child) = indicator {
        let _ = child.kill();
        let _ = child.wait();
    }

    Ok(())
}

async fn handle_device_action(
    adapter: &Adapter,
    dev: &MenuDevice,
    reconnect_cooldown_secs: u64,
) -> Result<()> {
    let device = adapter.device(dev.address)?;

    let mut actions = Vec::new();
    if dev.connected {
        actions.push("Disconnect".to_string());
    } else if dev.paired {
        actions.push("Connect".to_string());
    } else {
        actions.push("Pair & Connect".to_string());
    }
    if dev.paired {
        actions.push("Remove / Forget".to_string());
    }
    actions.push("Cancel".to_string());

    let choice = match run_wofi(&actions, &dev.name)? {
        Some(c) => c,
        None => return Ok(()),
    };

    let result = match choice.as_str() {
        "Disconnect" => device.disconnect().await.context("disconnect failed"),
        "Connect" => device.connect().await.context("connect failed"),
        "Pair & Connect" => {
            async {
                device.pair().await.context("pairing failed")?;
                device
                    .set_trusted(true)
                    .await
                    .context("failed to trust device")?;
                device
                    .connect()
                    .await
                    .context("connect after pairing failed")
            }
            .await
        }
        "Remove / Forget" => adapter
            .remove_device(dev.address)
            .await
            .context("failed to remove device"),
        _ => Ok(()),
    };

    match &result {
        Ok(()) => {
            let verb = match choice.as_str() {
                "Disconnect" => "Disconnected from",
                "Connect" => "Connected to",
                "Pair & Connect" => "Paired and connected to",
                "Remove / Forget" => "Forgot",
                _ => "",
            };
            if !verb.is_empty() {
                notify(&format!("Bluetooth: {}", verb), &dev.name, false);
            }
            if choice == "Disconnect" {
                // Tell bluetooth-reconnectd to leave this device alone for
                // a while — otherwise it sees "trusted + disconnected"
                // and reconnects within one poll cycle, fighting a
                // deliberate disconnect.
                mark_ignored(dev.address, reconnect_cooldown_secs);
            }
        }
        Err(e) => {
            notify("Bluetooth error", &friendly_error(&dev.name, e), true);
        }
    }

    result?;
    Ok(())
}

/// Turns a raw anyhow error chain (which usually bottoms out in a BlueZ
/// D-Bus error like "org.bluez.Error.AuthenticationFailed: Authentication
/// Failed") into the full technical message plus a plain-English hint,
/// matched by keyword rather than by BlueZ error type — this keeps it
/// working across bluer/BlueZ versions without depending on exact enum
/// variants.
fn friendly_error(context: &str, e: &anyhow::Error) -> String {
    let raw = format!("{:#}", e);
    let lower = raw.to_lowercase();

    let hint = if lower.contains("authentication failed") {
        "The device may still think it's already paired with this machine. Try forgetting/resetting pairing on the device itself, not just here, then pair again."
    } else if lower.contains("already exists") {
        "This device is already paired. Try Connect instead of Pair, or Remove it first if you want a clean re-pair."
    } else if lower.contains("does not exist") {
        "BlueZ has no record of this device anymore. Try scanning again."
    } else if lower.contains("host is down")
        || lower.contains("connection attempt failed")
        || lower.contains("connection abort")
    {
        "The device didn't respond. Make sure it's powered on, in range, and not already connected to something else (like your phone)."
    } else if lower.contains("in progress") {
        "Another Bluetooth operation is already running. Wait a moment and try again."
    } else if lower.contains("not ready") {
        "The Bluetooth adapter isn't ready yet. Give it a second and retry."
    } else if lower.contains("timeout") {
        "The device didn't respond in time. Check it's powered on, charged, and in pairing mode if this is a first-time pair."
    } else if lower.contains("rejected") || lower.contains("cancel") {
        "The pairing request was rejected or cancelled, either by you or the device."
    } else if lower.contains("not authorized") || lower.contains("permission") {
        "Permission denied by BlueZ. Check that your user has rights to manage Bluetooth (polkit rules)."
    } else {
        ""
    };

    if hint.is_empty() {
        format!("{}: {}", context, raw)
    } else {
        format!("{}: {}\n\n{}", context, raw, hint)
    }
}

/// Fire a desktop notification via `notify-send`, unless disabled via
/// --no-notify. Best-effort — if notify-send isn't installed we just
/// skip it rather than failing the whole action.
fn notify(summary: &str, body: &str, urgent: bool) {
    if !NOTIFY_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let mut cmd = Command::new("notify-send");
    cmd.arg("--app-name=bluwofi");
    if urgent {
        cmd.args(["--urgency", "critical"]);
    }
    cmd.args([summary, body]);
    let _ = cmd.stdout(Stdio::null()).stderr(Stdio::null()).spawn();
}

/// Same as notify(), but ignores --no-notify. Reserved for fatal
/// startup failures — when launched from a sway keybind with no
/// terminal attached, this notification is the *only* feedback the
/// user gets that something went wrong, so it must not be suppressible.
fn notify_fatal(summary: &str, body: &str) {
    let _ = Command::new("notify-send")
        .arg("--app-name=bluwofi")
        .args(["--urgency", "critical"])
        .args([summary, body])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

/// Pipe `lines` into wofi as a dmenu list and return the selected line
/// (None if the user pressed escape / closed the menu).
fn run_wofi(lines: &[String], prompt: &str) -> Result<Option<String>> {
    let mut child = Command::new("wofi")
        .args([
            "--dmenu",
            "--prompt",
            prompt,
            "--insensitive",
            "--cache-file",
            "/dev/null",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("failed to launch wofi (is it installed and on PATH?)")?;

    {
        let stdin = child.stdin.as_mut().context("failed to open wofi stdin")?;
        stdin.write_all(lines.join("\n").as_bytes())?;
    }

    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Ok(None);
    }
    let selection = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(if selection.is_empty() {
        None
    } else {
        Some(selection)
    })
}
