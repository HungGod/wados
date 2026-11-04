#!/usr/bin/env bash
# WadOS in a VM, without root (phase 4): the host image made into a bootable
# disk, booted with KVM, and checked end to end.
#
#   tools/vm.sh disk [--fresh] [IMAGE]
#                       .build/vm/wados.qcow2 from IMAGE (localhost/wados-host:latest)
#   tools/vm.sh boot [--fresh]
#                       boots it in the background (--fresh: as installed)
#   tools/vm.sh ssh [ARGS…]      SSH into it as root (VM_USER=sam: as sam)
#   tools/vm.sh shot FILE        a screenshot (PNG)
#   tools/vm.sh keys 'text'      typed on its keyboard (US layout)
#   tools/vm.sh key COMBO…       keys pressed together: ret, esc, ctrl-alt-f2
#   tools/vm.sh stop             powers it off
#   tools/vm.sh status
#   tools/vm.sh check            boot → lock → unlock → a wadspace with sound → the
#                                machine's session → Restart → Power off → suspend
#                                (tools/vm/check.sh)
#   tools/vm.sh tailnet-check [--plain]
#                                two VMs, A and B, on a test tailnet (tools/tailnet.sh:
#                                a local Headscale): they see each other, Taildrive,
#                                the device that isn't trusted; then wadd's peers
#                                (tools/vm/check-tailnet.sh; --plain: Tailscale only)
#
# VM_NAME=a (or --name a, before the command): a second VM of the same disk,
# with its own layer, firmware variables, ports, QMP and serial log, in
# .build/vm/a/. Without it, the VM is the one check uses, as before.
#
# **disk**, without root: bootc writes a disk only as root, so it runs in a
# throwaway builder VM (Fedora Cloud Base 44, downloaded once into
# ~/.cache/wados/vm and checked against Fedora's signed CHECKSUM file).
# cloud-init (a NoCloud seed ISO) gives it this folder's SSH key. The images
# reach it through a registry on the laptop (podman's registry:2 on
# 127.0.0.1, 10.0.2.2 from the VM), and it installs onto its second disk,
# wados.qcow2 (40 GiB, sparse), with tools/vm/install-disk.sh: bootc install
# to-disk, root's SSH key (this disk's only: never in the image) and the
# wadspace base in root's podman storage. Then the builder, its registry and
# everything else stop. The builder's own disk is kept (it has podman);
# --fresh starts it from the cloud image again.
#
# **boot**: UEFI (Fedora's edk2-ovmf; SECURE_BOOT=1: with Secure Boot and
# Microsoft's keys, as a new PC ships), KVM, 2.5 GB, a virtio GPU labwc
# drives, no window (-display none): QMP (.build/vm/qmp.sock) takes the
# screenshots and the keys. Its sound card's output is recorded into
# .build/vm/sound.wav, and its SSH is on a port of 127.0.0.1 (.build/vm/ssh.port).
# It boots run.qcow2, a layer over wados.qcow2, so the installed disk stays
# as it was installed; check always starts from a fresh one.
#
# One VM at a time (this laptop has 7 GB), but for tailnet-check's two
# (smaller: TAILNET_VM_MEM): disk stops every test VM first.
# VM_MEM (MB), VM_CPUS, VM_RES (1920x1080), SECURE_BOOT=1, VM_KEEP=1 (check: leave it running).
set -euo pipefail

