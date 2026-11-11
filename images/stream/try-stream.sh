#!/usr/bin/env bash
# Streams a wadspace on this laptop, as wadd will, without wadd: the units
# wad-gen writes for it (crates/wad-gen/examples/stream-units.rs: the
# wadspace on localhost/wados-base:44, its sidecar on
# localhost/wados-stream:44 and the display volume they share), turned into
# podman commands by quadlet's own dry run and run by hand, rootless. Then
# the streaming socket is tried with and without its key (ws-socket.py), and
# a browser viewer, when there is one (WADOS_STREAM_VIEWER), opens the stream
# at http://127.0.0.1:<port>/ws-test/, as wadd's proxy would hand it on.
#
#   images/stream/try-stream.sh                all of it
#   images/stream/try-stream.sh --private      no public stream (WebRTC)
#   images/stream/try-stream.sh --no-move      no move from a screen
#   images/stream/try-stream.sh --move-only    only the move
#
# Checks, each ok or FAIL (exits 1 if any failed):
#   - the sidecar serves on 127.0.0.1 only, under /ws-test/ alone (any other
#     path, and the path without its slash, isn't Selkies' page), and its
#     health and status answer there without a token;
#   - it took wadd's token table (secure mode), and the wadspace's labwc
#     draws on its compositor, through the display volume they share, with
#     the same SELinux categories; the wadspace's sound plays into its
#     PulseAudio;
#   - private (WebSockets): with the stream's key (#token=, as a one-tap
#     link) the page streams the wadspace's desktop; with no token, or a
#     wrong one, it loads and nothing streams; the streaming socket turns
#     both away; a key taken out of the table is cut off, and refused after;
#   - public (WebRTC with dual mode, the mux port): the key streams over
#     WebRTC, no token streams nothing, and switching to WebSockets (dual
#     mode, the key's) streams over WebSockets;
#   - a move (moving_units): a wadspace running on a screen (a headless
#     labwc here, the machine's on WadOS) moves to its stream as wadd moves
#     it (the display file, then wadspaces-session switch), streams, moves
#     back, and keeps running when its sidecar stops; a terminal opened on
#     the screen comes back after each move with its tmux session.
#
# What differs from a machine's units, all for this laptop: the mux port is
# published on 127.0.0.1 and Selkies advertises 127.0.0.1 as its host
# candidates (SELKIES_WEBRTC_PUBLIC_IP), so WebRTC stays on loopback; the
# ICE servers are none (rtc.json's iceServers is empty: no STUN or TURN here;
# Cloudflare's are phase 7's public test); and for the move, the wadspace
# runs without SELinux's labels (as images/check.sh does: the headless
# labwc's socket is this session's). Screenshots and what the viewer saw go
# to .build/try-stream/. Everything it makes is removed.
#
# Needs: the images (images/build.sh), cargo, podman's quadlet and python3.
# WADOS_STREAM_VIEWER (optional): a command that takes URL OUT.png [--expect
# stream|none] [--wait SECONDS] --colour '#rrggbb', saves what it saw, prints
# JSON lines with the page's status and each socket's frames
# ({"url":".../api/websockets","frames":N}), and exits 0 when it saw what was
# expected. Without one, the browser's checks are skipped (the Node and
# Playwright one went with the v2 spike);
# for the move, the patched labwc (tools/build-labwc.sh) and wad-wayland's
# examples (built here when they're missing).
set -uo pipefail
ulimit -c 0
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "${HERE}/../.." && pwd)"
OUT="${ROOT}/.build/try-stream"
ID=trystream
GUID=test
WEB_PORT="${WEB_PORT:-48190}"
MUX_PORT="${MUX_PORT:-48291}"
WALLPAPER='#2e86de'
SIDECAR="wadstream-${ID}"
WADSPACE="wad-${ID}"
SECRET="wad_stream_master_${ID}"
AT="http://127.0.0.1:${WEB_PORT}"
PRIVATE=true
PUBLIC=true
MOVE=true
for a in "$@"; do
    case "${a}" in
        --private) PUBLIC=false ;;
        --no-move) MOVE=false ;;
        --move-only) PRIVATE=false; PUBLIC=false ;;
        *) sed -n '2,14p' "$0" | sed 's/^# \{0,1\}//'; exit 2 ;;
    esac
done

