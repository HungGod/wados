#!/usr/bin/env bash
# A private secure stream through wadd, end to end, on a VM of tools/vm.sh:
# started as the shell starts it (wadd's socket, as sam), streamed in a
# browser, stopped, and nothing of it left. For the VM tests (tools/vm/,
# whose owner calls this or folds it in); tools/krun/NOTES.md says why each
# part is as it is.
#
#   tools/krun/check-secure.sh [--name VM] [--keep]
#
# The VM must run a host image built from this tree (wadd with secure
# streams, crun-krun, /usr/lib/wados/secure), with localhost/wados-stream:44
# in root's podman, /dev/kvm (nested KVM), sam's session up, and the machine
# on a tailnet (private streams need one: tools/vm.sh tailnet-check's VMs).
# What's missing is said, and fails. --keep: the wadspace isn't put in the
# Trash after.
#
# ok/FAIL lines, as tools/vm/check.sh's; exits 1 if any failed.
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
NAME="${VM_NAME:-}"
KEEP=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --name) NAME="$2"; shift 2 ;;
        --keep) KEEP=1; shift ;;
        *) sed -n '2,19p' "$0" | sed 's/^# \{0,1\}//'; exit 2 ;;
    esac
done
RUN="${ROOT}/.build/vm${NAME:+/${NAME}}"
OUT="${ROOT}/.build/krun/check-secure"
UNITS=/run/containers/systemd
STREAMS=/var/lib/wados/streams
FAILED=0

ok() { printf 'ok    %s\n' "$*"; }
fail() { printf 'FAIL  %s\n' "$*"; FAILED=$((FAILED + 1)); }
note() { printf '      %s\n' "$*"; }
now() { date +%s; }
vm() { "${ROOT}/tools/vm.sh" ${NAME:+--name "${NAME}"} "$@"; }
as_root() { vm ssh "$@"; }
as_sam() { VM_USER=sam vm ssh "$@"; }
# wadd's API through its socket, as sam: api METHOD PATH [BODY].
api() {
    local body="${3:-}"
    if [[ -n "${body}" ]]; then
        as_sam "curl -sS --unix-socket /run/wadd/wadd.sock -X $1 -H 'content-type: application/json' --data-binary @- http://wadd$2" <<< "${body}"
    else
        as_sam "curl -sS --unix-socket /run/wadd/wadd.sock -X $1 http://wadd$2" < /dev/null
    fi
}
stream_of() { api GET /v1/streams | jq -c --arg id "$1" '.streams[] | select(.id == $id and (.peer == null))'; }
finish() {
    echo
    if [[ ${FAILED} -eq 0 ]]; then echo "secure stream: all ok"; else echo "secure stream: ${FAILED} failed"; fi
    exit $((FAILED > 0))
}

mkdir -p "${OUT}"
echo "== what a secure stream needs"
need="$(as_root 'for c in "command -v krun" "test -c /dev/kvm" "test -x /usr/lib/wados/secure/secure-init" \
        "test -x /usr/lib/wados/secure/svc-secure-stream/run" "podman image exists localhost/wados-stream:44" \
        "test -S /run/wadd/wadd.sock"; do sh -c "$c" > /dev/null 2>&1 || echo "missing: $c"; done' 2>&1)"
if [[ -z "${need}" ]]; then ok "krun, /dev/kvm, the guest side, the stream image, wadd"; else fail "${need//$'\n'/; }"; finish; fi
avail="$(api GET /v1/streams | jq -c '.available["private-secure"]' 2> /dev/null)"
if [[ "$(jq -r .available <<< "${avail}")" == true ]]; then
    ok "private-secure can start"
else
    fail "private-secure can't start: ${avail}"
    finish
fi
public="$(api GET /v1/streams | jq -r '.available["public-secure"].reason' 2> /dev/null)"
if [[ "${public}" == "Secure public streams come later: secure streams are private for now" ]]; then
    ok "public-secure stays off: ${public}"
else
    fail "public-secure: ${public}"
fi

