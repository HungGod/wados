#!/usr/bin/env bash
# A game streamed from this laptop to a phone on USB, as wadd streams one,
# without wadd: the sample's `retro` wadspace (Dolphin, starting straight
# into Mario Kart: Double Dash!!) on Dolphin's set image, its sidecar with
# player 1's gamepad (a game's own stream, as wadd's), and the disc given
# here as the machine's (`<state>/discs/<id>/<file>`, /config/Games inside).
# The units are wad-gen's (crates/wad-gen/examples/stream-units.rs), run by
# hand from quadlet's dry run, rootless, as images/stream/try-stream.sh runs
# them.
#
# The phone reaches the stream over USB (adb reverse: its 127.0.0.1:<port>
# is this laptop's), and the WadSpaces app opens the one-tap link (a debug
# build's `--es stream`: wadspaces-android's Fake.streamLink). It stays up
# until Enter (or --for SECONDS), then everything it made goes.
#
#   images/stream/try-game.sh DISC.rvz            Dolphin's set must be built:
#                                                 images/build.sh set dolphin
#   images/stream/try-game.sh DISC.rvz --image <ref> --for 1800 --no-phone
#   images/stream/try-game.sh DISC.rvz --public   a public stream's WebRTC (dual
#                                                 mode, the mux port), as wadd's
#                                                 "Stream publicly"; else private
#   images/stream/try-game.sh DISC.rvz --players 2  Play together: player 1 on the
#                                                 phone, the others' links printed
#                                                 (wadd's table: viewers, slots 2+)
#
# Checks, each ok or FAIL: the sidecar is up with gamepads on and its pads
# in the display volume; the wadspace has the disc at /config/Games and its
# game's launcher; Dolphin runs with the disc (not an empty desktop), full
# screen, with the interposer preloaded (the pads). Logs go to
# .build/try-game/.
set -uo pipefail
ulimit -c 0
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "${HERE}/../.." && pwd)"
OUT="${ROOT}/.build/try-game"
ID=trystream
GUID=game
WEB_PORT="${WEB_PORT:-48190}"
SIDECAR="wadstream-${ID}"
WADSPACE="wad-${ID}"
SECRET="wad_stream_master_${ID}"
AT="http://127.0.0.1:${WEB_PORT}"
DISC_ID=mario-kart-double-dash
DISC_FILE=mario-kart-double-dash.rvz

MUX_PORT="${MUX_PORT:-48191}"
DISC="" IMAGE="" FOR="" PHONE=true PUBLIC=false PLAYERS=1
while [[ $# -gt 0 ]]; do
    case "$1" in
        --image) IMAGE="$2"; shift 2 ;;
        --for) FOR="$2"; shift 2 ;;
        --no-phone) PHONE=false; shift ;;
        --public) PUBLIC=true; shift ;;
        --players) PLAYERS="$2"; shift 2 ;;
        -h|--help) sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) DISC="$1"; shift ;;
    esac
done
[[ "${PLAYERS}" =~ ^[1-4]$ ]] || { echo "try-game: --players 1 to 4" >&2; exit 2; }
[[ -f "${DISC}" ]] || { echo "try-game: a disc (.rvz or .iso) to play: try-game.sh DISC.rvz" >&2; exit 2; }
if [[ -z "${IMAGE}" ]]; then
    IMAGE="$(podman images --format '{{.Repository}}:{{.Tag}} {{.Labels}}' | awk '/^localhost\/wados-set-/ && /io.wadspaces.pkgs:dolphin[ \]]/ { print $1; exit }')"
fi
[[ -n "${IMAGE}" ]] && podman image exists "${IMAGE}" || { echo "try-game: Dolphin's set isn't built: images/build.sh set dolphin (or --image)" >&2; exit 2; }
podman image exists localhost/wados-stream:44 || { echo "try-game: needs localhost/wados-stream:44 (images/build.sh --only stream)" >&2; exit 2; }
QUADLET="$(command -v quadlet || ls /usr/libexec/podman/quadlet 2> /dev/null)" || { echo "needs podman's quadlet" >&2; exit 2; }

fails=0
check() { if eval "$2"; then echo "ok    $1"; else echo "FAIL  $1"; fails=$((fails + 1)); fi; }
log() { echo "== $*"; }
status() { curl -s -o /dev/null -m 10 -w '%{http_code}' "$@"; }

