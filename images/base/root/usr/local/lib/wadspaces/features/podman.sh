# Podman, buildah and skopeo: what builds and pushes images from inside a
# wadspace (the WadOS one: host/build.sh image, then push to ghcr, or wadd
# takes the build from here for "This machine's builds"). Rootless, as abc.
#
# Storage: overlay, rootless (the kernel's own in a user namespace), in the
# wadspace's /config volume, which is a plain filesystem, not an overlay, so
# it isn't an overlay on an overlay. Not vfs: a host image is 65 layers, and
# vfs keeps a whole copy of the filesystem for each one (tens of GB for
# fedora-bootc alone). fuse-overlayfs is here for a kernel that won't (the
# unit gives the wadspace /dev/fuse): set it as mount_program then.
#
# `wadspaces-ghcr-login` is the one thing that needs the machine's GitHub
# token: it signs podman in to ghcr.io with it (a token with write:packages
# pushes; a read-only one only pulls). The token is the machine's podman
# secret, read at /run/secrets/github_token (as `git-credential-github-token`
# reads it), never in the image or a file of a wadspace's.
dnf_install podman buildah skopeo fuse-overlayfs slirp4netns
mkdir -p /etc/containers
cat > /etc/containers/storage.conf <<'CONF'
# A wadspace's own images: rootless overlay, under its home so they live in
# its /config volume and outlive a restart. (rootless_storage_path: rootless
# podman doesn't read graphroot. Not storage/, where vfs kept them before: a
# store doesn't change driver; that one can go, `podman unshare rm -rf
# ~/.local/share/containers/storage`.)
[storage]
driver = "overlay"
runroot = "/run/containers/storage"
graphroot = "/var/lib/containers/storage"
rootless_storage_path = "/config/.local/share/containers/storage-overlay"
CONF
# The ids what it builds may own (a host image has files of many users):
# every id of the wadspace's own 65536 but abc's, as podman's own image
# does for its user. Without them, rootless podman has abc's one id.
for f in /etc/subuid /etc/subgid; do
    grep -q '^abc:' "${f}" 2> /dev/null || printf 'abc:1:999\nabc:1001:64535\n' >> "${f}"
done
# What maps them: newuidmap and newgidmap with their file capabilities, as
# Fedora's rpm and podman's own image have them (setuid isn't enough in a
# wadspace's user namespace: the kernel refuses its write). The base sets
# them too; dnf may have reinstalled shadow-utils since.
setcap cap_setuid=ep /usr/bin/newuidmap
setcap cap_setgid=ep /usr/bin/newgidmap
# Containers in a container (podman's own image's settings): the
# wadspace's network (no /dev/net/tun for a network of their own), its
# namespaces and no cgroups of their own (no systemd in a wadspace).
cat > /etc/containers/containers.conf <<'CONF'
[containers]
netns="host"
userns="host"
ipcns="host"
utsns="host"
cgroupns="host"
cgroups="disabled"
log_driver = "k8s-file"
[engine]
cgroup_manager = "cgroupfs"
events_logger="file"
runtime="crun"
CONF
cat > /usr/local/bin/wadspaces-ghcr-login <<'WRAP'
#!/bin/sh
# Signs this wadspace's podman in to ghcr.io with the machine's GitHub token
# (its podman secret at /run/secrets/github_token, or $GITHUB_TOKEN). Nothing
# goes where another wadspace sees it: the login is in the user's containers
# auth file (~/.config/containers/auth.json, 0600).
set -eu
token_file="${GITHUB_TOKEN_FILE:-/run/secrets/github_token}"
token="${GITHUB_TOKEN:-}"
if [ -z "${token}" ] && [ -r "${token_file}" ]; then
    token="$(head -n1 "${token_file}")"
fi
if [ -z "${token}" ]; then
    echo "wadspaces-ghcr-login: no GitHub token (${token_file}: the machine has no sign-in, or none handed over)" >&2
    exit 1
fi
user="${GITHUB_USER:-wadspaces}"
printf '%s' "${token}" | podman login --username "${user}" --password-stdin ghcr.io
WRAP
chmod 755 /usr/local/bin/wadspaces-ghcr-login
podman --version && buildah --version && skopeo --version
