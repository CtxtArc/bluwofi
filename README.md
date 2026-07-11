# bluwofi

A minimal Bluetooth manager for Sway/wlroots, driven by a `wofi` popup menu.
Talks directly to BlueZ over D-Bus using the `bluer` crate — no polkit
agent or GTK dependency required.

## Configuration

Optional config file at `~/.config/bluwofi/config.toml`. Every
field is optional — omit any you don't want to override:

```toml
# How long a manual "Scan for devices" stays open, in seconds.
scan_duration_secs = 5

# After you manually disconnect a device from the menu, how long
# bluetooth-reconnectd leaves it alone before trying to reconnect, in
# seconds.
reconnect_cooldown_secs = 120

# How often bluetooth-reconnectd sweeps for disconnected trusted
# devices, in seconds.
daemon_poll_interval_secs = 20

# Per-device connect() timeout during a daemon sweep, in seconds.
daemon_connect_timeout_secs = 8
```

No file, or a file missing some fields? Everything just falls back to
the defaults shown above. Both `bluwofi` and `bluetooth-reconnectd`
read the same file, so cooldown/timeout settings stay consistent between
the interactive menu and the background daemon.

## Requirements

- `bluetoothd` (BlueZ) running (`systemctl status bluetooth`)
- `wofi` installed and on `$PATH`
- `notify-send` (usually provided by `libnotify` or a notification daemon like `mako` on sway) — optional, used for connect/disconnect/pair/error toasts. If missing, notifications are silently skipped.
- Rust toolchain (`rustup`)

## Build

```sh
cd bluwofi
cargo build --release
```

The binary will be at `target/release/bluwofi`.


## Usage

Run it directly, or bind it to a key in your sway config:

```
# ~/.config/sway/config
bindsym $mod+b exec /path/to/bluwofi/target/release/bluwofi
```

Opening it shows:
- A toggle for adapter power
- "Scan for devices" (5 second discovery window, then reopens the menu)
- Known/paired devices, sorted connected-first

Selecting a device opens a second menu with Connect / Disconnect /
Pair & Connect / Remove, depending on its current state.

Desktop notifications (via `notify-send`) fire on connect/disconnect/pair
success and on errors. Disable them with:

```sh
bluwofi --no-notify
```

Bind that variant instead in your sway config if you'd rather not get
toasts, e.g.:

```
bindsym $mod+b exec /path/to/bluwofi/target/release/bluwofi --no-notify
```

## Waybar integration

Run with `--status` for a one-shot JSON status line (no wofi menu opens):

```sh
bluwofi --status
```

Output looks like:

```json
{"text":"\uf293 AirPods Pro","tooltip":"AirPods Pro (82%)","class":"connected"}
```

The `\uf293`/`\uf294` icons are Font Awesome glyphs (bluetooth on/off) — you need a
Nerd Font or Font Awesome installed for them to render; otherwise waybar will
show a blank box where the icon should be. Swap them for plain text/emoji in
`print_status()` if you don't want to bother with icon fonts.

Add a module to `~/.config/waybar/config`:

```jsonc
"custom/bluetooth": {
    "exec": "/path/to/bluwofi --status",
    "return-type": "json",
    "interval": 5,
    "on-click": "/path/to/bluwofi",
    "format": "{}"
}
```

Then add `"custom/bluetooth"` to one of the `modules-left`/`modules-center`/
`modules-right` arrays in the same config. Clicking the module launches the
normal interactive wofi menu; the bar itself just polls `--status` every 5
seconds.

## Auto-reconnect daemon

`cargo build --release` also produces a second binary,
`target/release/bluetooth-reconnectd` — a small background service that
polls every 20 seconds for devices that are paired + trusted but not
currently connected, and retries connecting to them (8s timeout per
device). Useful for things like earbuds that you want to just reconnect
automatically when they come back in range, without opening the menu.

Run it manually to try it out:

```sh
./target/release/bluetooth-reconnectd
```

Leave it running in a terminal and watch the stderr log as you power your
headphones on/off.

### Manual disconnects are respected

If you disconnect a device through the interactive `bluwofi` menu,
it writes a marker telling the daemon to leave that device alone for 2
minutes (`RECONNECT_COOLDOWN_SECS` in `main.rs`), so it won't immediately
reconnect something you just disconnected on purpose. Disconnects that
happen for other reasons (device powered off, out of range) aren't
affected — the daemon will just naturally fail to reconnect until the
device is actually back.

### Running it as a systemd user service

A unit file is provided at `systemd/bluetooth-reconnectd.service`. Edit
the `ExecStart` path to match where you actually built the binary, then:

```sh
mkdir -p ~/.config/systemd/user
cp systemd/bluetooth-reconnectd.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now bluetooth-reconnectd
```

Check it's running and see its logs:

```sh
systemctl --user status bluetooth-reconnectd
journalctl --user -u bluetooth-reconnectd -f
```

## Extending it

Some natural next steps if you want to go further:

- **Per-device-type icons**: BlueZ reports a device class (headset,
  mouse, phone...) that could map to distinct icons instead of the
  generic connected/paired/available dots.
