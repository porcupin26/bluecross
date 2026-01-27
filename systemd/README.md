# BlueCross systemd Service Files

These are systemd user service files for running BlueCross as a background service.

## Installation

1. Copy the appropriate service file to your user systemd directory:

```bash
# For server machine:
mkdir -p ~/.config/systemd/user
cp bluecross-server.service ~/.config/systemd/user/

# For client machine:
mkdir -p ~/.config/systemd/user
cp bluecross-client.service ~/.config/systemd/user/
```

2. Make sure your config file is at `~/.config/bluecross/bluecross.json`, or edit the service file to point to your config location.

3. Reload systemd and enable the service:

```bash
systemctl --user daemon-reload

# For server:
systemctl --user enable bluecross-server
systemctl --user start bluecross-server

# For client:
systemctl --user enable bluecross-client
systemctl --user start bluecross-client
```

## Managing the Service

```bash
# Check status
systemctl --user status bluecross-server
systemctl --user status bluecross-client

# View logs
journalctl --user -u bluecross-server -f
journalctl --user -u bluecross-client -f

# Stop service
systemctl --user stop bluecross-server
systemctl --user stop bluecross-client

# Disable auto-start
systemctl --user disable bluecross-server
systemctl --user disable bluecross-client
```

## Notes

- These are user services, not system services. They run as your user.
- Services require a graphical session to be active.
- Make sure the bluecross commands are in your PATH (installed via pip).
- On Wayland, you may need additional environment variables for clipboard access.
