# Steam: RPM Fusion's steam (its 32-bit libraries come with it, from Fedora's
# i686 packages) and Fedora's gamescope. `wadspaces-steam` starts it, with its
# CEF sandbox off where a wadspace has no user namespaces.
# Proton isn't a package: Steam downloads it, once signed in, for the first
# game that needs it. `wadspaces-steam` makes it the default for every game
# on a new account's first start, as Settings ▸ Compatibility would.
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
config="${HOME}/.local/share/Steam/config/config.vdf"
if [ ! -e "${config}" ]; then
    mkdir -p "${config%/*}"
    cat > "${config}" <<'VDF'
"InstallConfigStore"
{
	"Software"
	{
		"Valve"
		{
			"Steam"
			{
				"CompatToolMapping"
				{
					"0"
					{
						"name"		"proton_experimental"
						"config"		""
						"priority"		"75"
					}
				}
			}
		}
	}
}
VDF
fi
exec /usr/bin/steam "$@"
WRAP
chmod 755 /usr/local/bin/wadspaces-steam
sed -i 's|^Exec=/usr/bin/steam|Exec=/usr/local/bin/wadspaces-steam|; s|^Exec=steam|Exec=/usr/local/bin/wadspaces-steam|' \
    /usr/share/applications/steam.desktop
rpm -q steam gamescope
