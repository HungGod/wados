#!/usr/bin/env bash
# Secure streams (phase 7) without wadd: a wadspace streamed from inside a
# libkrun micro-VM (podman --runtime=krun), in a test VM of tools/vm.sh
# (nested KVM), from the real pieces: wad-gen's units (secure_units, through
# its secure-units example), the host image's guest side
# (host/usr/lib/wados/secure), the stream image (localhost/wados-stream:44,
# from the laptop) and the wadspace base. Nothing is installed on the laptop.
#
#   tools/krun/prototype.sh setup [--reload]
#        boots the test VM `krun` (tools/vm.sh --name krun) if it isn't up;
#        crun-krun into a throwaway /usr (bootc usr-overlay), as the host
#        image has it; host/usr/lib/wados/secure where the host image keeps
#        it; the stream image into root's podman (--reload: again)
#   tools/krun/prototype.sh plain     krun's own checks: fedora:44 and wados-base:44, timed
#   tools/krun/prototype.sh up [--check]
#        wad-gen's units for `krunproto`, its stream's folder (a token table)
#        and master token as wadd makes them, started by systemd, until
#        Selkies' health answers (--check: with a project folder from the
#        VM's host and guest/check's hook)
#   tools/krun/prototype.sh view [OUT.png] [SECONDS]
#                                     a browser on the laptop, through an SSH tunnel, on its stream
#   tools/krun/prototype.sh check [OUT.png]
#                                     view, and part-way through the guest plays a tone (after up
#                                     --check): the audio reaching the browser, the folder both ways
#   tools/krun/prototype.sh logs      the wadspace's log
#   tools/krun/prototype.sh down      stops it (timed: SIGINT, a clean s6 stop) and takes away
#                                     what up made
#   tools/krun/prototype.sh vm-stop   powers the test VM off
#
# VM_NAME (krun), VM_MEM (2560), KRUN_CPUS (2), KRUN_RAM_MIB (1536), PORT (48190).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
HERE="${ROOT}/tools/krun"
NAME="${VM_NAME:-krun}"
RUN="${ROOT}/.build/vm/${NAME}"
OUT_DIR="${ROOT}/.build/krun"
STREAM_IMAGE=localhost/wados-stream:44
ID=krunproto
GUID=proto
PORT="${PORT:-48190}"
UNITS=/run/containers/systemd
STATE=/var/lib/wados-krun
AUTH="${STATE}/streams/${ID}"
PROJECT="${STATE}/project"

vm() { "${ROOT}/tools/vm.sh" --name "${NAME}" "$@"; }
say() { printf '\n== %s\n' "$*"; }
die() { echo "prototype.sh: $*" >&2; exit 1; }

