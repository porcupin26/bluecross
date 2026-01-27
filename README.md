# BlueCross

Share keyboard, mouse and clipboard across multiple Linux computers over the network. Similar to Barrier/Synergy but designed to work on both X11 and Wayland.

## Features

- Share a single keyboard and mouse across multiple computers
- Seamless mouse cursor switching at screen edges
- Clipboard sharing between machines
- Support for Wayland (via wl-clipboard) and X11 (via xclip/xsel)
- Low-latency input forwarding

## Requirements

- Python 3.10+
- Linux with evdev support
- For clipboard sharing: `wl-clipboard` (Wayland) or `xclip`/`xsel` (X11)

Install clipboard tools:
```bash
# Wayland
sudo apt install wl-clipboard

# X11
sudo apt install xclip
```

## Configuration

Create a `bluecross.json` file:

```json
{
  "server": {
    "host": "0.0.0.0",
    "port": 12345,
    "edge_threshold": 5,
    "clipboard_sharing": true,
    "clients": {
      "laptop": "left",
      "desktop2": "right"
    }
  },
  "client": {
    "server_host": "192.168.1.100",
    "server_port": 12345,
    "name": "laptop",
    "clipboard_sharing": true
  }
}
```

### Configuration Options

**Server:**
- `host`: IP address to listen on (use `0.0.0.0` for all interfaces)
- `port`: TCP port to listen on
- `screen_width`, `screen_height`: Screen resolution (auto-detected if not specified, supports fractional scaling)
- `edge_threshold`: Pixels from screen edge to trigger switch (default: 5)
- `clipboard_sharing`: Enable/disable clipboard sync (default: true)
- `clients`: Map of client names to their position relative to server (`left`, `right`, `top`, `bottom`)

**Client:**
- `server_host`: IP address of the server
- `server_port`: TCP port of the server
- `screen_width`, `screen_height`: Screen resolution (auto-detected if not specified, supports fractional scaling)
- `name`: Client name (must match a key in server's `clients` config)
- `clipboard_sharing`: Enable/disable clipboard sync (default: true)

Screen size is automatically detected on both server and client using:
- GNOME/Mutter D-Bus interface (Wayland with fractional scaling)
- KDE kscreen-doctor (Plasma Wayland)
- wlr-randr (wlroots-based compositors like Sway)
- xrandr (X11)

## Running the Server

The server requires access to input devices. Add your user to the `input` group:

```bash
sudo usermod -aG input $USER
```

You'll need to log out and back in (or reboot) for the group change to take effect.

Start the server:
```bash
uv run python -m bluecross.server
```

## Running the Client

The client requires access to `/dev/uinput` to create virtual input devices.

**Option 1: Run with sudo**
```bash
sudo uv run python -m bluecross.client
```

**Option 2: Add a udev rule (permanent fix)**

Create a udev rule to allow your user to access `/dev/uinput`:

```bash
sudo tee /etc/udev/rules.d/99-uinput.rules << 'EOF'
KERNEL=="uinput", MODE="0660", GROUP="input", OPTIONS+="static_node=uinput"
EOF
```

Then add your user to the `input` group and reload:

```bash
sudo usermod -aG input $USER
sudo udevadm control --reload-rules
sudo udevadm trigger
```

You'll need to log out and back in (or reboot) for the group change to take effect.

Then run without sudo:
```bash
uv run python -m bluecross.client
```

## Usage

1. Start the server on the main machine (the one with the keyboard/mouse)
2. Start the client on the secondary machine(s)
3. Move your mouse to the configured edge to switch control to the client
4. Move back to the opposite edge on the client to return control to the server

## Troubleshooting

**"Permission denied" errors:**
- Ensure your user is in the `input` group
- Log out and back in after adding to the group
- For client, ensure udev rules are set up or run with sudo

**Clipboard not syncing:**
- Ensure `wl-clipboard` (Wayland) or `xclip` (X11) is installed
- Check that `clipboard_sharing` is `true` in config

**Laggy mouse movement:**
- Check network latency between machines
- Ensure both machines have adequate CPU resources