rm -rf "${OUT}"
mkdir -p "${OUT}"
# On disk, not /tmp (memory here): it holds a copy of the disc.
mkdir -p "${ROOT}/.build"
WORK="$(mktemp -d "${ROOT}/.build/try-game-work.XXXXXX")"
AUTH="${WORK}/auth"
stop_all() {
    podman rm -f -t 3 "${WADSPACE}" "${SIDECAR}" > /dev/null 2>&1
    podman volume rm -f "${SIDECAR}" "wad-${ID}-config" > /dev/null 2>&1
    podman secret rm "${SECRET}" > /dev/null 2>&1
}
finish() {
    stop_all
    ${PHONE} && adb reverse --remove "tcp:${WEB_PORT}" > /dev/null 2>&1
    ${PHONE} && ${PUBLIC} && adb reverse --remove "tcp:${MUX_PORT}" > /dev/null 2>&1
    rm -rf "${WORK}"
}
trap finish EXIT
# Stopped (Ctrl+C, a kill): the same clean-up, the disc's copy too.
trap 'exit 130' INT TERM HUP
stop_all
mkdir -m 700 "${AUTH}"

log "the disc, as the machine's: ${DISC} (a copy)"
mkdir -p "${WORK}/state/discs/${DISC_ID}"
cp --reflink=auto "${DISC}" "${WORK}/state/discs/${DISC_ID}/${DISC_FILE}" || exit 1

(cd "${ROOT}" && cargo build -q -p wad-gen --example stream-units) || exit 1
public_args=()
${PUBLIC} && public_args=(--mux-port "${MUX_PORT}")
"${ROOT}/target/debug/examples/stream-units" --units "${WORK}/units" --state "${WORK}/state" --guid "${GUID}" \
    --web-port "${WEB_PORT}" --auth "${AUTH}" --wallpaper '#1b1530' --image "${IMAGE}" --players "${PLAYERS}" "${public_args[@]}" || exit 1
# Public: this laptop's loopback stands in for the machine's addresses
# (as try-stream.sh), and the phone reaches the mux port over USB too:
# WebRTC's TCP candidate, 127.0.0.1:<mux>.
${PUBLIC} && sed -i "s|^PublishPort=${MUX_PORT}:|PublishPort=127.0.0.1:${MUX_PORT}:|" "${WORK}/units/${SIDECAR}.container"

