# Podman, buildah and skopeo: what builds and pushes images from inside a
# wadspace (the WadOS one: host/build.sh image, then push to ghcr). Rootless,
# with vfs storage: a wadspace is a container itself, where an overlay on an
# overlay isn't reliable, and vfs needs no kernel features beyond user
# namespaces (the unit gives it those, and /dev/fuse for fuse-overlayfs).
#
# `wadspaces-ghcr-login` is the one thing that needs the machine's GitHub
# token: it signs podman in to ghcr.io with it (a token with write:packages
# pushes; a read-only one only pulls). The token is the machine's podman
# secret, read at /run/secrets/github_token (as `git-credential-github-token`
# reads it), never in the image or a file of a wadspace's.
dnf_install podman buildah skopeo fuse-overlayfs slirp4netns
mkdir -p /etc/containers
cat > /etc/containers/storage.conf <<'CONF'
# A wadspace's own images: rootless, vfs (no overlay on an overlay), under
# its home so they live in its /config volume and outlive a restart.
[storage]
driver = "vfs"
runroot = "/run/containers/storage"
graphroot = "/config/.local/share/containers/storage"
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