if [[ "${1:-}" == --name ]]; then
    [[ $# -ge 2 ]] || { echo "vm.sh: --name NAME" >&2; exit 2; }
    VM_NAME="$2"
    shift 2
fi

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
HERE="${ROOT}/tools/vm"
VM="${ROOT}/.build/vm"
CACHE="${XDG_CACHE_HOME:-${HOME}/.cache}/wados/vm"
FEDORA="https://download.fedoraproject.org/pub/fedora/linux/releases/44/Cloud/x86_64/images"
CLOUD="Fedora-Cloud-Base-Generic-44-1.7.x86_64.qcow2"
CHECKSUM="Fedora-Cloud-44-1.7-x86_64-CHECKSUM"
FEDORA_KEY=/etc/pki/rpm-gpg/RPM-GPG-KEY-fedora-44-primary
OVMF=/usr/share/edk2/ovmf
MEM="${VM_MEM:-2560}"
CPUS="${VM_CPUS:-2}"
RES="${VM_RES:-1920x1080}"
KEY="${VM}/id_ed25519"
# This VM's own files: the layer, its firmware variables, pid, QMP, serial,
# sound and SSH port. The disk (wados.qcow2) and the key are shared.
VM_NAME="${VM_NAME:-}"
[[ -z "${VM_NAME}" || "${VM_NAME}" =~ ^[a-z0-9]+$ ]] || { echo "vm.sh: VM_NAME: letters and digits" >&2; exit 2; }
RUN="${VM}${VM_NAME:+/${VM_NAME}}"
REGISTRY=wados-vm-registry
BASE_IMAGE=localhost/wados-base:44
# sam's password on the test disk: a test user's, typed by QMP. Not a secret.
SAM_PASSWORD="${VM_PASSWORD:-Wados-44!}"

die() { echo "vm.sh: $*" >&2; exit 1; }
say() { printf '\n== %s\n' "$*"; }
now() { date +%s; }

free_port() {
    python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])'
}

# A QEMU of ours running from its pidfile.
running() {
    local pid
    pid="$(cat "$1" 2> /dev/null)" || return 1
    [[ -n "${pid}" ]] && kill -0 "${pid}" 2> /dev/null && grep -q qemu "/proc/${pid}/cmdline" 2> /dev/null
}

ssh_opts() {
    printf '%s\n' -i "${KEY}" -p "$1" -o BatchMode=yes -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
        -o LogLevel=ERROR -o ConnectTimeout=5 -o ServerAliveInterval=10 -o ServerAliveCountMax=6
}

# ssh_to PORT USER [COMMAND…]
ssh_to() {
    local port="$1" user="$2"
    shift 2
    local opts
    mapfile -t opts < <(ssh_opts "${port}")
    ssh "${opts[@]}" "${user}@127.0.0.1" "$@"
}

# wait_ssh PORT USER SECONDS PIDFILE
wait_ssh() {
    local deadline=$(($(now) + $3))
    while (($(now) < deadline)); do
        running "$4" || return 1
        ssh_to "$1" "$2" true 2> /dev/null && return 0
        sleep 3
    done
    return 1
}

need_key() {
    mkdir -p "${VM}"
    [[ -f "${KEY}" ]] || ssh-keygen -q -t ed25519 -N '' -C wados-vm -f "${KEY}"
}

firmware() {
    if [[ "${SECURE_BOOT:-0}" == 1 ]]; then
        CODE="${OVMF}/OVMF_CODE.secboot.fd" VARS="${OVMF}/OVMF_VARS.secboot.fd"
    else
        CODE="${OVMF}/OVMF_CODE.fd" VARS="${OVMF}/OVMF_VARS.fd"
    fi
    [[ -r "${CODE}" && -r "${VARS}" ]] || die "no UEFI firmware in ${OVMF} (Fedora's edk2-ovmf)"
}

# ------------------------------------------------------------------ disk

