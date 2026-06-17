# BlueCross systemd Service Files

systemd **user** service units for running BlueCross as a background service.

When installed from the `.deb`/`.rpm` packages these units are already placed in
`/usr/lib/systemd/user/`, so you can skip straight to enabling them. If you built
from source, copy them into place first:

```bash
mkdir -p ~/.config/systemd/user
cp bluecross-server.service ~/.config/systemd/user/   # server machine
cp bluecross-client.service ~/.config/systemd/user/   # client machine
```

## Enable

Make sure your config is at `~/.config/bluecross/bluecross.json` (including a
shared `psk`), then:

```bash
systemctl --user daemon-reload

# Server machine:
systemctl --user enable --now bluecross-server

# Client machine:
systemctl --user enable --now bluecross-client
```

## Managing the service

```bash
systemctl --user status bluecross-server
journalctl --user -u bluecross-server -f      # follow logs
systemctl --user restart bluecross-server
systemctl --user stop bluecross-server
systemctl --user disable bluecross-server
```

## Notes

- These are **user** services and run as your user (needed for clipboard + the
  Wayland session). Your user must be in the `input` group (see the README).
- Units use `Restart=always`. When you upgrade the `.deb`/`.rpm` package, any
  running BlueCross user service is restarted automatically by the package
  scripts, so the new binary takes effect without manual intervention.
- Wayland-only. The user manager normally exports `WAYLAND_DISPLAY` and
  `XDG_RUNTIME_DIR`; if clipboard sharing can't reach the compositor, run
  `systemctl --user import-environment WAYLAND_DISPLAY XDG_RUNTIME_DIR`.
- To start at boot without an interactive login, enable lingering:
  `sudo loginctl enable-linger $USER`.
