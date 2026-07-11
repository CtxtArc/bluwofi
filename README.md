# bluwofi

<p align="center">
  <img src="assets/demo.gif" alt="bluwofi demo" width="800">
</p>

A small Bluetooth manager for Sway and wlroots-based desktops, using a
`wofi` popup interface.

`bluwofi` communicates directly with BlueZ over D-Bus through the
[`bluer`](https://crates.io/crates/bluer) Rust crate. It does not require a
GTK Bluetooth applet or a full desktop environment.

## Features

* `wofi` based Bluetooth management menu
* Direct BlueZ D-Bus communication
* Connect, disconnect, pair, and remove devices
* Device discovery from the menu
* Optional desktop notifications
* Waybar status integration
* Background auto-reconnect daemon
* Small TOML configuration file

## Requirements

* `bluetoothd` running:

```sh
systemctl status bluetooth
```

* `wofi` installed and available in `$PATH`
* `notify-send` (optional, provided by `libnotify` or a notification daemon
  such as `mako`)
* Rust toolchain (`rustup`)
* `make`

## Installation

### From source

Clone the repository:

```sh
git clone https://github.com/CtxtArc/bluwofi.git
cd bluwofi
```

Build and install:

```sh
make
sudo make install
```

By default, files are installed to:

```text
/usr/local/bin
/usr/local/lib/systemd/user
```

You can change the installation prefix:

```sh
sudo make install PREFIX=/usr
```

For package maintainers, `DESTDIR` is supported:

```sh
make install DESTDIR=/tmp/package
```

The installed binaries are:

```text
bluwofi
bluetooth-reconnectd
```

## Manual build

If you only want to build without installing:

```sh
cargo build --release
```

The binaries will be located at:

```text
target/release/bluwofi
target/release/bluetooth-reconnectd
```

## Uninstall

Remove installed files:

```sh
sudo make uninstall
```

## Configuration

Optional configuration file:

```text
~/.config/bluwofi/config.toml
```

All fields are optional. Missing values use the defaults below:

```toml
# How long a manual "Scan for devices" stays open, in seconds.
scan_duration_secs = 5

# After manually disconnecting a device, how long the reconnect daemon
# waits before trying again.
reconnect_cooldown_secs = 120

# Interval between reconnect daemon scans.
daemon_poll_interval_secs = 20

# Timeout for individual connection attempts.
daemon_connect_timeout_secs = 8
```

Both `bluwofi` and `bluetooth-reconnectd` read the same configuration file.

## Usage

Run the menu:

```sh
bluwofi
```

The main menu provides:

* Bluetooth adapter power toggle
* Device scanning
* Known devices sorted with connected devices first

Selecting a device opens available actions:

* Connect
* Disconnect
* Pair & Connect
* Remove

Notifications are shown for connection, disconnection, pairing, and errors.

Disable notifications with:

```sh
bluwofi --no-notify
```

## Sway integration

Add a key binding to:

```text
~/.config/sway/config
```

Example:

```ini
bindsym $mod+b exec bluwofi
```

Without notifications:

```ini
bindsym $mod+b exec bluwofi --no-notify
```

## Waybar integration

`bluwofi` can output a JSON status line:

```sh
bluwofi --status
```

Example:

```json
{
  "text": "\uf293 AirPods Pro",
  "tooltip": "AirPods Pro (82%)",
  "class": "connected"
}
```

The Bluetooth icons use Font Awesome glyphs. A Nerd Font or Font Awesome
font is required for them to display correctly.

Add a module to your Waybar configuration:

```jsonc
"custom/bluetooth": {
    "exec": "bluwofi --status",
    "return-type": "json",
    "interval": 5,
    "on-click": "bluwofi",
    "format": "{}"
}
```

Then add:

```json
"custom/bluetooth"
```

to one of:

* `modules-left`
* `modules-center`
* `modules-right`

## Auto-reconnect daemon

The project includes a second binary:

```text
bluetooth-reconnectd
```

The daemon periodically checks for devices that are:

* paired
* trusted
* currently disconnected

and attempts to reconnect them.

Run it manually:

```sh
bluetooth-reconnectd
```

### Manual disconnect handling

Disconnecting a device from the `bluwofi` menu creates a cooldown marker.

During this cooldown period, the daemon will not attempt to reconnect that
device.

This prevents reconnecting something that was intentionally disconnected.

Disconnects caused by devices leaving range or powering off are not treated as
manual disconnects.

## Running as a systemd user service

A service file is included:

```text
systemd/bluetooth-reconnectd.service
```

Enable it:

```sh
systemctl --user daemon-reload
systemctl --user enable --now bluetooth-reconnectd
```

Check its status:

```sh
systemctl --user status bluetooth-reconnectd
```

Follow logs:

```sh
journalctl --user -u bluetooth-reconnectd -f
```

## Possible improvements

Some possible future additions:

* Per-device-type icons (headphones, mouse, phone, etc.)
* Better battery reporting
* Audio profile switching
* More device filtering options
* Additional configuration options