fails=0
check() { if eval "$2"; then echo "ok    $1"; else echo "FAIL  $1"; fails=$((fails + 1)); fi; }
log() { echo "== $*"; }
status() { curl -s -o /dev/null -m 10 -w '%{http_code}' "$@"; }
# A browser viewer's look (WADOS_STREAM_VIEWER), or nothing without one.
probe() {
    [[ -n "${WADOS_STREAM_VIEWER:-}" ]] || return 0
    (cd "${OUT}" && timeout 150 ${WADOS_STREAM_VIEWER} "$@" --colour "${WALLPAPER}")
}
# A check on what the viewer saw: skipped without one.
vcheck() {
    if [[ -z "${WADOS_STREAM_VIEWER:-}" ]]; then echo "skip  $1 (no browser viewer: WADOS_STREAM_VIEWER)"; return; fi
    check "$@"
}
# What the streaming socket does for a client holding a token (?token=), for
# `seconds`: `messages=<how many it sent> closed=<its close code, or none>`.
# Selkies upgrades any handshake that carries a token, then closes it (4001)
# when the token isn't in its table.
socket() { python3 -I "${HERE}/ws-socket.py" "$1" "${2:-4}"; }

for img in localhost/wados-base:44 localhost/wados-stream:44; do
    podman image exists "${img}" || { echo "needs ${img} (images/build.sh)" >&2; exit 1; }
done
QUADLET="$(command -v quadlet || ls /usr/libexec/podman/quadlet 2> /dev/null)" || { echo "needs podman's quadlet" >&2; exit 1; }
command -v python3 > /dev/null || { echo "needs python3" >&2; exit 1; }

rm -rf "${OUT}"
mkdir -p "${OUT}"
WORK="$(mktemp -d)"
AUTH="${WORK}/auth"
lpid=""
stop_all() {
    podman rm -f -t 3 "${WADSPACE}" "${SIDECAR}" > /dev/null 2>&1
    podman volume rm -f "${SIDECAR}" "wad-${ID}-config" > /dev/null 2>&1
    podman secret rm "${SECRET}" > /dev/null 2>&1
}
finish() {
    stop_all
    [[ -n "${lpid}" ]] && kill "${lpid}" 2> /dev/null
    declare -F stop_labwc > /dev/null && stop_labwc
    rm -rf "${WORK}"
}
trap finish EXIT
stop_all
mkdir -m 700 "${AUTH}"

(cd "${ROOT}" && cargo build -q -p wad-gen --example stream-units) || exit 1
# wad-gen's units for it, into WORK/<dir>.
units() {
    local dir="$1"
    shift
    "${ROOT}/target/debug/examples/stream-units" --units "${WORK}/${dir}" --state "${WORK}/state" --guid "${GUID}" \
        --web-port "${WEB_PORT}" --auth "${AUTH}" --wallpaper "${WALLPAPER}" "$@"
}
# What wadd writes in its extra folder before it starts or moves (units::DISPLAY_FILE).
display() { printf '%s\n' "$1" > "${WORK}/state/extra/${ID}/display"; }