# Fedora Cloud Base, downloaded once, its CHECKSUM file's signature checked
# with Fedora 44's key and the image against it.
cloud_image() {
    mkdir -p "${CACHE}"
    local img="${CACHE}/${CLOUD}" sums="${CACHE}/${CHECKSUM}"
    [[ -f "${sums}" ]] || curl -sSLf -o "${sums}" "${FEDORA}/${CHECKSUM}"
    if [[ -r "${FEDORA_KEY}" ]] && command -v gpgv > /dev/null; then
        local ring
        ring="$(mktemp)"
        gpg --dearmor < "${FEDORA_KEY}" > "${ring}"
        gpgv --keyring "${ring}" "${sums}" 2> /dev/null || { rm -f "${ring}"; die "${CHECKSUM}: a bad signature"; }
        rm -f "${ring}"
    else
        echo "   (no ${FEDORA_KEY}: the CHECKSUM file's signature isn't checked)"
    fi
    local want
    want="$(sed -n "s/^SHA256 (${CLOUD}) = \([0-9a-f]*\)$/\1/p" "${sums}")"
    [[ -n "${want}" ]] || die "${CLOUD} isn't in ${CHECKSUM}"
    if [[ ! -f "${img}" ]]; then
        echo "   downloading ${CLOUD}"
        curl -sSLf -o "${img}.part" "${FEDORA}/${CLOUD}"
        mv "${img}.part" "${img}"
    fi
    [[ "$(sha256sum "${img}" | cut -d' ' -f1)" == "${want}" ]] || { rm -f "${img}"; die "${CLOUD}: wrong checksum (removed)"; }
    echo "   ${CLOUD}: checksum ok"
}

seed_iso() {
    mkdir -p "${VM}/seed"
    cat > "${VM}/seed/meta-data" << EOF
instance-id: wados-builder
local-hostname: wados-builder
EOF
    cat > "${VM}/seed/user-data" << EOF
#cloud-config
disable_root: false
ssh_pwauth: false
ssh_authorized_keys:
  - $(cat "${KEY}.pub")
growpart:
  mode: auto
  devices: ["/"]
EOF
    xorriso -as mkisofs -quiet -output "${VM}/seed.iso" -volid cidata -joliet -rock \
        "${VM}/seed/user-data" "${VM}/seed/meta-data" 2> /dev/null
}

registry_stop() {
    podman rm -f -t 2 "${REGISTRY}" > /dev/null 2>&1 || true
}

builder_stop() {
    local pid
    running "${VM}/builder.pid" || { rm -f "${VM}/builder.pid" "${VM}/builder.qmp"; return 0; }
    pid="$(cat "${VM}/builder.pid")"
    [[ -n "${BUILDER_PORT:-}" ]] && ssh_to "${BUILDER_PORT}" root poweroff > /dev/null 2>&1 || true
    for _ in $(seq 60); do kill -0 "${pid}" 2> /dev/null || break; sleep 1; done
    if kill -0 "${pid}" 2> /dev/null; then
        python3 "${HERE}/qmp.py" "${VM}/builder.qmp" quit 2> /dev/null || kill "${pid}" 2> /dev/null || true
        for _ in $(seq 10); do kill -0 "${pid}" 2> /dev/null || break; sleep 1; done
    fi
    rm -f "${VM}/builder.pid" "${VM}/builder.qmp"
}

