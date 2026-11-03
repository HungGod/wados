# Visual Studio Code from Microsoft's yum repository (its key imported into
# rpm's keyring, which the package keeps). The repository is gone again once
# it's installed: an image's own dnf runs never reach Microsoft, and the
# package stays as it was built. The RPM brings the libraries Electron needs.
rpm --import https://packages.microsoft.com/keys/microsoft.asc
cat > /etc/yum.repos.d/vscode.repo <<'REPO'
[code]
name=Visual Studio Code
baseurl=https://packages.microsoft.com/yumrepos/vscode
enabled=1
type=rpm-md
gpgcheck=1
gpgkey=https://packages.microsoft.com/keys/microsoft.asc
REPO
dnf_install code
rm -f /etc/yum.repos.d/vscode.repo
# The desktop's launcher runs the Electron binary through the shared wrapper
# (--no-sandbox, native Wayland under labwc) like the other Electron apps.
# --password-store=basic: there is no keyring daemon in the session.
# `code` in a terminal is still Microsoft's CLI at /usr/bin/code.
wadspaces-wrap-electron /usr/share/code/code /usr/local/bin/code-desktop --password-store=basic
cat > /usr/share/applications/wadspaces-vscode.desktop <<'DESK'
[Desktop Entry]
Type=Application
Name=VS Code
Comment=Code editor
Exec=/usr/local/bin/code-desktop %F
Icon=vscode
Terminal=false
Categories=Development;IDE;
StartupWMClass=Code
DESK
add_desktop_entry vscode wadspaces-vscode.desktop
# Its terminals in tmux too, so their jobs carry on while the wadspace moves
# between the screen and a stream (the base's vscode-terminal.py, at each
# desktop start: only what you haven't set yourself).
mkdir -p /etc/wadspaces/autostart.d
cat > /etc/wadspaces/autostart.d/40-vscode-terminal.sh <<'HOOK'
python3 /usr/local/lib/wadspaces/vscode-terminal.py > /dev/null 2>&1 || true
HOOK