# A unit's podman command, from quadlet's dry run (as try-stream.sh).
command_of() {
    QUADLET_UNIT_DIRS="${WORK}/units" "${QUADLET}" -dryrun -user 2> /dev/null |
        awk -v unit="---$1---" '$0 == unit { on = 1; next } /^---/ { on = 0 } on && /^ExecStart=/ { sub(/^ExecStart=/, ""); print; exit }' |
        python3 -I -c '
import shlex, sys
args = shlex.split(sys.stdin.read().replace("%%", "%"))
skip = ("--cidfile", "--cgroups", "--sdnotify")
print("\n".join(a for a in args if not a.startswith(skip)))'
}
# (Arguments after the service go before the image: the last.)
run_unit() {
    local service="$1" cmd
    shift
    mapfile -t cmd < <(command_of "${service}")
    [[ ${#cmd[@]} -gt 2 ]] || { echo "quadlet made no command of ${service}" >&2; exit 1; }
    "${cmd[@]:0:${#cmd[@]}-1}" "$@" "${cmd[-1]}" > /dev/null || { echo "${service} didn't start" >&2; exit 1; }
}
put() { printf '%s\n' "$2" > "${AUTH}/.$1" && chmod 600 "${AUTH}/.$1" && mv "${AUTH}/.$1" "${AUTH}/$1"; }
KEY="$(head -c 24 /dev/urandom | base64 | tr '+/' '-_' | tr -d '=')"
# Player 1 at the controls, as wadd's table for a game's own stream; Play
# together's others as viewers in their own slots (wadd's token_table).
OTHERS=() table="\"${KEY}\": {\"role\": \"controller\", \"slot\": 1}"
for ((n = 2; n <= PLAYERS; n++)); do
    OTHERS+=("$(head -c 24 /dev/urandom | base64 | tr '+/' '-_' | tr -d '=')")
    table+=", \"${OTHERS[-1]}\": {\"role\": \"viewer\", \"slot\": ${n}}"
done
put tokens.json "{${table}}"
put rtc.json '{"iceServers": []}'
head -c 32 /dev/urandom | base64 | podman secret create "${SECRET}" - > /dev/null

log "the sidecar (gamepads on$(${PUBLIC} && echo ", public: WebRTC")), then the wadspace (${IMAGE})"
run_unit "${SIDECAR}-volume.service"
if ${PUBLIC}; then run_unit "${SIDECAR}.service" -e SELKIES_WEBRTC_PUBLIC_IP=127.0.0.1; else run_unit "${SIDECAR}.service"; fi
for _ in $(seq 60); do [[ "$(status "${AT}/ws-${GUID}/api/health")" == 200 ]] && break; sleep 1; done
check "the sidecar answers on ${AT}/ws-${GUID}/" "[[ \"\$(status ${AT}/ws-${GUID}/api/health)\" == 200 ]]"
${PUBLIC} && check "it streams WebRTC" "curl -s -m 5 ${AT}/ws-${GUID}/api/status | grep -q '\"current_mode\": \"webrtc\"'"
check "its gamepads are on" "podman exec ${SIDECAR} printenv SELKIES_GAMEPAD_ENABLED | grep -qx true"
check "every player's page polls its gamepad from the start" "podman exec ${SIDECAR} printenv SELKIES_GAMEPAD_ON_START | grep -qx true"
check "its pads are in the display volume" "podman exec ${SIDECAR} test -e /run/wadspaces-display/interposer/selkies_input_interposer.so"
run_unit "${WADSPACE}.service"
# (Not grep -q: with pipefail, podman logs cut off by it fails the line.)
drawn() { podman logs "${WADSPACE}" 2>&1 | grep 'svc-de. drawing on ' > /dev/null; }
for _ in $(seq 180); do drawn && break; sleep 1; done
check "its desktop draws on the stream" drawn
check "the disc is at /config/Games" "podman exec ${WADSPACE} test -s /config/Games/${DISC_ID}/${DISC_FILE}"
check "its game has a launcher" "podman exec ${WADSPACE} grep -q 'Games/${DISC_ID}' /usr/share/applications/wadspaces-game-${DISC_ID}.desktop"
# Fedora's dolphin-emu runs dolphin-emu-x11: that's the emulator.
emu() { podman exec --user abc "${WADSPACE}" pgrep -f "dolphin-emu-x11.*${DISC_FILE}" 2> /dev/null | head -1; }
for _ in $(seq 60); do [[ -n "$(emu)" ]] && break; sleep 1; done
pid="$(emu)"
check "Dolphin runs the disc, straight away" "[[ -n '${pid}' ]]"
check "Dolphin has the interposer preloaded (the pads)" "podman exec --user abc ${WADSPACE} grep -q selkies_input_interposer /proc/${pid}/maps"
check "Dolphin plays full screen, and takes the pads unfocused" \
    "podman exec --user abc ${WADSPACE} grep -qx 'Fullscreen = True' /config/.config/dolphin-emu/Dolphin.ini && podman exec --user abc ${WADSPACE} grep -qx 'BackgroundInput = True' /config/.config/dolphin-emu/Dolphin.ini"
check "the page opens the touch gamepad on a phone" "curl -sf ${AT}/ws-${GUID}/ | grep wados-touch-pad.js > /dev/null"

LINK="${AT}/ws-${GUID}/#token=${KEY}"
if ${PHONE}; then
    if adb get-state > /dev/null 2>&1; then
        adb reverse "tcp:${WEB_PORT}" "tcp:${WEB_PORT}" > /dev/null
        ${PUBLIC} && adb reverse "tcp:${MUX_PORT}" "tcp:${MUX_PORT}" > /dev/null
        log "the phone ($(adb shell getprop ro.product.model | tr -d '\r')) opens the stream in WadSpaces"
        # A stream screen left open would take the link with its old key: closed first.
        adb shell am force-stop io.wadspaces.android
        adb shell "am start -n io.wadspaces.android/.MainActivity --es stream '${LINK}' --es title 'Mario Kart' --ez sideways true" > /dev/null \
            || echo "the WadSpaces app (a debug build) isn't on the phone: open ${LINK} in it yourself"
    else
        echo "no phone on adb: open ${LINK} yourself"
    fi
fi
echo
echo "   the stream: ${LINK}"
echo "   (on the phone over USB, the same address: adb reverse tcp:${WEB_PORT})"
for ((n = 2; n <= PLAYERS; n++)); do
    echo "   player ${n}: ${AT}/ws-${GUID}/#token=${OTHERS[n - 2]}"
done
echo
((fails)) && echo "try-game: ${fails} failed" || echo "try-game: all ok"
podman logs "${WADSPACE}" > "${OUT}/wadspace.log" 2>&1
if [[ -n "${FOR}" ]]; then
    echo "up for ${FOR} s"
    # (In the background, so a stop isn't held until it ends.)
    sleep "${FOR}" &
    wait $!
else
    read -r -p "Enter to stop it… " _
fi
podman logs "${WADSPACE}" > "${OUT}/wadspace.log" 2>&1
podman logs "${SIDECAR}" > "${OUT}/sidecar.log" 2>&1
exit $((fails > 0))