disk() {
    local fresh="" image=""
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --fresh) fresh=1; shift ;;
            -*) die "disk: unknown option $1" ;;
            *) image="$1"; shift ;;
        esac
    done
    image="${image:-localhost/wados-host:latest}"
    local target="${VM_TARGET_IMGREF:-${image}}" t0 t
    t0=$(now)
    podman image exists "${image}" || die "no ${image} here: host/build.sh image (or name another, e.g. quay.io/fedora/fedora-bootc:44)"
    podman image exists "${BASE_IMAGE}" || die "no ${BASE_IMAGE} here: images/build.sh"
    [[ -w /dev/kvm ]] || die "no /dev/kvm"
    local d
    for d in "${VM}" "${VM}"/*/; do
        d="${d%/}"
        if running "${d}/qemu.pid"; then
            echo "   the test VM$([[ "${d}" != "${VM}" ]] && echo " ${d##*/}") is running: stopping it first (one VM at a time)"
            RUN="${d}" stop
        fi
    done
    running "${VM}/builder.pid" && die "a builder VM is running already (pid $(cat "${VM}/builder.pid"))"
    need_key

    say "the builder: Fedora Cloud Base 44"
    t=$(now)
    cloud_image
    if [[ -n "${fresh}" || ! -f "${VM}/builder.qcow2" ]]; then
        rm -f "${VM}/builder.qcow2"
        qemu-img create -q -f qcow2 -F qcow2 -b "${CACHE}/${CLOUD}" "${VM}/builder.qcow2" 24G
    fi
    seed_iso
    # Written as wados-new.qcow2, and only a whole install becomes wados.qcow2.
    rm -f "${VM}/wados-new.qcow2"
    qemu-img create -q -f qcow2 "${VM}/wados-new.qcow2" 40G
    echo "   ($(($(now) - t)) s)"

    trap 'set +e; builder_stop; registry_stop' EXIT
    say "a registry on the laptop, with ${image} and ${BASE_IMAGE}"
    t=$(now)
    registry_stop
    local reg
    reg="$(free_port)"
    podman run -d --rm --name "${REGISTRY}" -p "127.0.0.1:${reg}:5000" docker.io/library/registry:2 > /dev/null
    for _ in $(seq 50); do curl -sf "http://127.0.0.1:${reg}/v2/" > /dev/null && break; sleep 0.2; done
    # zstd, quickly: it's unpacked a minute later.
    local push=(podman push -q --tls-verify=false --compression-format zstd --compression-level 1)
    "${push[@]}" "${image}" "localhost:${reg}/wados-host:latest"
    "${push[@]}" "${BASE_IMAGE}" "localhost:${reg}/${BASE_IMAGE#localhost/}"
    echo "   127.0.0.1:${reg} ($(($(now) - t)) s)"

    say "the builder VM, with wados.qcow2 as its second disk"
    t=$(now)
    BUILDER_PORT="$(free_port)"
    qemu-system-x86_64 -name wados-vm-builder \
        -machine q35,accel=kvm -cpu host -smp "${CPUS}" -m "${MEM}" \
        -drive "if=none,id=os,format=qcow2,file=${VM}/builder.qcow2,cache=unsafe,discard=unmap" \
        -device virtio-blk-pci,drive=os,serial=builder,bootindex=0 \
        -drive "if=none,id=target,format=qcow2,file=${VM}/wados-new.qcow2,discard=unmap" \
        -device virtio-blk-pci,drive=target,serial=wados-target \
        -drive "if=none,id=seed,format=raw,file=${VM}/seed.iso,readonly=on" \
        -device virtio-blk-pci,drive=seed \
        -netdev "user,id=net,hostfwd=tcp:127.0.0.1:${BUILDER_PORT}-:22" -device virtio-net-pci,netdev=net \
        -device virtio-rng-pci \
        -display none -serial "file:${VM}/builder-serial.log" \
        -qmp "unix:${VM}/builder.qmp,server=on,wait=off" \
        -pidfile "${VM}/builder.pid" -daemonize
    wait_ssh "${BUILDER_PORT}" root 300 "${VM}/builder.pid" || die "the builder's SSH didn't come up (${VM}/builder-serial.log)"
    echo "   up ($(($(now) - t)) s)"

    say "installing ${image} onto wados.qcow2 (tools/vm/install-disk.sh)"
    t=$(now)
    ssh_to "${BUILDER_PORT}" root 'cat > /root/wados-vm.pub' < "${KEY}.pub"
    ssh_to "${BUILDER_PORT}" root 'cat > /root/install-disk.sh' < "${HERE}/install-disk.sh"
    ssh_to "${BUILDER_PORT}" root bash /root/install-disk.sh "10.0.2.2:${reg}" wados-host:latest "${target}" \
        "${BASE_IMAGE#localhost/}" /root/wados-vm.pub < /dev/null
    echo "   ($(($(now) - t)) s)"

    say "stopping the builder and the registry"
    t=$(now)
    builder_stop
    registry_stop
    trap - EXIT
    rm -f "${VM}/run.qcow2" "${VM}/vars.fd" "${VM}"/*/run.qcow2 "${VM}"/*/vars.fd
    mv -f "${VM}/wados-new.qcow2" "${VM}/wados.qcow2"
    echo "   ($(($(now) - t)) s)"
    echo
    echo "${VM}/wados.qcow2: $(du -h "${VM}/wados.qcow2" | cut -f1) on disk, from ${image} ($(($(now) - t0)) s in all)"
    echo "next: tools/vm.sh boot, or tools/vm.sh check"
}

