# Steam: RPM Fusion's steam (its 32-bit libraries come with it, from Fedora's
# i686 packages) and Fedora's gamescope. `wadspaces-steam` starts it, with its
# CEF sandbox off where a wadspace has no user namespaces.
# RPM Fusion's release package is the only thing taken from outside Fedora's
# repositories, and it's signed: dnf checks it against Fedora's own key file.
FEDORA="$(rpm -E %fedora)"
dnf_install "https://mirrors.rpmfusion.org/nonfree/fedora/rpmfusion-nonfree-release-${FEDORA}.noarch.rpm"
dnf_install steam gamescope
cat > /usr/local/bin/wadspaces-steam <<'WRAP'
#!/bin/sh
if ! bwrap --ro-bind / / --dev /dev --proc /proc --unshare-pid /bin/true 2> /dev/null; then
    set -- -no-cef-sandbox "$@"
fi
exec /usr/bin/steam "$@"
WRAP
chmod 755 /usr/local/bin/wadspaces-steam
sed -i 's|^Exec=/usr/bin/steam|Exec=/usr/local/bin/wadspaces-steam|; s|^Exec=steam|Exec=/usr/local/bin/wadspaces-steam|' \
    /usr/share/applications/steam.desktop
rpm -q steam gamescope
