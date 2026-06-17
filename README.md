# BlueCross

Share one keyboard, mouse and clipboard across multiple Linux computers over the
network — like Barrier/Synergy. **Wayland only.**

The network link is **encrypted and mutually authenticated** with a pre-shared
key (Noise protocol), so keystrokes and clipboard contents are protected in
transit and a peer cannot connect or inject input without the shared secret.

## Features

- Single keyboard/mouse shared across machines, with edge-of-screen switching
- Encrypted, PSK-authenticated transport (no plaintext keystrokes on the wire)
- Clipboard sharing via `wl-clipboard`
- Low-latency input forwarding over the kernel evdev/uinput interfaces
- Single static binary, systemd user services, `.deb` / `.rpm` packages

## Platform support

BlueCross is **Linux + Wayland only**. It relies on Linux kernel interfaces
(`evdev` for capture, `EVIOCGRAB` for grabbing, `uinput` for injection) and on
`wl-clipboard` for the clipboard. **X11 is not supported**, and **Windows/macOS
are not supported** — those would require entirely different input backends.

## Requirements

- Linux with a Wayland session and `evdev`/`uinput` kernel support
- `wl-clipboard` for clipboard sharing:
  ```bash
  sudo apt install wl-clipboard      # Debian/Ubuntu
  sudo dnf install wl-clipboard      # Fedora
  ```
- Your user in the `input` group (see [Permissions](#permissions))

## Installation

### Debian / Ubuntu
```bash
sudo apt install ./bluecross_*.deb
```

### Fedora
```bash
sudo dnf install ./bluecross-*.rpm
```

### Portable static binary (any x86_64 Linux)
```bash
curl -L -o bluecross https://github.com/porcupin26/bluecross/releases/latest/download/bluecross-linux-x86_64
chmod +x bluecross
sudo mv bluecross /usr/local/bin/
```

### Build from source
```bash
# Requires the Rust toolchain (https://rustup.rs)
cargo build --release
sudo cp target/release/bluecross /usr/local/bin/
```

## Configuration

Create `~/.config/bluecross/bluecross.json`. The **same machine's file** can hold
both a `server` and a `client` section; `bluecross server` reads the former and
`bluecross client` the latter.

```json
{
  "server": {
    "host": "0.0.0.0",
    "port": 12345,
    "edge_threshold": 5,
    "clipboard_sharing": true,
    "psk": "REPLACE-with-a-long-random-shared-secret",
    "clients": {
      "laptop": "left",
      "desktop2": "right"
    }
  },
  "client": {
    "server_host": "192.168.1.100",
    "server_port": 12345,
    "name": "laptop",
    "clipboard_sharing": true,
    "psk": "REPLACE-with-a-long-random-shared-secret"
  }
}
```

### The pre-shared key (`psk`) — required

BlueCross **will not start without a `psk`** of at least 16 characters, and the
server and every client must use the **same** value. Generate one with:

```bash
head -c 32 /dev/urandom | base64
```

A wrong or missing key means the encrypted handshake fails and no input or
clipboard data is exchanged.

### Options

**Server**
- `host` — address to bind. Defaults to `127.0.0.1` (loopback). Set to your LAN
  IP or `0.0.0.0` to accept remote clients. Binding to all interfaces is logged
  as a warning; only the `psk` protects the listener, so use a trusted network.
- `port` — TCP port (default `12345`)
- `screen_width`, `screen_height` — auto-detected on Wayland if omitted
- `edge_threshold` — pixels from the edge that trigger a switch (default `5`)
- `clipboard_sharing` — enable/disable clipboard sync (default `true`)
- `psk` — shared secret (**required**)
- `clients` — map of client `name` → position (`left`/`right`/`top`/`bottom`)

**Client**
- `server_host`, `server_port` — where the server listens
- `name` — must match a key in the server's `clients` map
- `screen_width`, `screen_height` — auto-detected on Wayland if omitted
- `clipboard_sharing` — default `true`
- `psk` — shared secret (**required**, must match the server)

Screen size auto-detection tries, in order: GNOME/Mutter (D-Bus), wlroots
(`wlr-randr`), and KDE Plasma (`kscreen-doctor`). Set the dimensions explicitly
if detection fails.

## Permissions

Input capture (server) and injection (client) need device access. Add your user
to the `input` group, then log out and back in:

```bash
sudo usermod -aG input $USER
```

The packages install a udev rule granting the `input` group access to
`/dev/uinput`. If you installed the raw binary, add it yourself:

```bash
echo 'KERNEL=="uinput", MODE="0660", GROUP="input", OPTIONS+="static_node=uinput"' \
  | sudo tee /etc/udev/rules.d/99-bluecross-uinput.rules
sudo udevadm control --reload-rules && sudo udevadm trigger
```

## Usage

Run in the foreground while testing:

```bash
bluecross server -f      # on the machine with the keyboard/mouse
bluecross client -f      # on each secondary machine
```

Move the mouse to a configured edge to switch control to a client. To return,
push the mouse against the opposite edge on the client and hold for ~300 ms.

### As a systemd user service (recommended)

The packages install user units to `/usr/lib/systemd/user/`:

```bash
systemctl --user daemon-reload
systemctl --user enable --now bluecross-server   # or bluecross-client
journalctl --user -u bluecross-server -f
```

Units use `Restart=always`, and **upgrading the `.deb`/`.rpm` automatically
restarts any running BlueCross service** with the new binary. To start at boot
without an interactive login: `sudo loginctl enable-linger $USER`.

### Control utility (daemon mode without systemd)

```bash
bluecross ctl start server      # start as a background daemon
bluecross ctl status
bluecross ctl logs server -f
bluecross ctl stop server
```

Backward-compatible symlinks `bluecross-server`, `bluecross-client` and
`bluecrossctl` are also installed.

## Security notes

- Always set a strong random `psk`. It is the only thing authenticating peers and
  keying the encryption.
- Prefer binding the server to a specific LAN interface rather than `0.0.0.0`.
- A client trusts input from whatever server it connects to; only connect to
  servers you control. The PSK prevents impostor servers.
- For untrusted networks, additionally tunnel over WireGuard/SSH.

## Troubleshooting

**Won't start — "no 'psk' configured":** set the same `psk` (≥ 16 chars) in both
the server and client config.

**"Permission denied":** ensure your user is in the `input` group and you've
logged out/in; for the client, confirm the `/dev/uinput` udev rule is active.

**Clipboard not syncing:** install `wl-clipboard` and keep `clipboard_sharing`
set to `true`. BlueCross is Wayland-only — there is no X11 clipboard fallback.

**Laggy movement:** check network latency between the machines.