# A unit's podman command, from quadlet's dry run: what systemd would run
# (detached, --replace, --rm), less what only systemd can give it (the cid
# file, conmon's notify). One argument a line, for mapfile.
command_of() {
    QUADLET_UNIT_DIRS="${WORK}/$1" "${QUADLET}" -dryrun -user 2> /dev/null |
        awk -v unit="---$2---" '$0 == unit { on = 1; next } /^---/ { on = 0 } on && /^ExecStart=/ { sub(/^ExecStart=/, ""); print; exit }' |
        python3 -I -c '
import shlex, sys
args = shlex.split(sys.stdin.read().replace("%%", "%"))
skip = ("--cidfile", "--cgroups", "--sdnotify")
print("\n".join(a for a in args if not a.startswith(skip)))'
}
# Runs a unit of WORK/<dir> (a service quadlet made of it), with what `$@`
# adds before its image.
run_unit() {
    local dir="$1" service="$2" cmd
    shift 2
    mapfile -t cmd < <(command_of "${dir}" "${service}")
    [[ ${#cmd[@]} -gt 2 ]] || { echo "quadlet made no command of ${service} in ${WORK}/${dir}" >&2; exit 1; }
    "${cmd[@]:0:${#cmd[@]}-1}" "$@" "${cmd[-1]}" > /dev/null || { echo "${service} didn't start" >&2; exit 1; }
}

# wadd's folder for the stream: its token table (the stream's key, a
# controller) and ICE servers, the machine's user's, 0600, each written then
# renamed into place.
put() { printf '%s\n' "$2" > "${AUTH}/.$1" && chmod 600 "${AUTH}/.$1" && mv "${AUTH}/.$1" "${AUTH}/$1"; }
KEY="$(head -c 24 /dev/urandom | base64 | tr '+/' '-_' | tr -d '=')"
put tokens.json "{\"${KEY}\": {\"role\": \"controller\", \"slot\": null}}"
put rtc.json '{"iceServers": []}'
head -c 32 /dev/urandom | base64 | podman secret create "${SECRET}" - > /dev/null

# The wadspace's desktop's starts so far: how many, and the last one's display.
drawn() { podman logs "${WADSPACE}" 2>&1 | grep -c '^\[svc-de\] drawing on '; }
last_drawn() { podman logs "${WADSPACE}" 2>&1 | grep '^\[svc-de\] drawing on ' | tail -n 1; }
# Waits for its desktop to start again (after `before` starts).
redrawn() { for _ in $(seq 90); do [[ "$(drawn)" -gt "$1" ]] && return 0; sleep 1; done; return 1; }
healthy() { for _ in $(seq 60); do [[ "$(status "${AT}/ws-${GUID}/api/health")" == 200 ]] && return 0; sleep 1; done; return 1; }

# Starts the units in WORK/<dir>: the display volume, the sidecar (with what
# `$@` adds), then the wadspace (or, running already, waits for its desktop
# to come back on the new sidecar).
start() {
    local dir="$1" before=0
    shift
    # (Counted first: its desktop may come back before Selkies' page does.)
    podman container exists "${WADSPACE}" && before="$(drawn)"
    run_unit "${dir}" "${SIDECAR}-volume.service"
    run_unit "${dir}" "${SIDECAR}.service" "$@"
    healthy
    podman container exists "${WADSPACE}" || run_unit "${dir}" "${WADSPACE}.service"
    redrawn "${before}"
}

# ------------------------------------------------------------- private
if ${PRIVATE}; then
log "a private stream (WebSockets): units, then the sidecar and the wadspace"
units private || exit 1
start private
echo "   $(grep -E '^(Environment=SELKIES_MODE|Secret=|Volume=.*selkies|PublishPort)' "${WORK}/private/${SIDECAR}.container" | tr '\n' ' ')"
web="$(podman port "${SIDECAR}" 3000/tcp)"
check "the sidecar's web port is on 127.0.0.1 only (${web})" "[[ \"\${web}\" == 127.0.0.1:${WEB_PORT} ]]"
slog="$(podman logs "${SIDECAR}" 2>&1)"
grep -E '^(\[stream-display\]|INFO:main|INFO:ws:Updated)' <<< "${slog}" | sed 's/^/   /'
check "secure mode: Selkies took wadd's token table" "grep -q 'Now tracking 1 tokens' <<< \"\${slog}\""
wlog="$(last_drawn)"
echo "   ${wlog}"
check "the wadspace's labwc draws on the sidecar's compositor (the shared volume, at /run/wadspaces-stream)" \
    "grep -q 'drawing on /run/wadspaces-stream/wayland-' <<< \"\${wlog}\""
levels="$(podman inspect --format '{{.ProcessLabel}}' "${SIDECAR}" "${WADSPACE}" | sed 's/.*:s0/s0/' | sort -u)"
check "both run with the wadspace's SELinux categories (${levels//$'\n'/ })" "[[ \$(wc -l <<< \"\${levels}\") == 1 && \"\${levels}\" == s0:c* ]]"

podman exec -u abc "${WADSPACE}" sh -c 'timeout 15 gst-launch-1.0 -q audiotestsrc num-buffers=500 ! audioconvert ! pulsesink client-name=try-stream' > /dev/null 2>&1 &
tone=$!
sleep 3
sound="$(podman exec -u abc -e PULSE_SERVER=unix:/run/wadspaces-display/pulse/native "${SIDECAR}" pactl list sink-inputs 2>&1)"
wait "${tone}" 2> /dev/null
check "the wadspace's sound plays into the sidecar's PulseAudio (through /run/wadspaces-current)" \
    "grep -q 'application.name = \"try-stream\"' <<< \"\${sound}\""

log "its routes (no token)"
declare -A want=([/ws-${GUID}/]=200 [/ws-${GUID}/api/health]=200 [/ws-${GUID}/api/status]=200
    [/]=404 [/api/health]=404 [/ws-other/]=404 [/ws-${GUID}x/]=404)
for path in "${!want[@]}"; do
    got="$(status "${AT}${path}")"
    check "${path} answers ${want[${path}]} (${got})" "[[ ${got} == ${want[${path}]} ]]"
done
got="$(status "${AT}/ws-${GUID}")"
check "/ws-${GUID} without its slash isn't the page (${got}; wadd's proxy redirects it)" "[[ ${got} != 200 ]]"
echo "   status: $(curl -s -m 5 "${AT}/ws-${GUID}/api/status")"

log "the streaming socket"
none="$(status "${AT}/ws-${GUID}/api/websockets" --http1.1 -H 'Connection: Upgrade' -H 'Upgrade: websocket' \
    -H 'Sec-WebSocket-Version: 13' -H 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==')"
wrong="$(socket "${AT/http/ws}/ws-${GUID}/api/websockets?token=not-the-key")"
right="$(socket "${AT/http/ws}/ws-${GUID}/api/websockets?token=${KEY}")"
check "no token: the handshake is refused (${none})" "[[ ${none} == 401 ]]"
check "a wrong token: closed at once, nothing sent (${wrong})" "[[ '${wrong}' == 'messages=0 closed=4001' ]]"
check "the stream's key: it streams (${right})" "[[ '${right}' =~ ^messages=[1-9][0-9]*\ closed=none$ ]]"

log "a viewer (a browser: WADOS_STREAM_VIEWER)"
probe "${AT}/ws-${GUID}/#token=${KEY}" "${OUT}/private-key.png" --expect stream > "${OUT}/private-key.txt"
ok=$?
sed 's/^/   /' "${OUT}/private-key.txt"
vcheck "with the key (#token=) the page streams the wadspace's desktop (private-key.png)" \
    "[[ ${ok} == 0 ]] && grep -q '/api/websockets\",\"frames\":[1-9]' '${OUT}/private-key.txt'"
probe "${AT}/ws-${GUID}/" "${OUT}/private-no-token.png" --expect none --wait 20 > "${OUT}/private-no-token.txt"
ok=$?
vcheck "with no token the page loads and nothing streams (private-no-token.png)" "[[ ${ok} == 0 ]]"
probe "${AT}/ws-${GUID}/#token=not-the-key" "${OUT}/private-wrong-token.png" --expect none --wait 20 > "${OUT}/private-wrong-token.txt"
ok=$?
vcheck "with a wrong token nothing streams (private-wrong-token.png)" "[[ ${ok} == 0 ]]"

log "the key taken out of the table (wadd rewrites it)"
socket "${AT/http/ws}/ws-${GUID}/api/websockets?token=${KEY}" 15 > "${WORK}/cut" &
watcher=$!
sleep 4
put tokens.json '{"another-key": {"role": "controller", "slot": null}}'
wait "${watcher}"
cut="$(cat "${WORK}/cut")"
check "a viewer holding it is cut off (${cut})" "[[ '${cut}' =~ ^messages=[1-9][0-9]*\ closed=[0-9]+$ ]]"
after="$(socket "${AT/http/ws}/ws-${GUID}/api/websockets?token=${KEY}")"
check "and it's refused after (${after})" "[[ '${after}' == 'messages=0 closed=4001' ]]"
put tokens.json "{\"${KEY}\": {\"role\": \"controller\", \"slot\": null}}"
fi

# ------------------------------------------------------------- public
if ${PUBLIC}; then
    log "a public stream (WebRTC, dual mode, mux port ${MUX_PORT}): the sidecar again, the wadspace stays"
    units public --mux-port "${MUX_PORT}" || exit 1
    podman rm -f -t 3 "${SIDECAR}" > /dev/null
    # This laptop's loopback stands in for the machine's addresses (see the top).
    sed -i "s|^PublishPort=${MUX_PORT}:|PublishPort=127.0.0.1:${MUX_PORT}:|" "${WORK}/public/${SIDECAR}.container"
    start public -e SELKIES_WEBRTC_PUBLIC_IP=127.0.0.1
    echo "   $(grep -E '^(Environment=SELKIES_(MODE|ENABLE_DUAL|RTC|WEBRTC)|PublishPort)' "${WORK}/public/${SIDECAR}.container" | tr '\n' ' ')"
    st="$(curl -s -m 5 "${AT}/ws-${GUID}/api/status")"
    check "Selkies runs WebRTC with dual mode (${st})" \
        "grep -q '\"current_mode\": \"webrtc\"' <<< \"\${st}\" && grep -q '\"enable_dual_mode\": true' <<< \"\${st}\""
    slog="$(podman logs "${SIDECAR}" 2>&1)"
    check "it reads wadd's ICE servers (rtc.json)" "grep -q \"Using JSON file '/run/wadspaces-selkies/rtc.json'\" <<< \"\${slog}\""
    wlog="$(last_drawn)"
    check "the wadspace came back on the new sidecar's compositor by itself" \
        "grep -q 'drawing on /run/wadspaces-stream/wayland-' <<< \"\${wlog}\""
    probe "${AT}/ws-${GUID}/#token=${KEY}" "${OUT}/public-key.png" --expect stream > "${OUT}/public-key.txt"
    ok=$?
    sed 's/^/   /' "${OUT}/public-key.txt"
    vcheck "with the key the page streams over WebRTC, no data socket (public-key.png)" \
        "[[ ${ok} == 0 ]] && grep -q '/api/webrtc/signaling' '${OUT}/public-key.txt' && ! grep -q '/api/websockets' '${OUT}/public-key.txt'"
    probe "${AT}/ws-${GUID}/" "${OUT}/public-no-token.png" --expect none --wait 20 > "${OUT}/public-no-token.txt"
    ok=$?
    vcheck "with no token nothing streams (public-no-token.png)" "[[ ${ok} == 0 ]]"
    sw="$(curl -s -m 10 -X POST -H "Authorization: Bearer ${KEY}" -H 'Content-Type: application/json' \
        -d '{"mode": "websockets"}' "${AT}/ws-${GUID}/api/switch")"
    nosw="$(status -X POST -H 'Content-Type: application/json' -d '{"mode": "websockets"}' "${AT}/ws-${GUID}/api/switch")"
    check "dual mode: the key switches it to WebSockets (${sw}); no token can't (${nosw})" \
        "grep -q '\"status\": \"success\"' <<< \"\${sw}\" && [[ ${nosw} == 401 ]]"
    probe "${AT}/ws-${GUID}/#token=${KEY}" "${OUT}/public-websockets.png" --expect stream > "${OUT}/public-websockets.txt"
    ok=$?
    sed 's/^/   /' "${OUT}/public-websockets.txt"
    vcheck "then the page streams over WebSockets (public-websockets.png)" \
        "[[ ${ok} == 0 ]] && grep -q '/api/websockets\",\"frames\":[1-9]' '${OUT}/public-websockets.txt'"
fi

# ------------------------------------------------------------- a move
if ${MOVE}; then
    log "a move: the wadspace on a screen (a headless labwc), then to its stream and back"
    stop_all
    head -c 32 /dev/urandom | base64 | podman secret create "${SECRET}" - > /dev/null
    ex="${ROOT}/target/debug/examples"
    if [[ ! -x "${ex}/listen" || ! -x "${ex}/toplevels" ]]; then
        (cd "${ROOT}" && cargo build -q -p wad-wayland --examples) || exit 1
    fi
    HL_NAME=try-stream
    # shellcheck source=../../tools/headless.sh
    . "${ROOT}/tools/headless.sh"
    start_labwc 1 || exit 1
    # The machine's screen folder for it, as wadd makes it: one socket, its own security context.
    mkdir -p "${WORK}/screens/${ID}"
    "${ex}/listen" "${hl_rt}/wayland-0" "${WORK}/screens/${ID}/wayland-1" "io.wadspaces.${ID}" > /dev/null 2>&1 &
    lpid=$!
    for _ in $(seq 50); do [[ -S "${WORK}/screens/${ID}/wayland-1" ]] && break; sleep 0.1; done
    units moving-screen --moving --no-stream --screen-dir "${WORK}/screens" || exit 1
    units moving-stream --moving --screen-dir "${WORK}/screens" || exit 1
    display screen
    run_unit moving-screen "${SIDECAR}-volume.service"
    run_unit moving-screen "${WADSPACE}.service" --security-opt label=disable
    redrawn 0
    tops="$(in_labwc "${ex}/toplevels" 2> /dev/null)"
    check "on the screen: it draws on the machine's labwc ($(last_drawn | sed 's/.*drawing on //'))" \
        "grep -q 'drawing on /run/wadspaces-display/wayland-1' <<< \"\$(last_drawn)\" && grep -q 'app_id=\"io.wadspaces.${ID}\"' <<< \"\${tops}\""
    grep -E '^(Volume=.*/run/wadspaces-(display|stream)|Environment=PULSE|BindsTo)' "${WORK}/moving-screen/${WADSPACE}.container" | sed 's/^/   /'
    # Something open on it, to see it come back after the move: a terminal,
    # whose tmux session (and its job) outlives the move.
    podman exec -u abc "${WADSPACE}" sh -c 'cd /config && XDG_RUNTIME_DIR=/config/.XDG WAYLAND_DISPLAY=$(basename $(ls -t /config/.XDG/wayland-[0-9] | head -n 1)) setsid wadspaces-term > /dev/null 2>&1 &'
    sleep 3

    # To the stream: the sidecar, the display file, then switch.
    before="$(drawn)"
    run_unit moving-stream "${SIDECAR}.service"
    healthy
    display stream
    sw="$(podman exec -u abc "${WADSPACE}" wadspaces-session switch --timeout 20 2>&1)"
    redrawn "${before}"
    echo "   switch: ${sw}"
    check "moved to its stream: it draws on the sidecar's compositor ($(last_drawn | sed 's/.*drawing on //'))" \
        "grep -q 'drawing on /run/wadspaces-stream/wayland-' <<< \"\$(last_drawn)\""
    # (Its terminal, back and maximized, covers the wallpaper: frames, not colour.)
    probe "${AT}/ws-${GUID}/#token=${KEY}" "${OUT}/moved-stream.png" --wait 15 > "${OUT}/moved-stream.txt"
    sed 's/^/   /' "${OUT}/moved-stream.txt"
    vcheck "and the page streams it (moved-stream.png)" "grep -q '\"page\":200.*/api/websockets\",\"frames\":[1-9][0-9]' '${OUT}/moved-stream.txt'"
    sessions="$(podman exec -u abc "${WADSPACE}" sh -c 'tmux list-sessions 2>&1; pgrep -a foot' 2>&1)"
    echo "${sessions}" | sed 's/^/   /'
    check "its terminal came back on the stream, in the tmux session it had" \
        "grep -q '^term-.*(attached)' <<< \"\${sessions}\" && grep -q 'foot -e tmux attach-session -t term-' <<< \"\${sessions}\""

    # Back to the screen, then the sidecar stops; the wadspace carries on.
    before="$(drawn)"
    display screen
    podman exec -u abc "${WADSPACE}" wadspaces-session switch --timeout 20 > /dev/null 2>&1
    redrawn "${before}"
    podman rm -f -t 3 "${SIDECAR}" > /dev/null
    sleep 3
    tops="$(in_labwc "${ex}/toplevels" 2> /dev/null)"
    check "moved back: it draws on the machine's labwc again" \
        "grep -q 'drawing on /run/wadspaces-display/wayland-1' <<< \"\$(last_drawn)\" && grep -q 'app_id=\"io.wadspaces.${ID}\"' <<< \"\${tops}\""
    check "and runs on with its sidecar gone (not bound to it)" \
        "[[ \"\$(podman inspect --format '{{.State.Running}}' ${WADSPACE} 2> /dev/null)\" == true ]]"
    sessions="$(podman exec -u abc "${WADSPACE}" sh -c 'tmux list-sessions 2>&1; pgrep -a foot' 2>&1)"
    check "its terminal came back on the screen too, in the same session" \
        "grep -q '^term-.*(attached)' <<< \"\${sessions}\" && [[ \$(grep -c '^term-' <<< \"\${sessions}\") == 1 ]]"
    mkdir -p "${OUT}" && shot HEADLESS-1 "${OUT}/moved-back.png" 2> /dev/null && echo "   screenshot: ${OUT}/moved-back.png"
fi

if [[ ${fails} -gt 0 ]]; then
    echo "-- the sidecar's log"
    podman logs "${SIDECAR}" 2>&1 | tail -n 30
    echo "-- the wadspace's log"
    podman logs "${WADSPACE}" 2>&1 | grep -E '^\[(svc-de|wadspaces)' | tail -n 15
fi
echo "screenshots: ${OUT}"
[[ ${fails} -eq 0 ]] && echo "all ok" || echo "${fails} failed"
exit $((fails > 0))