# ------------------------------------------------------------------ boot

boot() {
    local fresh=""
    [[ "${1:-}" == --fresh ]] && fresh=1
    [[ -f "${VM}/wados.qcow2" ]] || die "no ${VM}/wados.qcow2: tools/vm.sh disk"
    running "${VM}/builder.pid" && die "the builder VM is running (one VM at a time)"
    if running "${RUN}/qemu.pid"; then
        echo "running already (ssh on 127.0.0.1:$(cat "${RUN}/ssh.port"))"
        return 0
    fi
    need_key
    firmware
    mkdir -p "${RUN}"
    if [[ -n "${fresh}" || ! -f "${RUN}/run.qcow2" ]]; then
        rm -f "${RUN}/run.qcow2" "${RUN}/vars.fd"
        qemu-img create -q -f qcow2 -F qcow2 -b "${VM}/wados.qcow2" "${RUN}/run.qcow2"
    fi
    [[ -f "${RUN}/vars.fd" ]] || cp "${VARS}" "${RUN}/vars.fd"
    local port xres yres
    port="$(free_port)"
    echo "${port}" > "${RUN}/ssh.port"
    xres="${RES%x*}" yres="${RES#*x}"
    local args=(
        -name "wados-vm${VM_NAME:+-${VM_NAME}}"
        -machine "q35,accel=kvm$([[ "${SECURE_BOOT:-0}" == 1 ]] && echo ,smm=on)" -cpu host -smp "${CPUS}" -m "${MEM}"
    )
    [[ "${SECURE_BOOT:-0}" == 1 ]] && args+=(-global driver=cfi.pflash01,property=secure,value=on)
    args+=(
        -drive "if=pflash,format=raw,unit=0,readonly=on,file=${CODE}"
        -drive "if=pflash,format=raw,unit=1,file=${RUN}/vars.fd"
        -drive "if=none,id=disk,format=qcow2,file=${RUN}/run.qcow2,discard=unmap"
        -device virtio-blk-pci,drive=disk,bootindex=1
        # A virtio GPU (KMS, no 3D): labwc and the shell draw in software.
        -device "virtio-vga,xres=${xres},yres=${yres}"
        -display none
        -device virtio-tablet-pci
        -netdev "user,id=net,hostfwd=tcp:127.0.0.1:${port}-:22" -device virtio-net-pci,netdev=net
        -device virtio-rng-pci
        # No S3: OVMF's resume from it resets the VM. Linux suspends to
        # idle instead (s2idle), and a key wakes it (check.sh).
        -global ICH9-LPC.disable_s3=1
        # What its sound card plays, recorded.
        -audiodev "wav,id=snd0,path=${RUN}/sound.wav"
        -device intel-hda -device hda-duplex,audiodev=snd0
        -qmp "unix:${RUN}/qmp.sock,server=on,wait=off"
        -serial "file:${RUN}/serial.log"
        -pidfile "${RUN}/qemu.pid" -daemonize
    )
    # A named VM runs beside another (tailnet-check): what its Linux frees
    # goes back to the laptop (free page reporting), not kept by QEMU.
    [[ -n "${VM_NAME}" ]] && args+=(-device virtio-balloon-pci,free-page-reporting=on)
    # (hda-duplex's line in has nothing to record from: QEMU says so.)
    qemu-system-x86_64 "${args[@]}" 2> >(grep -v "backend for voice \`adc'" >&2)
    echo "booting${VM_NAME:+ ${VM_NAME}} (pid $(cat "${RUN}/qemu.pid")): ssh on 127.0.0.1:${port}, screen and keys on ${RUN}/qmp.sock"
}