echo "== a wadspace, streamed securely"
id="$(api POST /v1/ops '{"op": {"op": "space.create", "name": "VM Secure"}}' | jq -r .created 2> /dev/null)"
[[ -n "${id}" && "${id}" != null ]] || { fail "no wadspace made"; finish; }
api POST /v1/ops "{\"op\": {\"op\": \"apps.add\", \"ws\": \"${id}\", \"app\": \"terminal\"}}" > /dev/null
b="$(api POST "/v1/wadspaces/${id}/build")"
if [[ "$(jq -r .state.built <<< "${b}")" == built ]]; then ok "${id}: built"; else fail "building ${id}: ${b}"; finish; fi
t=$(now)
job="$(api POST "/v1/wadspaces/${id}/stream" '{"mode": "private-secure"}')"
note "job: $(jq -c '{id: .job.id, stages: [.job.stages[].label]}' <<< "${job}" 2> /dev/null || echo "${job}")"
for _ in $(seq 200); do
    s="$(stream_of "${id}")"
    stage="$(jq -r .stage <<< "${s}" 2> /dev/null)"
    [[ "${stage}" == live || "${stage}" == ended || -z "${s}" ]] && break
    sleep 3
done
if [[ "${stage}" == live && "$(jq -r .live <<< "${s}")" == true ]]; then
    ok "live after $(($(now) - t)) s: $(jq -c '{mode, transport, link}' <<< "${s}")"
else
    fail "not live: ${s:-no stream}"
    as_root "journalctl -u wadd -u wad-${id} --no-pager -n 40" | sed 's/^/      | /'
    finish
fi

echo "== in its VM, with Selkies inside"
unit="$(as_root "cat ${UNITS}/wad-${id}.container" 2> /dev/null)"
port="$(sed -n 's/^PublishPort=127.0.0.1:\([0-9]*\):3000$/\1/p' <<< "${unit}")"
guid="$(sed -n 's/^Label=wadspaces.stream=//p' <<< "${unit}")"
if grep -qx 'GlobalArgs=--runtime=krun' <<< "${unit}" && grep -qx 'Entrypoint=/usr/libexec/wados-secure-init' <<< "${unit}"; then
    ok "its unit runs it in krun, s6 behind secure-init"
else
    fail "its unit: $(grep -E '^(GlobalArgs|Entrypoint)' <<< "${unit}" | tr '\n' ' ')"
fi
facts="$(as_root "podman inspect wad-${id} --format '{{.OCIRuntime}}'; podman container exists wadstream-${id} && echo sidecar || echo no-sidecar;
    ps -o label= -p \$(podman inspect wad-${id} --format '{{.State.Pid}}') | cut -d: -f3;
    podman volume inspect wadsecure-${id} --format '{{.Driver}}' 2> /dev/null;
    curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:${port}/ws-${guid}/api/health; echo;
    curl -s -o /dev/null -w '%{http_code}' -H 'Connection: Upgrade' -H 'Upgrade: websocket' -H 'Sec-WebSocket-Version: 13' \
        -H 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==' http://127.0.0.1:${port}/ws-${guid}/api/websockets; echo" 2> /dev/null)"
mapfile -t f <<< "${facts}"
note "runtime ${f[0]:-?}, ${f[1]:-?}, VMM ${f[2]:-?}, image volume ${f[3]:-?}, health ${f[4]:-?}, socket without a token ${f[5]:-?}"
[[ "${f[0]:-}" == krun && "${f[1]:-}" == no-sidecar ]] && ok "krun, and no sidecar" || fail "runtime/sidecar: ${f[0]:-} ${f[1]:-}"
[[ "${f[2]:-}" == container_kvm_t ]] && ok "its VMM is confined (container_kvm_t)" || fail "its VMM's label: ${f[2]:-}"
[[ "${f[3]:-}" == image ]] && ok "the stream image's files: an image volume" || fail "wadsecure-${id}: ${f[3]:-none}"
[[ "${f[4]:-}" == 200 ]] && ok "Selkies' health answers on 127.0.0.1:${port}/ws-${guid}/" || fail "health: ${f[4]:-}"
[[ "${f[5]:-}" != 101 ]] && ok "its socket turns away a viewer without the key (${f[5]:-})" || fail "streamed without a token"
denials="$(as_root 'ausearch -m avc,user_avc -ts boot 2> /dev/null | grep -c "avc:"' 2> /dev/null)"
[[ "${denials:-0}" == 0 ]] && ok "no SELinux denials" || fail "${denials} SELinux denials (ausearch -m avc -ts boot)"

