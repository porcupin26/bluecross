Name:           bluecross
Version:        0.1.0
Release:        1%{?dist}
Summary:        Share keyboard, mouse and clipboard across Linux computers

License:        MIT
URL:            https://github.com/kzapolski/bluecross
Source0:        %{name}-%{version}.tar.gz

BuildArch:      noarch
BuildRequires:  python3-devel
BuildRequires:  python3-pip
BuildRequires:  python3-hatchling
BuildRequires:  python3-build

Requires:       python3 >= 3.10
Requires:       python3-evdev >= 1.6.0

Recommends:     wl-clipboard
Suggests:       xclip
Suggests:       xsel

%description
BlueCross allows sharing a single keyboard and mouse across multiple Linux
computers over the network, similar to Barrier/Synergy but designed to work
on both X11 and Wayland.

Features include:
- Seamless mouse cursor switching at screen edges
- Clipboard sharing between machines
- Support for Wayland and X11
- Low-latency input forwarding

%prep
%autosetup -n %{name}-%{version}

%build
%pyproject_wheel

%install
%pyproject_install

# Install udev rules
install -D -m 644 packaging/udev/99-bluecross-uinput.rules \
    %{buildroot}%{_udevrulesdir}/99-bluecross-uinput.rules

# Install example config
install -D -m 644 bluecross.json \
    %{buildroot}%{_datadir}/%{name}/bluecross.json.example

# Install systemd user service files
install -D -m 644 systemd/bluecross-server.service \
    %{buildroot}%{_datadir}/%{name}/systemd/bluecross-server.service
install -D -m 644 systemd/bluecross-client.service \
    %{buildroot}%{_datadir}/%{name}/systemd/bluecross-client.service

# Install documentation
install -D -m 644 README.md %{buildroot}%{_docdir}/%{name}/README.md

%post
# Reload udev rules
udevadm control --reload-rules || :
udevadm trigger --subsystem-match=misc --attr-match=name=uinput || :

echo ""
echo "BlueCross has been installed successfully!"
echo ""
echo "Post-installation steps:"
echo "  1. Add your user to the 'input' group:"
echo "     sudo usermod -aG input \$USER"
echo ""
echo "  2. Log out and back in (or reboot) for group changes to take effect"
echo ""
echo "  3. Copy the example config to your user config directory:"
echo "     mkdir -p ~/.config/bluecross"
echo "     cp %{_datadir}/%{name}/bluecross.json.example ~/.config/bluecross/bluecross.json"
echo ""
echo "  4. Edit the configuration file for your setup"
echo ""
echo "  5. (Optional) Install systemd user services:"
echo "     cp %{_datadir}/%{name}/systemd/*.service ~/.config/systemd/user/"
echo "     systemctl --user daemon-reload"
echo ""

%postun
# Reload udev rules after uninstall
udevadm control --reload-rules || :

%files
%license packaging/debian/copyright
%doc README.md
%{python3_sitelib}/%{name}/
%{python3_sitelib}/%{name}-%{version}*
%{_bindir}/bluecross-server
%{_bindir}/bluecross-client
%{_bindir}/bluecrossctl
%{_udevrulesdir}/99-bluecross-uinput.rules
%{_datadir}/%{name}/

%changelog
* Sun Jan 26 2025 Porcupin - 0.1.0-1
- Initial release
