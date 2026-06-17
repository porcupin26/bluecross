Name:           bluecross
Version:        0.1.0
Release:        1%{?dist}
Summary:        Share keyboard, mouse and clipboard across Wayland computers

License:        MIT
URL:            https://github.com/porcupin26/bluecross
Source0:        %{name}-%{version}.tar.gz

BuildRequires:  gcc

Recommends:     wl-clipboard

%description
BlueCross shares a single keyboard and mouse across multiple Linux computers
over the network, similar to Barrier/Synergy. It is Wayland-only and uses the
kernel evdev/uinput interfaces for input capture and injection.

The network link is encrypted and mutually authenticated with a pre-shared key
(Noise protocol), so keystrokes and clipboard contents are protected in transit.

Features include:
- Seamless mouse cursor switching at screen edges
- Encrypted, PSK-authenticated transport
- Clipboard sharing between machines (wl-clipboard)
- Low-latency input forwarding

%prep
%autosetup -n %{name}-%{version}

%build
source "$HOME/.cargo/env" 2>/dev/null || true
cargo build --release

%install
# Install binary
install -D -m 755 target/release/bluecross \
    %{buildroot}%{_bindir}/bluecross

# Create symlinks for backward compatibility
ln -sf bluecross %{buildroot}%{_bindir}/bluecross-server
ln -sf bluecross %{buildroot}%{_bindir}/bluecross-client
ln -sf bluecross %{buildroot}%{_bindir}/bluecrossctl

# Install udev rules
install -D -m 644 packaging/udev/99-bluecross-uinput.rules \
    %{buildroot}/usr/lib/udev/rules.d/99-bluecross-uinput.rules

# Install example config
install -D -m 644 bluecross.json \
    %{buildroot}%{_datadir}/%{name}/bluecross.json.example

# Install systemd user units to the system-wide user-unit dir
install -D -m 644 systemd/bluecross-server.service \
    %{buildroot}%{_userunitdir}/bluecross-server.service
install -D -m 644 systemd/bluecross-client.service \
    %{buildroot}%{_userunitdir}/bluecross-client.service

# Install documentation
install -D -m 644 README.md %{buildroot}%{_docdir}/%{name}/README.md

%post
# Reload udev rules
udevadm control --reload-rules || :
udevadm trigger --subsystem-match=misc --attr-match=name=uinput || :

# On upgrade ($1 == 2), restart any running user services with the new binary.
if [ "$1" -ge 2 ]; then
    if [ -d /run/user ]; then
        for rundir in /run/user/*; do
            [ -d "$rundir" ] || continue
            uid=$(basename "$rundir")
            user=$(id -nu "$uid" 2>/dev/null) || continue
            runuser -u "$user" -- env XDG_RUNTIME_DIR="$rundir" \
                systemctl --user daemon-reload >/dev/null 2>&1 || :
            for svc in bluecross-server.service bluecross-client.service; do
                runuser -u "$user" -- env XDG_RUNTIME_DIR="$rundir" \
                    systemctl --user try-restart "$svc" >/dev/null 2>&1 || :
            done
        done
    fi
fi

if [ "$1" -eq 1 ]; then
    echo ""
    echo "BlueCross has been installed successfully!"
    echo ""
    echo "Post-installation steps:"
    echo "  1. Add your user to the 'input' group:"
    echo "     sudo usermod -aG input \$USER"
    echo ""
    echo "  2. Log out and back in (or reboot) for group changes to take effect"
    echo ""
    echo "  3. Copy the example config and set a shared 'psk' (>= 16 chars):"
    echo "     mkdir -p ~/.config/bluecross"
    echo "     cp %{_datadir}/%{name}/bluecross.json.example ~/.config/bluecross/bluecross.json"
    echo "     # generate a key:  head -c 32 /dev/urandom | base64"
    echo ""
    echo "  4. Edit the configuration file for your setup"
    echo ""
    echo "  5. (Optional) Enable the systemd user service:"
    echo "     systemctl --user daemon-reload"
    echo "     systemctl --user enable --now bluecross-server   # or bluecross-client"
    echo ""
fi

%postun
# Reload udev rules after uninstall
udevadm control --reload-rules || :

%files
%doc README.md
%{_bindir}/bluecross
%{_bindir}/bluecross-server
%{_bindir}/bluecross-client
%{_bindir}/bluecrossctl
/usr/lib/udev/rules.d/99-bluecross-uinput.rules
%{_userunitdir}/bluecross-server.service
%{_userunitdir}/bluecross-client.service
%{_datadir}/%{name}/

%changelog
* Tue Jun 17 2026 Porcupin <porcupin26@proton.me> - 0.1.0-1
- Wayland-only release with encrypted, PSK-authenticated transport