stop() {
    local pid
    if ! running "${RUN}/qemu.pid"; then
        rm -f "${RUN}/qemu.pid" "${RUN}/qmp.sock"
        echo "not running"
        return 0
    fi
    pid="$(cat "${RUN}/qemu.pid")"
    python3 "${HERE}/qmp.py" "${RUN}/qmp.sock" powerdown 2> /dev/null || true
    for _ in $(seq 60); do kill -0 "${pid}" 2> /dev/null || break; sleep 1; done
    if kill -0 "${pid}" 2> /dev/null; then
        python3 "${HERE}/qmp.py" "${RUN}/qmp.sock" quit 2> /dev/null || kill "${pid}" 2> /dev/null || true
        for _ in $(seq 10); do kill -0 "${pid}" 2> /dev/null || break; sleep 1; done
    fi
    rm -f "${RUN}/qemu.pid" "${RUN}/qmp.sock"
    echo "stopped"
}

status() {
    if running "${RUN}/qemu.pid"; then
        echo "test VM${VM_NAME:+ ${VM_NAME}}: running (pid $(cat "${RUN}/qemu.pid"), ssh on 127.0.0.1:$(cat "${RUN}/ssh.port"), $(python3 "${HERE}/qmp.py" "${RUN}/qmp.sock" status))"
    else
        echo "test VM${VM_NAME:+ ${VM_NAME}}: not running"
    fi
    local d
    if [[ -z "${VM_NAME}" ]]; then
        for d in "${VM}"/*/; do
            d="${d%/}"
            running "${d}/qemu.pid" && echo "test VM ${d##*/}: running (pid $(cat "${d}/qemu.pid"), ssh on 127.0.0.1:$(cat "${d}/ssh.port"))"
        done
    fi
    running "${VM}/builder.pid" && echo "builder VM: running (pid $(cat "${VM}/builder.pid"))"
    podman container exists "${REGISTRY}" 2> /dev/null && echo "registry: running"
    [[ -f "${VM}/wados.qcow2" ]] && echo "disk: ${VM}/wados.qcow2 ($(du -h "${VM}/wados.qcow2" | cut -f1) on disk)"
    return 0
}

need_vm() {
    running "${RUN}/qemu.pid" || die "the VM${VM_NAME:+ ${VM_NAME}} isn't running: tools/vm.sh${VM_NAME:+ --name ${VM_NAME}} boot"
}

vm_ssh() {
    need_vm
    ssh_to "$(cat "${RUN}/ssh.port")" "${VM_USER:-root}" "$@"
}

qmp() {
    need_vm
    python3 "${HERE}/qmp.py" "${RUN}/qmp.sock" "$@"
}

# ------------------------------------------------------------------ check
# tools/vm/check.sh: the end-to-end check, with these helpers.

# ------------------------------------------------------------------

cmd="${1:-}"
[[ $# -gt 0 ]] && shift
case "${cmd}" in
    disk) disk "$@" ;;
    boot) boot "$@" ;;
    ssh) vm_ssh "$@" ;;
    shot) [[ $# -eq 1 ]] || die "shot FILE"; qmp shot "$1" ;;
    keys) [[ $# -eq 1 ]] || die "keys 'text'"; qmp keys "$1" ;;
    key) [[ $# -ge 1 ]] || die "key COMBO…"; qmp key "$@" ;;
    stop) stop ;;
    status) status ;;
    check) [[ -z "${VM_NAME}" ]] || die "check is the unnamed VM's"; . "${HERE}/check.sh"; check ;;
    tailnet-check) [[ -z "${VM_NAME}" ]] || die "tailnet-check names its own VMs"; . "${HERE}/check-tailnet.sh"; tailnet_check "$@" ;;
    *) sed -n '2,26p' "$0" | sed 's/^# \{0,1\}//'; exit 2 ;;
esac
