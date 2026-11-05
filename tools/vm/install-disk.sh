#!/usr/bin/env bash
# In the builder VM, as root (tools/vm.sh disk sends it over SSH): the host
# image written onto the second virtual disk, with the wadspace base in its
# root podman storage.
#
#   install-disk.sh REGISTRY IMAGE TARGET_IMGREF BASE KEY [EXTRA…]
#
# REGISTRY: the laptop's throwaway registry as the VM sees it (10.0.2.2:PORT);
# IMAGE, BASE: names in it; TARGET_IMGREF: what the installed system says it
# follows (bootc status); KEY: the public key root's SSH takes, on this disk
# only; EXTRA: more images for root's podman storage, as BASE (the stream
# sidecar, wados-stream:44).
set -euo pipefail
REG="$1" IMAGE="$2" TARGET="$3" BASE="$4" KEY="$5"
shift 5
EXTRA=("$@")
DISK=/dev/disk/by-id/virtio-wados-target
MNT=/mnt/wados
T0=$(date +%s)
step() { printf '\n>> [%3d s] %s\n' "$(($(date +%s) - T0))" "$*"; }

[[ -b "${DISK}" ]] || { echo "no target disk (${DISK})" >&2; exit 1; }
if ! command -v podman > /dev/null; then
    step "podman, into the builder (once: its disk is kept)"
    dnf -y -q install --setopt=install_weak_deps=False podman
fi
podman --version

step "pulling ${IMAGE} from the laptop"
podman pull -q --tls-verify=false "${REG}/${IMAGE}"
SRC="${REG}/${IMAGE}"

# bootc's own options, as this image's bootc has them.
HELP="$(podman run --rm "${SRC}" bootc install to-disk --help)"
has() { grep -q -- "$1" <<< "${HELP}"; }
args=(--wipe)
has --generic-image && args+=(--generic-image)
has --skip-fetch-check && args+=(--skip-fetch-check)
has --root-ssh-authorized-keys && args+=(--root-ssh-authorized-keys /run/wados-vm.pub)
has --target-imgref && args+=(--target-imgref "${TARGET}")
# The kernel's messages and systemd's on the serial port too (.build/vm/serial.log).
args+=(--karg console=tty0 --karg console=ttyS0,115200n8)
# A filesystem, unless the image says which (/usr/lib/bootc/install/*.toml).
if ! podman run --rm "${SRC}" sh -c 'cat /usr/lib/bootc/install/*.toml 2> /dev/null' | grep -qE '^\s*type\s*='; then
    args+=(--filesystem xfs)
fi

step "bootc install to-disk ${args[*]}"
podman run --rm --privileged --pid=host \
    -v /dev:/dev -v /var/lib/containers:/var/lib/containers \
    -v "${KEY}:/run/wados-vm.pub:ro" \
    --security-opt label=type:unconfined_t \
    "${SRC}" bootc install to-disk "${args[@]}" "$(readlink -f "${DISK}")"

step "the wadspace base (${BASE})${EXTRA[*]:+ and ${EXTRA[*]}}, into the new disk's root podman storage"
udevadm settle
ROOTFS="$(lsblk -nrpo NAME,PARTLABEL,LABEL "$(readlink -f "${DISK}")" | awk '$2 == "root" || $3 == "root" { print $1; exit }')"
[[ -n "${ROOTFS}" ]] || { lsblk -o NAME,PARTLABEL,LABEL,FSTYPE "${DISK}"; echo "no root partition" >&2; exit 1; }
mkdir -p "${MNT}"
mount "${ROOTFS}" "${MNT}"
trap 'umount -R "${MNT}" 2> /dev/null || true' EXIT
VAR="$(find "${MNT}/ostree/deploy" -mindepth 2 -maxdepth 2 -name var -type d | head -1)"
[[ -n "${VAR}" ]] || { echo "no ostree deployment's /var on ${ROOTFS}" >&2; exit 1; }
echo "   its /var: ${VAR#"${MNT}"}"
mkdir -p "${VAR}/lib/containers"
# Its storage at the path the machine's podman uses, and runtime folders of
# its own: podman's database keeps those paths and checks them when it starts.
unshare --mount --propagation private bash -euo pipefail -c '
    mkdir -p /var/lib/containers /run/containers /run/libpod
    mount --bind "$1/lib/containers" /var/lib/containers
    mount -t tmpfs tmpfs /run/containers
    mount -t tmpfs tmpfs /run/libpod
    reg="$2"
    shift 2
    for img in "$@"; do
        podman pull -q --tls-verify=false "${reg}/${img}"
        podman tag "${reg}/${img}" "localhost/${img}"
        podman untag "${reg}/${img}" "${reg}/${img}"
    done
    podman images --format "   {{.Repository}}:{{.Tag}} {{.ID}} {{.Size}}"
    # Labelled as the machine labels its own (container_var_lib_t, container_ro_file_t…).
    restorecon -R /var/lib/containers
' _ "${VAR}" "${REG}" "${BASE}" "${EXTRA[@]}"

step "root's SSH, on this disk only"
# The image's sshd says PermitRootLogin no (etc/ssh/sshd_config.d/40-wados.conf).
# sshd takes the first value it reads, and reads its drop-ins in order: this
# one, in the installed /etc (never the image), comes first. Keys only.
ETC="$(find "${MNT}/ostree/deploy" -mindepth 4 -maxdepth 4 -path '*/deploy/*/etc' -type d | head -1)"
[[ -n "${ETC}" ]] || { echo "no deployment's /etc" >&2; exit 1; }
mkdir -p "${ETC}/ssh/sshd_config.d"
cat > "${ETC}/ssh/sshd_config.d/30-wados-vm.conf" << 'EOF'
# tools/vm.sh's test disk (not the image): root may sign in with the key
# bootc installed (--root-ssh-authorized-keys), for the VM's checks.
PermitRootLogin prohibit-password
EOF
chcon --reference="${ETC}/ssh/sshd_config.d" "${ETC}/ssh/sshd_config.d/30-wados-vm.conf" 2> /dev/null || true
chcon -t etc_t "${ETC}/ssh/sshd_config.d/30-wados-vm.conf" 2> /dev/null || true
ls -Z "${ETC}/ssh/sshd_config.d/" | sed 's/^/   /'

umount -R "${MNT}"
trap - EXIT
sync
# The builder's disk is kept (it has podman): not the images it pulled.
podman rmi -f "${SRC}" > /dev/null 2>&1 || true
podman image prune -f > /dev/null 2>&1 || true
# …and their blocks given back to its qcow2 (attached with discard=unmap).
fstrim -a > /dev/null 2>&1 || true
step "done"
