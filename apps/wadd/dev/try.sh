#!/usr/bin/env bash
# wadd for real, rootless, on a headless labwc: a wadspace built and opened
# on the screen, as the shell would have it, then stopped.
#
#   apps/wadd/dev/try.sh             the terminal-only wadspace (the base as it
#                                    is), then a set's cold build and rebuild
#   apps/wadd/dev/try.sh --no-set    only the terminal-only one
#   TRY_TOOL=python …                the tool the set has (one not built yet
#                                    makes the cold build cold)
#
# 1. A headless patched labwc (tools/headless.sh, tools/build-labwc.sh).
# 2. `wadd serve --user` with a state, socket and screen folder of its own
#    (a short temporary folder: sockets' paths are short), its units in your
#    user manager's quadlet folder ($XDG_RUNTIME_DIR/containers/systemd).
# 3. The headless labwc as the screen (PUT /v1/screen/session, as the shell does).
# 4. A wadspace with only the terminal (POST /v1/ops): built (nothing to
#    build: it runs the base), opened on the screen, ready when its desktop
#    is, its labwc's window listed with app id io.wadspaces.<id>
#    (crates/wad-wayland/examples/toplevels), its clipboard bridged; stopped.
# 5. A wadspace with the terminal and $TRY_TOOL: a cold build (its package
#    built here), an edit, the rebuild from the cached package, and a
#    wadspace with the same set (no build at all); opened, and stopped.
#
# Needs: podman (`systemctl --user start podman.socket`, run here), the base
# (`images/build.sh`), curl, jq. Everything it started is stopped and
# removed afterwards (containers, units, homes, sets); packages and chain
# images stay, as a machine keeps them.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
SET=true
[[ "${1:-}" == --no-set ]] && SET=false
TOOL="${TRY_TOOL:-python}"
BASE=localhost/wados-base:44
TRY="$(mktemp -d /tmp/wadd-try.XXXXXX)"
UNITS="${XDG_RUNTIME_DIR}/containers/systemd"
say() { printf '\n== %s\n' "$*"; }
ok() { printf 'ok    %s\n' "$*"; }
fail() { printf 'FAIL  %s\n' "$*"; FAILED=$((FAILED + 1)); }
FAILED=0
IDS=()

podman image exists "${BASE}" || { echo "${BASE} isn't built: images/build.sh" >&2; exit 1; }
systemctl --user start podman.socket
(cd "${ROOT}" && cargo build -q -p wadd -p wad-clip && cargo build -q -p wad-wayland --examples)
[[ -x "${ROOT}/.build/labwc/labwc" ]] || "${ROOT}/tools/build-labwc.sh" > /dev/null
WADD="${ROOT}/target/debug/wadd"
TOPLEVELS="${ROOT}/target/debug/examples/toplevels"

# shellcheck source=../../../tools/headless.sh
. "${ROOT}/tools/headless.sh"

api() {
    local method="$1" path="$2" body="${3:-}"
    if [[ -n "${body}" ]]; then
        curl -sS --unix-socket "${TRY}/wadd.sock" -X "${method}" -H 'content-type: application/json' -d "${body}" "http://wadd${path}"
    else
        curl -sS --unix-socket "${TRY}/wadd.sock" -X "${method}" "http://wadd${path}"
    fi
}
op() { api POST /v1/ops "{\"op\": $1}"; }
run_of() { api GET /v1/runs | jq -c --arg id "$1" '.[] | select(.id == $id)'; }
build_of() { api GET /v1/builds | jq -c --arg id "$1" '.[] | select(.id == $id)'; }

cleanup() {
    set +e
    say "cleaning up"
    for id in "${IDS[@]}"; do
        api POST "/v1/wadspaces/${id}/stop" > /dev/null 2>&1
        podman rm -f -t 3 "wad-${id}" > /dev/null 2>&1
        podman volume rm -f "wad-${id}-config" > /dev/null 2>&1
        rm -f "${UNITS}/wad-${id}.container"
    done
    systemctl --user daemon-reload
    for img in "${MADE_SETS[@]}"; do podman rmi "${img}" > /dev/null 2>&1; done
    [[ -n "${WADD_PID:-}" ]] && kill "${WADD_PID}" 2> /dev/null && wait "${WADD_PID}" 2> /dev/null
    stop_labwc
    echo "wadd's log: ${TRY}/wadd.log (kept)"
    find "${TRY}" -mindepth 1 -maxdepth 1 ! -name wadd.log -exec rm -rf {} +
}
MADE_SETS=()
trap cleanup EXIT