echo "== streamed: its one-tap link, in a browser"
share="$(api GET "/v1/streams/${id}/share")"
key="$(jq -r .openLink <<< "${share}" | sed -n 's/.*#token=//p')"
if [[ -n "${key}" ]]; then ok "its share has a one-tap link"; else fail "share: $(jq -c '{link, mode}' <<< "${share}")"; fi
# The browser on the laptop, at the VM's port (as wadd's proxy hands it on).
lport="$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')"
ssh -i "${ROOT}/.build/vm/id_ed25519" -p "$(cat "${RUN}/ssh.port")" -o BatchMode=yes -o StrictHostKeyChecking=no \
    -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ExitOnForwardFailure=yes \
    -N -L "127.0.0.1:${lport}:127.0.0.1:${port}" root@127.0.0.1 &
tunnel=$!
sleep 2
seen="$(node "${ROOT}/tools/krun/view.mjs" "http://localhost:${lport}/ws-${guid}/#token=${key}" "${OUT}/stream.png" 12 2>&1)"
kill "${tunnel}" 2> /dev/null
wait "${tunnel}" 2> /dev/null
first="$(head -1 <<< "${seen}")"
frames="$(sed -n 3p <<< "${seen}" | grep -o ', [0-9]*' | awk '{s += $2} END {print s + 0}')"
if grep -q '"drawn":true' <<< "${first}" && [[ "${frames:-0}" -gt 50 ]]; then
    ok "the browser draws its desktop (${frames} frames; ${OUT}/stream.png, stream-start.png: its Start menu)"
else
    fail "the browser: ${first}"
    note "${seen}"
fi

echo "== stopped, and nothing left"
t=$(now)
ended="$(api DELETE "/v1/wadspaces/${id}/stream")"
[[ "$(jq -r .stage <<< "${ended}")" == ended ]] && ok "ended" || fail "stop: ${ended}"
for _ in $(seq 60); do as_root "podman container exists wad-${id}" 2> /dev/null || break; sleep 1; done
stop_s=$(($(now) - t))
left="$(as_root "podman container exists wad-${id} && echo 'its container';
    podman volume exists wadsecure-${id} && echo 'its image volume';
    systemctl -q is-active wadsecure-${id}-volume.service && echo 'its image volume unit, active';
    ls ${UNITS}/wadsecure-${id}.volume ${UNITS}/wadstream-${id}.container ${UNITS}/wadstream-${id}.volume 2> /dev/null;
    podman secret ls --format '{{.Name}}' | grep -E '^wad_stream_.*_${id}\$';
    test -e ${STREAMS}/${id} && echo 'its stream folder';
    ps -eo label | grep -q container_kvm_t && echo 'a VM still running';
    journalctl -u wad-${id} --no-pager -n 50 | grep -q SIGKILL && echo 'it was killed, not stopped (SIGKILL)';
    tailscale serve status 2> /dev/null | grep -q ':${port}' && echo 'the tailnet still serves it'" 2> /dev/null)"
if [[ -z "${left}" ]]; then ok "stopped in ${stop_s} s (SIGINT, a clean s6 stop), nothing of it left"; else fail "left: ${left//$'\n'/; }"; fi
run="$(api GET /v1/runs | jq -r --arg id "${id}" '.[] | select(.id == $id) | .phase')"
[[ "${run}" == idle ]] && ok "the wadspace is idle" || fail "the wadspace: ${run}"
if [[ -z "${KEEP}" ]]; then
    api POST /v1/ops "{\"op\": {\"op\": \"ws.trash\", \"ws\": \"${id}\"}}" > /dev/null 2>&1 || true
fi
finish
