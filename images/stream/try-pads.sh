#!/usr/bin/env bash
# Play together's four pads, without wadd or a game: the sidecar with a token
# per player, a "wadspace" that reads /dev/input/js0-3 through the Input
# Interposer from the display volume, and four players on Selkies' socket.
# Each player's button must arrive on that player's pad, and player 2 must
# not reach player 1's. Everything it makes is removed.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
IMAGE=localhost/wados-stream:44
GUID=pads
VOL=wadpads-display
SIDE=wadpads-side
GAME=wadpads-game
DIR="$(mktemp -d)"
KEYS=(aaaaaaaaaaaaaaaa1 bbbbbbbbbbbbbbbb2 cccccccccccccccc3 dddddddddddddddd4)
LEVEL=s0:c111,c222
FAILED=0
ok() { printf 'ok    %s\n' "$*"; }
fail() { printf 'FAIL  %s\n' "$*"; FAILED=$((FAILED + 1)); }
done_() {
    podman rm -f -t 1 "${GAME}" "${SIDE}" > /dev/null 2>&1
    podman volume rm -f "${VOL}" > /dev/null 2>&1
    rm -rf "${DIR:?}"
}
trap done_ EXIT
done_ 2> /dev/null
DIR="$(mktemp -d)"
cat > "${DIR}/tokens.json" << JSON
{"${KEYS[0]}": {"role": "controller", "slot": 1},
 "${KEYS[1]}": {"role": "viewer", "slot": 2},
 "${KEYS[2]}": {"role": "viewer", "slot": 3},
 "${KEYS[3]}": {"role": "viewer", "slot": 4}}
JSON
chmod 755 "${DIR}"
chmod 644 "${DIR}/tokens.json"
podman volume create "${VOL}" > /dev/null
podman run -d --name "${SIDE}" --security-opt "label=level:${LEVEL}" \
    -v "${VOL}:/run/wadspaces-display:z" -v "${DIR}:/run/wadspaces-selkies:ro,z" \
    -e PUID=1000 -e PGID=1000 -e SELKIES_MASTER_TOKEN=pads-master-token \
    -e SELKIES_TOKENS_FILE=/run/wadspaces-selkies/tokens.json -e "SELKIES_SUBFOLDER=/ws-${GUID}" \
    -e SELKIES_MODE=websockets -e SELKIES_ENABLE_DUAL_MODE=false -e SELKIES_AUTO_GPU=false -e SELKIES_USE_CPU=true \
    -e SELKIES_GAMEPAD_ENABLED=true -e SELKIES_UINPUT_GAMEPAD=false \
    "${IMAGE}" > /dev/null
for _ in $(seq 60); do
    podman exec "${SIDE}" curl -sf "http://127.0.0.1:3000/ws-${GUID}/api/health" > /dev/null 2>&1 && break
    sleep 1
done
podman exec "${SIDE}" curl -sf "http://127.0.0.1:3000/ws-${GUID}/api/health" > /dev/null \
    && ok "the sidecar is up with gamepads on" || { fail "the sidecar didn't come up"; podman logs "${SIDE}" | tail -20; exit 1; }
podman exec "${SIDE}" test -e /run/wadspaces-display/interposer/selkies_input_interposer.so \
    && ok "the interposer is in the display volume" || fail "no interposer in the volume"
L=/run/wadspaces-stream/interposer
podman run -d --name "${GAME}" --security-opt "label=level:${LEVEL}" --user 1000:1000 --entrypoint /opt/selkies/bin/python \
    -v "${VOL}:/run/wadspaces-stream:z" -v "${HERE}/reader.py:/tmp/reader.py:ro,z" --tmpfs /dev/input:rw,mode=1777 \
    -e SELKIES_JS_SOCKET_PATH=/run/wadspaces-stream -e "SELKIES_INTERPOSER=${L}/selkies_input_interposer.so" \
    -e "LD_PRELOAD=${L}/selkies_input_interposer.so:${L}/libudev.so.1" \
    "${IMAGE}" -u /tmp/reader.py > /dev/null
for _ in $(seq 20); do podman logs "${GAME}" 2>&1 | grep -q '^open$' && break; sleep 1; done
podman logs "${GAME}" 2>&1 | grep -q '^open$' && ok "the game opened all four pads" || fail "the game didn't open them: $(podman logs "${GAME}" 2>&1 | tail -3)"
podman cp "${HERE}/driver.py" "${SIDE}:/tmp/driver.py"
podman exec "${SIDE}" /opt/selkies/bin/python /tmp/driver.py "${GUID}" "${KEYS[@]}" 2>&1 | tail -3
sleep 1
got="$(podman logs "${GAME}" 2>&1)"
echo "${got}" | sed 's/^/      | /'
for slot in 1 2 3 4; do
    pad=$((slot - 1))
    grep -qx "pad ${pad} button ${slot}" <<< "${got}" && ok "player ${slot}'s button reached pad ${pad}" || fail "player ${slot}'s button didn't arrive"
done
grep -qx 'pad 0 button 9' <<< "${got}" && fail "player 2 pressed player 1's pad" || ok "player 2 can't press player 1's pad"
echo
((FAILED)) && { echo "try-pads: ${FAILED} failed"; podman logs "${SIDE}" 2>&1 | grep -iE 'gamepad|interposer|token' | tail -12 | cut -c1-160; exit 1; }
echo "try-pads: all ok"