say "a headless labwc, and wadd on it"
start_labwc 1
mkdir -p "${TRY}/rt"
cat > "${TRY}/wadd.toml" << EOF
[machine]
id = "try"
name = "Try"
[daemon]
screen_dir = "${TRY}/screen"
signins_dir = "${TRY}/state/keys"
reconcile_s = 2
[display]
# Nothing to fall back on: the screen is what the shell says.
runtime_dir = "${TRY}/rt"
EOF
RUST_LOG=wadd=debug,info "${WADD}" serve --user --config "${TRY}/wadd.toml" --state-dir "${TRY}/state" \
    --socket "${TRY}/wadd.sock" > "${TRY}/wadd.log" 2>&1 &
WADD_PID=$!
for _ in $(seq 100); do [[ -S "${TRY}/wadd.sock" ]] && break; sleep 0.05; done
[[ "$(api GET /v1/health)" == ok ]] && ok "wadd answers" || { fail "wadd didn't start"; cat "${TRY}/wadd.log"; exit 1; }
screen="$(api PUT /v1/screen/session "{\"waylandSocket\": \"${hl_rt}/wayland-0\", \"uid\": $(id -u)}")"
echo "   ${screen}"
[[ "$(jq -r .from <<< "${screen}")" == shell ]] && ok "the shell's compositor is the screen" || fail "the screen: ${screen}"

# Opens a wadspace on the screen; waits until it's ready; checks its window.
open_and_check() {
    local id="$1" started st phase tops
    started=$(date +%s%3N)
    api POST "/v1/wadspaces/${id}/open" '{"display": "screen"}' | jq -c '{phase, message, display}'
    for _ in $(seq 600); do
        st="$(run_of "${id}")"
        phase="$(jq -r .phase <<< "${st}")"
        [[ "${phase}" == ready || "${phase}" == error ]] && break
        sleep 0.5
    done
    printf '   %s after %d ms\n' "$(jq -c '{phase, display, error}' <<< "${st}")" "$(( $(date +%s%3N) - started ))"
    [[ "${phase}" == ready ]] && ok "${id} is ready (its desktop is up)" || { fail "${id}: ${st}"; podman logs --tail 30 "wad-${id}" 2>&1 | sed 's/^/   /'; return; }
    tops=""
    for _ in $(seq 20); do
        tops="$(in_labwc "${TOPLEVELS}" 2> /dev/null)"
        grep -q "app_id=\"io.wadspaces.${id}\"" <<< "${tops}" && break
        sleep 0.5
    done
    sed 's/^/   toplevels: /' <<< "${tops}"
    grep -q "app_id=\"io.wadspaces.${id}\"" <<< "${tops}" && ok "its window is on the screen as io.wadspaces.${id}" || fail "no io.wadspaces.${id} window"
    mkdir -p "${ROOT}/.build/wadd"
    sleep 2
    shot HEADLESS-1 "${ROOT}/.build/wadd/try-${id}.png" 2> /dev/null && echo "   screenshot: .build/wadd/try-${id}.png"
    sleep 3
    api GET '/v1/logs?n=400' | jq -r '.[].message' | grep -E "^${id}: " | sed 's/^/   log: /'
    api GET /v1/screen | jq -c '{from, served}' | sed 's/^/   screen: /'
}

stop_and_check() {
    local id="$1" st
    st="$(api POST "/v1/wadspaces/${id}/stop")"
    echo "   $(jq -c '{phase, display, error}' <<< "${st}")"
    [[ "$(jq -r .phase <<< "${st}")" == idle ]] && ok "${id} stopped" || fail "stopping ${id}: ${st}"
    [[ "$(podman container exists "wad-${id}" && echo yes || echo no)" == no ]] && ok "its container is gone" || fail "wad-${id} still there"
    [[ ! -S "${TRY}/screen/${id}/wayland-1" ]] && ok "its socket is gone" || fail "its socket is still there"
}