setup() {
    local reload=""
    [[ "${1:-}" == --reload ]] && reload=1
    if ! vm status | grep -q running; then
        say "booting the test VM ${NAME} ($(free -m | awk '/^Mem:/ {print $7}') MB free on the laptop)"
        vm boot
        for _ in $(seq 60); do vm ssh true 2> /dev/null && break; sleep 3; done
    fi
    say "crun-krun, into a throwaway /usr (bootc usr-overlay), as host/Containerfile installs it"
    vm ssh 'set -e
        [ -c /dev/kvm ] || { echo "no /dev/kvm in the VM (nested KVM?)"; exit 1; }
        if ! command -v krun > /dev/null; then
            touch /usr/.wados-rw-test 2> /dev/null && rm -f /usr/.wados-rw-test || bootc usr-overlay
            dnf -y -q --setopt=install_weak_deps=False install crun-krun
        fi
        rpm -q crun crun-krun libkrun libkrunfw | sed "s/^/   /"'
    say "the guest side, where the host image keeps it (host/usr/lib/wados/secure)"
    # A disk from this tree has it already; else (an older one) into the /usr overlay.
    local want
    want="$(cd "${ROOT}/host/usr/lib/wados/secure" && find . -type f | sort | xargs sha256sum | sha256sum | cut -c1-16)"
    tar c -C "${ROOT}/host/usr/lib/wados" secure | vm ssh "set -e
        have=\$(cd /usr/lib/wados/secure 2> /dev/null && find . -type f | sort | xargs sha256sum | sha256sum | cut -c1-16) || true
        if [ \"\${have}\" = ${want} ]; then
            echo '   the host image has it (as in the tree)'; cat > /dev/null
        else
            touch /usr/.wados-rw-test 2> /dev/null && rm -f /usr/.wados-rw-test || bootc usr-overlay > /dev/null
            rm -rf /usr/lib/wados/secure && mkdir -p /usr/lib/wados && tar x --no-same-owner -C /usr/lib/wados
            restorecon -R /usr/lib/wados/secure
            echo '   copied from the tree (into the /usr overlay)'
        fi
        ls -lZ /usr/lib/wados/secure | sed 's/^/   /'"
    tar c -C "${HERE}/guest" check | vm ssh "set -e
        rm -rf ${STATE}/check && mkdir -p ${STATE} && tar x --no-same-owner -C ${STATE}
        chcon -R -t container_file_t -l s0 ${STATE}/check"
    if [[ -n "${reload}" ]] || ! vm ssh "podman image exists ${STREAM_IMAGE}"; then
        podman image exists "${STREAM_IMAGE}" || die "no ${STREAM_IMAGE} on the laptop: images/build.sh --only stream"
        say "${STREAM_IMAGE}, from the laptop into the VM's root podman"
        local t
        t=$(date +%s)
        podman save "${STREAM_IMAGE}" | vm ssh 'podman load -q'
        echo "   ($(($(date +%s) - t)) s)"
    fi
}

plain() {
    vm ssh 'el() { python3 -c "print(round($EPOCHREALTIME - $1, 2))"; }
        podman image exists registry.fedoraproject.org/fedora:44 || podman pull -q registry.fedoraproject.org/fedora:44 > /dev/null
        echo "== podman run --runtime=krun fedora:44 uname -a"
        podman run --runtime=krun --rm registry.fedoraproject.org/fedora:44 uname -a
        for rt in krun crun; do t=$EPOCHREALTIME; podman run --runtime=${rt} --rm registry.fedoraproject.org/fedora:44 true; echo "   ${rt}: $(el $t) s for true"; done
        echo "== the VM by default, with krun.cpus=1 krun.ram_mib=1536, and with --memory 768m"
        podman run --runtime=krun --rm registry.fedoraproject.org/fedora:44 sh -c "echo \"   \$(nproc) vCPUs, \$(grep MemTotal /proc/meminfo)\""
        podman run --runtime=krun --rm --annotation krun.cpus=1 --annotation krun.ram_mib=1536 registry.fedoraproject.org/fedora:44 sh -c "echo \"   \$(nproc) vCPUs, \$(grep MemTotal /proc/meminfo)\""
        podman run --runtime=krun --rm --memory 768m registry.fedoraproject.org/fedora:44 sh -c "echo \"   \$(nproc) vCPUs, \$(grep MemTotal /proc/meminfo)\""
        echo "== localhost/wados-base:44 with a command"
        t=$EPOCHREALTIME; podman run --runtime=krun --rm --entrypoint "" localhost/wados-base:44 sh -c "labwc --version; id abc"; echo "   $(el $t) s"
        echo "== its own /init, without secure-init (s6-overlay wants PID 1; init.krun is)"
        podman run --runtime=krun --rm localhost/wados-base:44 2>&1 | tail -1
        echo "== podman exec"
        podman run --runtime=krun -d --name krun-exec-check --entrypoint "" localhost/wados-base:44 sleep 60 > /dev/null
        podman exec krun-exec-check true 2>&1 | tail -1
        podman rm -f -t 0 krun-exec-check > /dev/null
        echo "== SELinux denials since boot"
        ausearch -m avc,user_avc -ts boot 2> /dev/null | grep -c "avc:" || true'
}

# The check's extras (up --check): a folder from the VM's host (labelled for
# containers, as a GitHub clone wadd made is), and the check's hook.
CHECK_LINES="Volume=${PROJECT}:/config/Desktop/Project:rw\\nVolume=${STATE}/check/90-krun-check.sh:/etc/wadspaces/autostart.d/90-krun-check.sh:ro"

up() {
    local check=""
    [[ "${1:-}" == --check ]] && check=1
    say "wad-gen's units (secure_units: crates/wad-gen/examples/secure-units.rs)"
    local gen="${OUT_DIR}/gen"
    rm -rf "${gen}"
    (cd "${ROOT}" && cargo run -q -p wad-gen --example secure-units -- --units "${gen}/units" --root "${gen}/root" \
        --state "${STATE}" --guid "${GUID}" --web-port "${PORT}" --auth "${AUTH}" --wallpaper '#2e86de' \
        --cpus "${KRUN_CPUS:-2}" --ram-mib "${KRUN_RAM_MIB:-1536}")
    if [[ -n "${check}" ]]; then
        sed -i "/^Entrypoint=/i ${CHECK_LINES}" "${gen}/units/wad-${ID}.container"
    fi
    grep -E '^(GlobalArgs|Annotation|Entrypoint|Stop|PublishPort|Volume=wadsecure)' "${gen}/units/wad-${ID}.container" | sed 's/^/   /'
    # The stream's key (Selkies' token: the one-tap link's) and master token, as wadd makes them.
    local key master
    key="$(python3 -c 'import secrets; print(secrets.token_hex(16))')"
    master="$(python3 -c 'import secrets; print(secrets.token_hex(32))')"
    mkdir -p "${OUT_DIR}"
    (umask 077 && printf '%s' "${key}" > "${OUT_DIR}/key")
    say "into the VM: units, extra folder, the stream's folder and master token"
    tar c -C "${gen}" units root | vm ssh "set -e
        el() { python3 -c \"print(round(\$EPOCHREALTIME - \$1, 2))\"; }
        systemctl stop wad-${ID}.service 2> /dev/null || true
        tmp=\$(mktemp -d); tar x -C \"\${tmp}\"
        mkdir -p ${UNITS} ${STATE}
        cp \"\${tmp}\"/units/* ${UNITS}/
        rm -rf ${STATE}/extra && cp -a \"\${tmp}\"/root${STATE}/extra ${STATE}/
        chmod 755 ${STATE}/extra/${ID}
        chcon -R -t container_file_t -l s0 ${STATE}/extra
        rm -rf \"\${tmp}\"
        # The stream's folder: 0700, its token table 0600, the machine's user's (wadd's write_folder).
        rm -rf ${AUTH}; mkdir -p ${AUTH}; chmod 700 ${AUTH}
        printf '{\"%s\": {\"role\": \"controller\", \"slot\": null}}' '${key}' > ${AUTH}/tokens.json
        chmod 600 ${AUTH}/tokens.json; chown -R 1000:1000 ${AUTH}
        podman secret rm wad_stream_master_${ID} > /dev/null 2>&1 || true
        printf '%s' '${master}' | podman secret create wad_stream_master_${ID} - > /dev/null
        if [ -n '${check}' ]; then
            rm -rf ${PROJECT}; mkdir -p ${PROJECT}; chown 1000:1000 ${PROJECT}; chcon -t container_file_t -l s0 ${PROJECT}
        fi
        systemctl daemon-reload
        # The image volume, made again from the stream image (wadd's drop_secure_volume).
        systemctl stop wadsecure-${ID}-volume.service 2> /dev/null || true
        podman volume rm -f wadsecure-${ID} > /dev/null 2>&1 || true
        t=\$EPOCHREALTIME; systemctl start wad-${ID}.service; echo \"   started: \$(el \$t) s\"
        url=http://127.0.0.1:${PORT}/ws-${GUID}/api/health
        for _ in \$(seq 480); do curl -sf -o /dev/null \$url && break; systemctl -q is-active wad-${ID}.service || break; sleep 0.5; done
        curl -sf -o /dev/null \$url && echo \"   Selkies' health answers (\$url): \$(el \$t) s\" ||
            { echo '   no answer'; journalctl -u wad-${ID} --no-pager | tail -30; exit 1; }
        for _ in \$(seq 60); do podman logs wad-${ID} 2>&1 | grep -q 'drawing on' && break; sleep 0.5; done
        podman logs wad-${ID} 2>&1 | grep -E '^\[(svc-de|secure-stream|secure-init)\]|^INFO:main|^\[Wayland\] Socket' | sed 's/^/   /'
        echo \"   runtime: \$(podman inspect wad-${ID} --format '{{.OCIRuntime}}'); no sidecar: \$(podman container exists wadstream-${ID} && echo NO || echo yes)\"
        echo \"   the VM (libkrun's VMM, on the host): \$(ps -o pid=,rss=,label= -p \$(podman inspect wad-${ID} --format '{{.State.Pid}}') | awk '{printf \"pid %s, %d MB resident, %s\", \$1, \$2/1024, \$3}')\"
        echo \"   without a token: \$(curl -s -o /dev/null -w '%{http_code}' -H 'Connection: Upgrade' -H 'Upgrade: websocket' -H 'Sec-WebSocket-Version: 13' -H 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==' http://127.0.0.1:${PORT}/ws-${GUID}/api/websockets) on its socket\""
}

view() {
    local out="${1:-${OUT_DIR}/stream.png}" secs="${2:-20}" lport pid key
    key="$(cat "${OUT_DIR}/key" 2> /dev/null)" || die "no key: up first"
    mkdir -p "$(dirname "${out}")"
    lport="$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')"
    say "a browser on the laptop, on the VM's 127.0.0.1:${PORT} (ssh -L ${lport}), its one-tap link"
    ssh -i "${ROOT}/.build/vm/id_ed25519" -p "$(cat "${RUN}/ssh.port")" -o BatchMode=yes -o StrictHostKeyChecking=no \
        -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ExitOnForwardFailure=yes \
        -N -L "127.0.0.1:${lport}:127.0.0.1:${PORT}" root@127.0.0.1 &
    pid=$!
    for _ in $(seq 40); do curl -sf -o /dev/null "http://127.0.0.1:${lport}/ws-${GUID}/api/health" && break; sleep 0.25; done
    local rc=0
    # WADOS_KRUN_VIEWER: a browser viewer taking URL OUT.png SECONDS (the
    # Node one, view.mjs, went with the v2 spike's Playwright).
    if [[ -n "${WADOS_KRUN_VIEWER:-}" ]]; then
        ${WADOS_KRUN_VIEWER} "http://localhost:${lport}/ws-${GUID}/#token=${key}" "${out}" "${secs}" || rc=$?
    else
        echo "   no browser viewer (WADOS_KRUN_VIEWER): not looked at"
    fi
    kill "${pid}" 2> /dev/null || true
    wait "${pid}" 2> /dev/null || true
    echo "   ${out}"
    return "${rc}"
}

check() {
    local out="${1:-${OUT_DIR}/stream.png}"
    TRIGGER_AFTER=6 TRIGGER_CMD="'${ROOT}/tools/vm.sh' --name '${NAME}' ssh 'touch ${PROJECT}/play && chown 1000:1000 ${PROJECT}/play'" \
        view "${out}" 22
    say "what the guest saw (its files in the project folder, read on the host)"
    tar c -C "${HERE}" wavstat.py | vm ssh "set -e; mkdir -p /root/krun; tar x -C /root/krun
        P=${PROJECT}
        for _ in \$(seq 20); do [ -f \"\${P}/guest.txt\" ] && grep -q recorded \"\${P}/guest.txt\" && break; sleep 1; done
        sed 's/^/   /' \"\${P}/guest.txt\" 2> /dev/null || echo '   no guest.txt: the hook never saw play'
        ls -lnZ \"\${P}\" | sed 's/^/   /'
        [ -f \"\${P}/monitor.wav\" ] && python3 /root/krun/wavstat.py \"\${P}/monitor.wav\""
}

logs() { vm ssh "podman logs wad-${ID} 2>&1 | tail -${1:-60}"; }

down() {
    vm ssh "el() { python3 -c \"print(round(\$EPOCHREALTIME - \$1, 2))\"; }
        t=\$EPOCHREALTIME; systemctl stop wad-${ID}.service; echo \"   stopped: \$(el \$t) s (\$(systemctl show wad-${ID} -p Result --value), status \$(systemctl show wad-${ID} -p ExecMainStatus --value))\"
        journalctl -u wad-${ID} --no-pager -n 200 | grep -E 'secure-init|SIGKILL' | tail -3 | sed 's/^/   /'
        # What wadd's teardown takes away: the image volume's unit, then the
        # volume; the units; the master token; the stream's folder.
        systemctl stop wadsecure-${ID}-volume.service 2> /dev/null || true
        podman volume rm -f wadsecure-${ID} > /dev/null 2>&1 || true
        rm -f ${UNITS}/wad-${ID}.container ${UNITS}/wadsecure-${ID}.volume
        systemctl daemon-reload
        podman secret rm wad_stream_master_${ID} > /dev/null 2>&1 || true
        rm -rf ${AUTH}
        left=\$( (podman ps -a --format '{{.Names}}'; podman volume ls --format '{{.Name}}'; podman secret ls --format '{{.Name}}') | grep -c ${ID} || true)
        echo \"   left of it in podman: \${left}\""
    rm -f "${OUT_DIR}/key"
}

cmd="${1:-}"
[[ $# -gt 0 ]] && shift
case "${cmd}" in
    setup) setup "$@" ;;
    plain) plain ;;
    up) up "$@" ;;
    view) view "$@" ;;
    check) check "$@" ;;
    logs) logs "$@" ;;
    down) down ;;
    vm-stop) vm stop ;;
    *) sed -n '2,31p' "$0" | sed 's/^# \{0,1\}//'; exit 2 ;;
esac