# Waits for a build's job; prints its time.
wait_job() {
    local job="$1" j status
    for _ in $(seq 3600); do
        j="$(api GET "/v1/jobs/${job}")"
        status="$(jq -r .job.status <<< "${j}")"
        [[ "${status}" == done || "${status}" == error || "${status}" == cancelled ]] && break
        sleep 1
    done
    jq -r '.lines[]' <<< "${j}" | grep -E '^(»|✓|✗)' | sed 's/^/   /'
    printf '   %s in %d s\n' "${status}" "$(( ($(jq -r .job.endedAt <<< "${j}") - $(jq -r .job.startedAt <<< "${j}")) / 1000 ))"
    [[ "${status}" == done ]]
}

say "a wadspace with only the terminal"
id="$(op '{"op": "space.create", "name": "Try Terminal"}' | jq -r .created)"
IDS+=("${id}")
op "{\"op\": \"apps.add\", \"ws\": \"${id}\", \"app\": \"terminal\"}" > /dev/null
b="$(api POST "/v1/wadspaces/${id}/build")"
echo "   $(jq -c '{built: .state.built, layer: .state.layer, image: .state.image, job: .job.id}' <<< "${b}")"
[[ "$(jq -r .state.built <<< "${b}")" == built && "$(jq -r .job <<< "${b}")" == null ]] \
    && ok "nothing to build: it runs the base" || fail "build: ${b}"
open_and_check "${id}"
stop_and_check "${id}"

if ${SET}; then
    say "a wadspace with the terminal and ${TOOL}: its set"
    a="$(op '{"op": "space.create", "name": "Try Set"}' | jq -r .created)"
    IDS+=("${a}")
    op "{\"op\": \"builder.add\", \"ws\": \"${a}\", \"section\": \"addons\", \"item\": \"terminal:${TOOL}\"}" > /dev/null
    echo "   before: $(build_of "${a}" | jq -c '{built, layer}')"
    b="$(api POST "/v1/wadspaces/${a}/build")"
    job="$(jq -r .job.id <<< "${b}")"
    echo "   stages: $(jq -c '[.job.stages[].id]' <<< "${b}")"
    if wait_job "${job}"; then ok "the cold build"; else fail "the cold build"; fi
    MADE_SETS+=("$(build_of "${a}" | jq -r .image)")
    echo "   after: $(build_of "${a}" | jq -c '{built, layer, image, sizeMb}')"

    say "an edit (a step of its own): an update until it's built again"
    op "{\"op\": \"space.addCommand\", \"ws\": \"${a}\", \"kind\": \"run\", \"text\": \"echo hello > /etc/wadd-try\"}" | jq -c '{needsBuild}'
    st="$(build_of "${a}" | jq -c '{built, image}')"
    echo "   ${st}"
    [[ "$(jq -r .built <<< "${st}")" == update ]] && ok "it's an update, running the image it has" || fail "after the edit: ${st}"
    b="$(api POST "/v1/wadspaces/${a}/build")"
    if wait_job "$(jq -r .job.id <<< "${b}")"; then ok "the rebuild, from the cached package"; else fail "the rebuild"; fi
    MADE_SETS+=("$(build_of "${a}" | jq -r .image)")

    say "a wadspace with the same set as the cold build's: no build at all"
    c="$(op '{"op": "space.create", "name": "Try Same"}' | jq -r .created)"
    IDS+=("${c}")
    op "{\"op\": \"builder.add\", \"ws\": \"${c}\", \"section\": \"addons\", \"item\": \"terminal:${TOOL}\"}" > /dev/null
    b="$(api POST "/v1/wadspaces/${c}/build")"
    echo "   $(jq -c '{built: .state.built, image: .state.image, job: .job.id}' <<< "${b}")"
    [[ "$(jq -r .state.built <<< "${b}")" == built && "$(jq -r .job <<< "${b}")" == null ]] \
        && ok "built already (the same set)" || fail "the same set: ${b}"

    say "the rebuilt one, opened"
    open_and_check "${a}"
    podman exec "wad-${a}" sh -c "cat /etc/wadd-try; command -v ${TOOL} python3 node 2> /dev/null | head -1" 2>&1 | sed 's/^/   in it: /'
    stop_and_check "${a}"
fi

say "the result"
[[ ${FAILED} -eq 0 ]] && echo "all ok" || echo "${FAILED} failed"
exit $((FAILED > 0))
