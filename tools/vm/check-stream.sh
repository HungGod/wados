# tools/vm.sh stream-check | stream-phone: streams on a real VM (phase 7),
# sourced by tools/vm.sh, whose helpers it uses. One VM (VM_NAME stream:
# .build/vm/stream/), from a fresh layer over wados.qcow2, thrown away after.
# Every step says ok or FAIL, and the check goes on after a FAIL where it can.
# Screenshots and what the viewers saw: .build/vm/stream-shots/.
#
#   tools/vm.sh stream-check [--no-secrets | --turn]
#   tools/vm.sh stream-phone [--turn]
#
# Real TURN is off unless --turn says so (the person's permission: it puts
# their Cloudflare TURN key in the VM); --no-turn is the same as nothing.
#
# **stream-check --no-secrets**: the machine as it is without the installer
# stick's provision: no /etc/wados/turn.toml, no [streaming] host, no
# tailnet.
#   1. it boots; sam (uid 1000), greetd, unlocked; a wadspace (Stream Clock,
#      the terminal) built
#   2. GET /v1/streams: public off with the tunnel's reason, private with the
#      tailnet's, secure with the VM runtime's unless krun is here; starting
#      each answers 409 with that reason; no share, nothing to stop; wadd
#      (the same process) and the shell keep running; cloudflared.service
#      stays off (its ConditionPathExists); the shell's stream sheet with
#      its buttons off (a screenshot) and its Stream menu's reasons
# **stream-check** (or --turn): all of that, then in the same VM:
#   3. a quick tunnel: `cloudflared tunnel --url http://localhost:8085` as a
#      transient unit (cloudflared is in the image), its *.trycloudflare.com
#      host as [streaming] host with tunnel = "external" (tests only), wadd
#      restarted; public is available; the tunnel reaches wadd's proxy (the
#      expired page). The VM reaches Cloudflare through QEMU's NAT: this
#      check has no fence (the tailnet check's nftables), deliberately.
#   4. TURN, with --turn only: Cloudflare's key from the person's secrets
#      ([cloudflare.turn] only: tools/vm/turn-secret.py) over SSH's stdin
#      into the VM's /etc/wados/turn.toml (0600), never a command line, a
#      log or a file here. Without it (the default), no turn.toml: the
#      stream must say turnDown, and the TURN parts are skipped with a note.
#   5. a public stream of the wadspace while it's on the screen: 409 without
#      takeover, then the Stream job with it (its stages); live, its link at
#      the quick host, turnDown as expected, the shell's list; its share
#      (kept in a private folder, never printed); its folder, files, units
#      and ICE servers (their kinds only); a clock in it, moving
#   6. viewers from the laptop (a browser viewer: WADOS_STREAM_VIEWER, a
#      command taking view()'s JSON options on stdin and printing what it saw
#      as JSON; without one, the viewers' checks are skipped), through the tunnel: the routes without a key (308, the
#      card, 401, the open health, the expired page). In WebRTC mode (as
#      Selkies' public mode starts): the one-tap link and the user and
#      password, and the transport each ends up with (with --turn: they
#      stream, relayed; TURN only streams through Cloudflare's TURN;
#      without: what happens behind QEMU's NAT, recorded). A wrong token
#      and a wrong password stream nothing. Then the switch to WebSockets
#      from the page on its cookie alone, and in WebSocket mode the one-tap
#      link and the user and password stream: a screenshot not blank, a
#      tone in the wadspace reaching the viewer (Selkies' audio frames),
#      the address bar keeping no token, no 401s, frames on api/websockets;
#      while it's watched the stream's viewers and lastActive move, in wadd
#      and in the shell. No 4001 (a refused socket) in the sidecar's log.
#   7. adopted: wadd restarted, the stream is still live, and streams
#   8. stopped (DELETE): its link is the expired page through the tunnel;
#      nothing is left (route, units, sidecar, volume, secrets, folder)
#   9. no key, password or TURN credential in the journal (every field),
#      wadd's logs and jobs, or a command line in the VM, nor in what the
#      viewers wrote here
#  10. idle (STREAM_IDLE=0: skipped): with [streaming] idle_minutes = 1 (the
#      whole run's), a stream watched for longer than that stays live; left
#      alone, it ends by itself; three times, nothing left each time
#   (and on the way: every wadd restart, with the shell connected, under 10 s
#   and the shell's can= right after; after a DELETE, a wadspace that moved
#   to its stream is back on the screen, not restarted, three times, nothing
#   left each time; wadd's transport follows Selkies' switches within 5 s;
#   the sheet's Stream button off without secrets)
#  11. secure streams: private only for now, so tools/vm.sh tailnet-check
#      (step 17) checks them; a stub here for public-secure
#   Then: the quick tunnel stopped, turn.toml removed, the VM and its layer gone.
#
# **stream-phone**: the phone test, with a person. One VM, a quick tunnel,
# real TURN (--turn), a public stream of a wadspace with a clock running in it; it
# prints the one-tap link (and a QR code of it in the terminal: qrencode, or
# none), the plain link with its user and password, and
# waits for Enter (STREAM_PHONE_WAIT seconds at most, 1800), saying every
# 15 s who's watching and how. Then the stream's stats (viewers, transport,
# Selkies' status) and everything goes. STREAM_PHONE_VIEWER=browser (with WADOS_STREAM_VIEWER): the
# laptop's Chromium stands in for the phone (a test of the test), and no
# Enter is waited for.
#
# Secrets: WADOS_SECRETS, a folder with cloudflare.toml (or the file), by
# default host/secrets; only [cloudflare.turn] is read. All
# this prints goes through turn-secret.py mask. wadd mints the TURN
# credentials; this never asks Cloudflare for them.
#
# VM_MEM (MB, 2560), STREAM_IDLE (1), VM_KEEP=1 (leave the VM, its tunnel and
# turn.toml: tools/vm.sh --name stream ssh; then stream-check's cleanup by
# hand, or tools/vm.sh --name stream stop).

S_NAME=stream
S_UNIT=wados-quick-tunnel
S_PROTO="${ROOT}/crates/wad-proto/src/streams.rs"
S_TUNNEL_RS="${ROOT}/apps/wadd/src/tunnel.rs"
# A browser viewer (none: the checks that need one are skipped). The Node and
# Playwright one went with the v2 spike.
S_VIEW="${WADOS_STREAM_VIEWER:-}"
S_SECRET="${HERE}/turn-secret.py"

ok() { printf 'ok    %s\n' "$*"; }
fail() { printf 'FAIL  %s\n' "$*"; FAILED=$((FAILED + 1)); }
note() { printf '      %s\n' "$*"; }
skip() { printf 'skip  %s\n' "$*"; }
indent() { sed 's/^/      | /'; }
step_time() { printf '      (%d s)\n' "$(($(now) - STEP_T))"; STEP_T=$(now); }

wait_for() {
    local deadline=$(($(now) + $1))
    shift
    while (($(now) < deadline)); do
        "$@" && return 0
        sleep 2
    done
    return 1
}

s_port() { cat "${VM}/${S_NAME}/ssh.port"; }
as_root() { ssh_to "$(s_port)" root "$@"; }
as_sam() { ssh_to "$(s_port)" sam "$@"; }
shell() { as_sam "XDG_RUNTIME_DIR=/run/user/1000 wadsctl $*" 2> /dev/null; }
s_qmp() { python3 "${HERE}/qmp.py" "${VM}/${S_NAME}/qmp.sock" "$@"; }
s_shot() {
    if s_qmp shot "${SHOTS}/$1" 2> /dev/null; then note "screenshot: ${SHOTS}/$1"; else note "(no screenshot: $1)"; fi
}
s_journal() { as_root "journalctl -b --no-pager -o short-monotonic $* | tail -n 25" 2> /dev/null | indent; }

# wadd's API through its socket, as sam: sets CODE and BODY; true on a 2xx.
call() {
    local m="$1" p="$2" body="${3:-}" out
    local curl="curl -sS --max-time 120 --unix-socket /run/wadd/wadd.sock -X ${m} -w '\n%{http_code}'"
    if [[ -n "${body}" ]]; then
        out="$(as_sam "${curl} -H 'content-type: application/json' --data-binary @- http://wadd${p}" <<< "${body}" 2>&1)"
    else
        out="$(as_sam "${curl} http://wadd${p}" < /dev/null 2>&1)"
    fi
    CODE="${out##*$'\n'}"
    BODY="${out%$'\n'*}"
    [[ "${BODY}" == "${CODE}" ]] && BODY=""
    [[ "${CODE}" == 2?? ]]
}
# The body of a GET (empty unless it's a 2xx).
get() { call GET "$1" && printf '%s' "${BODY}"; }

# A reason as wad-proto (or wadd's tunnel.rs) says it, from the source.
reason() {
    python3 -I -c 'import re, sys
m = re.search(r"pub const " + sys.argv[2] + r": &str =\s*\"((?:[^\"\\]|\\.)*)\"", open(sys.argv[1]).read())
print(m.group(1) if m else "?")' "${2:-${S_PROTO}}" "$1"
}

job_done() {
    local id="$1" secs="${2:-300}" st deadline
    deadline=$(($(now) + secs))
    while (($(now) < deadline)); do
        if call GET "/v1/jobs/${id}"; then
            st="$(jq -r '.job.status // .status' <<< "${BODY}" 2> /dev/null)"
            case "${st}" in
                done) printf '%s' "${BODY}"; return 0 ;;
                error | cancelled) printf '%s' "${BODY}"; return 1 ;;
            esac
        fi
        sleep 2
    done
    printf 'still %s after %d s: %s' "${st:-?}" "${secs}" "${BODY}"
    return 1
}
job_summary() { jq -c '(.job // .) | {kind, status, title, error}' 2> /dev/null <<< "$1" || head -c 300 <<< "$1"; }
stages_of() { jq -r '(.job // .) | .stages[]? | "\(.label): \(.state)\(if .error then " (" + .error + ")" else "" end)"' <<< "$1" 2> /dev/null; }

# This wadspace's stream in /v1/streams (empty: none).
stream_of() { get /v1/streams | jq -c --arg id "$1" '.streams[]? | select(.id == $id and (.peer == null))' 2> /dev/null; }
phase_of() { get /v1/runs | jq -r --arg id "$1" '[.[] | select(.id == $id) | .phase][0] // "idle"' 2> /dev/null; }

# ------------------------------------------------------------ helpers (phase 7 fixes)

# systemctl restart wadd with the shell connected: under 10 s; then the
# shell's can= is what /v1/streams says (`can=?`: not known yet).
restart_wadd() {
    local took want got t_can
    took="$(as_root 's=$(date +%s%N); systemctl restart wadd; e=$(date +%s%N); echo $(( (e - s) / 1000000 ))' 2> /dev/null)"
    if [[ "${took}" =~ ^[0-9]+$ ]] && ((took < 10000)); then
        ok "systemctl restart wadd, with the shell connected: $((took / 1000)).$(printf '%03d' $((took % 1000))) s"
    else
        fail "systemctl restart wadd took ${took:-?} ms (want under 10 s)"
    fi
    wait_for 30 eval '[[ "$(as_sam "curl -s --unix-socket /run/wadd/wadd.sock http://wadd/v1/health" 2> /dev/null)" == ok ]]'
    want="$(get /v1/streams | jq -r '[.available | to_entries[] | select(.value.available) | .key] | sort | join(",")' 2> /dev/null)"
    t_can=$(now)
    got="?"
    while (($(now) - t_can < 15)); do
        got="$(shell stream | sed -n 's/^ok can=\([^ |]*\).*/\1/p' | tr ',' '\n' | sort | paste -sd, -)"
        [[ "${got}" == "${want}" ]] && break
        sleep 1
    done
    if [[ "${got}" == "${want}" ]]; then
        ok "and the shell's can= is right $(($(now) - t_can)) s after wadd answers (can=${got:-nothing}, as /v1/streams says)"
    else
        fail "the shell's can= after the restart: '${got}', wadd says '${want}'"
    fi
}

# Selkies switched (the key's), then wadd's transport follows within about 5 s:
# transport_follows websockets|webrtc websocket|webrtc
transport_follows() {
    local t0 tr="" took
    switch_to "$1"
    t0="$(date +%s%N)"
    for _ in $(seq 30); do
        tr="$(stream_of "${WS}" | jq -r .transport 2> /dev/null)"
        [[ "${tr}" == "$2"* ]] && break
        sleep 0.5
    done
    took=$((($(date +%s%N) - t0) / 1000000))
    if [[ "${tr}" == "$2"* ]] && ((took <= 6000)); then
        ok "GET /v1/streams says transport ${tr} $((took / 1000)).$(printf '%03d' $((took % 1000))) s after the switch to $1"
    else
        fail "GET /v1/streams' transport after the switch to $1: ${tr:-?} after ${took} ms"
    fi
}

# Its container's start time (a restart changes it).
started_at() { as_root "podman inspect wad-$1 --format '{{.State.StartedAt}}'" 2> /dev/null; }

# After a DELETE, a wadspace that moved to its stream as it ran is back on the
# screen, its window there, without a restart: back_on_screen WS STARTED_AT
back_on_screen() {
    local ws="$1" started="$2" run wins="" now_started
    wait_for 60 eval '[[ "$(get /v1/runs | jq -r --arg id "${ws}" ".[] | select(.id == \$id) | \"\(.phase) \(.display)\"" 2> /dev/null)" == "ready screen" ]]'
    run="$(get /v1/runs | jq -c --arg id "${ws}" '.[] | select(.id == $id) | {phase, display}' 2> /dev/null)"
    for _ in $(seq 15); do
        wins="$(shell wm list)"
        grep -q "wadspace ${ws} " <<< "${wins}" && break
        sleep 2
    done
    now_started="$(started_at "${ws}")"
    if [[ "${run}" == '{"phase":"ready","display":"screen"}' ]] && grep -q "wadspace ${ws} " <<< "${wins}" && [[ -n "${started}" && "${now_started}" == "${started}" ]]; then
        ok "back on the screen (${run}, its window in wm list), not restarted (its container started at ${started}, still)"
    else
        fail "after the DELETE: run ${run:-none}; its window in wm list: $(grep -c "wadspace ${ws} " <<< "${wins}"); its container started ${started:-?} → ${now_started:-gone}"
    fi
}

# Three times: on the screen, taken off it to stream (it moves as it runs),
# DELETEd: each time nothing of the stream left, and it's back on the screen
# as it was, not restarted.
delete_repeats() {
    local ws="$1" i id j ph started
    for i in 1 2 3; do
        if [[ "$(get /v1/runs | jq -r --arg id "${ws}" '.[] | select(.id == $id) | "\(.phase) \(.display)"' 2> /dev/null)" != "ready screen" ]]; then
            call POST "/v1/wadspaces/${ws}/open" '{"display": "screen"}' > /dev/null
            for _ in $(seq 150); do
                ph="$(phase_of "${ws}")"
                [[ "${ph}" == ready || "${ph}" == error ]] && break
                sleep 2
            done
            [[ "${ph}" == ready ]] || { fail "#${i}: opening it on the screen: ${ph}"; continue; }
            sleep 3
        fi
        started="$(started_at "${ws}")"
        if ! call POST "/v1/wadspaces/${ws}/stream" '{"mode": "public", "takeover": true}'; then
            fail "#${i}: taking it off the screen to stream: ${CODE} ${BODY}"
            continue
        fi
        id="$(jq -r '.id // .job.id' <<< "${BODY}")"
        if ! j="$(job_done "${id}" 300)"; then fail "#${i}: its Stream job: $(job_summary "${j}")"; continue; fi
        if [[ "$(started_at "${ws}")" == "${started}" ]]; then ok "#${i}: it moved to its stream as it ran (not restarted)"; else fail "#${i}: its container started again for the stream"; fi
        if call DELETE "/v1/wadspaces/${ws}/stream"; then ok "#${i}: DELETE: stopped ($(jq -c '{stage, live}' <<< "${BODY}"))"; else fail "#${i}: DELETE: ${CODE} ${BODY}"; fi
        wait_for 30 eval '[[ -z "$(stream_of "${ws}")" ]]' || fail "#${i}: still listed"
        stream_leftovers "${ws}"
        back_on_screen "${ws}" "${started}"
    done
}

# ------------------------------------------------------------ the start

stream_main() {
    local cmd="$1"
    shift
    set +e
    FAILED=0
    S_TUNNEL_ON=""
    S_WATCHDOG=""
    S_MASK=""
    VM_NAME="${S_NAME}" RUN="${VM}/${S_NAME}"
    SHOTS="${VM}/stream-shots"
    mkdir -p "${SHOTS}"
    # What a stream hands on (its share), here only for the run: a private
    # folder outside the repo, gone at the end.
    S_PRIV="$(mktemp -d "${XDG_RUNTIME_DIR:-/tmp}/wados-stream-check.XXXXXX")"
    chmod 0700 "${S_PRIV}"
    # Everything printed goes through the mask (the TURN pair's values).
    exec 3>&1 4>&2
    exec > >(python3 -I "${S_SECRET}" mask >&3) 2>&1
    S_MASK=$!
    trap 'stream_cleanup' EXIT
    trap 'exit 130' INT TERM
    case "${cmd}" in
        stream-check) stream_check "$@" ;;
        stream-phone) stream_phone "$@" ;;
    esac
}

stream_preflight() {
    local free_mb
    [[ -f "${VM}/wados.qcow2" ]] || { fail "no ${VM}/wados.qcow2: tools/vm.sh disk"; return 1; }
    [[ -n "${S_VIEW}" ]] || note "no browser viewer (WADOS_STREAM_VIEWER): what a viewer sees is skipped"
    free_mb="$(awk '/^MemAvailable:/ { a = $2 } /^SwapFree:/ { s = $2 } END { print int((a + s / 2) / 1024) }' /proc/meminfo)"
    note "the laptop has about ${free_mb} MB to spare (available, and half the free swap) for ${MEM} MB"
    ((free_mb < MEM + 512)) && note "WARNING: that's tight; other VMs or builds may be running"
    # One VM at a time on this laptop: another's (another check, a builder)
    # is waited for, STREAM_WAIT_VM seconds at most (1200), then it goes on.
    local d others deadline=$(($(now) + ${STREAM_WAIT_VM:-1200}))
    while :; do
        others=""
        for d in "${VM}" "${VM}"/*/; do
            d="${d%/}"
            [[ "${d##*/}" == "${S_NAME}" ]] && continue
            running "${d}/qemu.pid" && others+=" ${d##*/}"
        done
        running "${VM}/builder.pid" && others+=" builder"
        [[ -z "${others}" ]] && break
        if (($(now) >= deadline)); then
            note "WARNING: another VM is still running (${others# }): going on beside it"
            break
        fi
        note "another VM is running (${others# }): waiting for it (one VM at a time)"
        sleep 30
    done
    return 0
}

# Boots the stream VM from a fresh layer, with the laptop's memory watched.
stream_boot() {
    STEP_T=$(now)
    stop > /dev/null
    rm -f "${RUN}/run.qcow2" "${RUN}/vars.fd"
    boot --fresh
    rm -f "${VM}/stream-starved"
    (
        while sleep 5; do
            left="$(awk '/^MemAvailable:/ { a = $2 } /^SwapFree:/ { s = $2 } END { print int((a + s) / 1024) }' /proc/meminfo)"
            if ((left < 300)); then
                echo "${left}" > "${VM}/stream-starved"
                python3 "${HERE}/qmp.py" "${VM}/${S_NAME}/qmp.sock" quit 2> /dev/null
                exit 0
            fi
        done
    ) &
    S_WATCHDOG=$!
    if wait_ssh "$(s_port)" root 420 "${RUN}/qemu.pid"; then
        ok "root's SSH after $(($(now) - STEP_T)) s ($(as_root 'sed -n "s/^PRETTY_NAME=//p" /etc/os-release' | tr -d '"'))"
    else
        fail "no SSH within 420 s (${RUN}/serial.log)"
        tail -n 20 "${RUN}/serial.log" | indent
        return 1
    fi
}

# sam, as check.sh makes them; their session up and unlocked; the machine's
# id; the sidecar's image in root's podman (loaded from the laptop if the
# disk was made without it); a wadspace with the terminal, built.
stream_ready() {
    local name="$1" b
    if as_root bash -s -- "${SAM_PASSWORD}" << 'EOF'
set -e
groups=wheel
getent group wados > /dev/null && groups+=,wados
getent passwd 1000 > /dev/null || useradd -m -u 1000 -U -G "${groups}" -c Sam sam
echo "sam:$1" | chpasswd
install -d -m 0700 -o sam -g sam ~sam/.ssh
install -m 0600 -o sam -g sam /root/.ssh/authorized_keys ~sam/.ssh/authorized_keys
hostnamectl set-hostname wados-stream
mkdir -p /etc/wados
printf '%s\n' '# tools/vm/check-stream.sh (the installer'"'"'s, on a real machine).' '[machine]' 'id = "wados-stream"' > /etc/wados/wadd.toml
rm -f /etc/wados/turn.toml
systemctl restart wadd
systemctl reset-failed greetd 2> /dev/null || true
systemctl restart greetd
EOF
    then
        ok "sam made, wadd's [machine] id (no host, no turn.toml), greetd started again"
    else
        fail "making sam"
    fi
    if wait_for 240 eval '[[ "$(shell locked)" == "ok true" ]]'; then
        s_qmp keys "${SAM_PASSWORD}" && s_qmp key ret
        if wait_for 30 eval '[[ "$(shell locked)" == "ok false" ]]'; then ok "sam's session is up and unlocked"; else fail "still locked after sam's password"; fi
    else
        fail "sam's session didn't come up locked ($(shell locked))"
        s_journal -u greetd -t labwc
    fi
    if as_root "podman image exists localhost/wados-stream:44"; then
        ok "the sidecar's image is in root's podman (localhost/wados-stream:44)"
    elif podman image exists localhost/wados-stream:44; then
        note "the disk hasn't localhost/wados-stream:44 (made before tools/vm.sh disk put it there): loading it over SSH"
        if podman save localhost/wados-stream:44 | as_root 'podman load -q' > /dev/null 2>&1; then
            ok "localhost/wados-stream:44 loaded into root's podman"
        else
            fail "loading localhost/wados-stream:44 into the VM"
        fi
    else
        fail "no localhost/wados-stream:44 in the VM or here (images/build.sh)"
    fi
    wait_for 30 eval '[[ "$(as_sam "curl -s --unix-socket /run/wadd/wadd.sock http://wadd/v1/health" 2> /dev/null)" == ok ]]'
    WS="$(call POST /v1/ops "{\"op\": {\"op\": \"space.create\", \"name\": \"${name}\"}}" && jq -r .created <<< "${BODY}")"
    if [[ -z "${WS}" || "${WS}" == null ]]; then
        fail "no wadspace made: ${CODE} ${BODY}"
        WS=""
        return 1
    fi
    call POST /v1/ops "{\"op\": {\"op\": \"apps.add\", \"ws\": \"${WS}\", \"app\": \"terminal\"}}" > /dev/null
    call POST "/v1/wadspaces/${WS}/build"
    b="${BODY}"
    if [[ "$(jq -r '.state.built // .built' <<< "${b}" 2> /dev/null)" == built ]]; then
        ok "${WS} (${name}, the terminal): built"
    else
        local j
        j="$(jq -r '.job.id // empty' <<< "${b}" 2> /dev/null)"
        [[ -n "${j}" ]] && job_done "${j}" 600 > /dev/null
        if [[ "$(get /v1/builds | jq -r --arg id "${WS}" '.[] | select(.id == $id) | (.state.built // .built)' 2> /dev/null)" == built ]]; then
            ok "${WS} (${name}): built"
        else
            fail "building ${WS}: ${b}"
        fi
    fi
}

# A clock, moving, in the wadspace's terminal (on whatever compositor it
# draws on now): something for frames to show.
clock_in() {
    local out
    out="$(as_root "podman exec -i --user abc wad-$1 sh -s" 2>&1 << 'EOF'
cd /config
export XDG_RUNTIME_DIR=/config/.XDG
WAYLAND_DISPLAY="$(basename "$(ls -t /config/.XDG/wayland-[0-9] 2> /dev/null | head -n 1)")"
export WAYLAND_DISPLAY
cat > /tmp/wados-clock.sh << 'CLOCK'
while :; do printf '\033[2J\033[H\n\n   %s\n\n   WadOS stream\n' "$(date +%H:%M:%S)"; sleep 1; done
CLOCK
setsid foot --title Clock --font=monospace:size=40 sh /tmp/wados-clock.sh < /dev/null > /dev/null 2>&1 &
sleep 2
pgrep -f wados-clock.sh > /dev/null && echo "running on ${WAYLAND_DISPLAY}" || echo "not running (display: ${WAYLAND_DISPLAY:-none})"
EOF
)"
    if [[ "${out}" == running* ]]; then ok "a clock in ${1}'s terminal, ${out}"; else fail "the clock in ${1}: ${out}"; fi
}

# ------------------------------------------------------------ stream-check

stream_check() {
    local no_secrets="" turn="" t0
    case "${1:-}" in
        --no-secrets) no_secrets=1 ;;
        --turn) turn=1 ;;
        --no-turn | "") ;;
        *) echo "stream-check [--no-secrets | --turn]"; exit 2 ;;
    esac
    rm -f "${SHOTS}"/*.png "${SHOTS}"/*.json "${SHOTS}"/*.err
    t0=$(now)
    STEP_T=$(now)
    S_TURN=""
    if [[ -n "${turn}" ]]; then
        if python3 -I "${S_SECRET}" check; then
            S_TURN=1
        else
            note "--turn, but no [cloudflare.turn] in the secrets (WADOS_SECRETS): the TURN parts are skipped"
        fi
    fi

    say "0. before it starts"
    stream_preflight || stream_finish "${t0}"
    say "1. it boots (a fresh layer), sam's session, a wadspace"
    stream_boot || stream_finish "${t0}"
    stream_ready "Stream Clock"
    step_time

    say "2. no secrets: no tunnel, no TURN, no tailnet"
    no_secrets_check
    step_time
    if [[ -n "${no_secrets}" ]]; then
        secure_stub
        stream_finish "${t0}"
    fi

    say "3. a quick tunnel to wadd's proxy (cloudflared, a transient unit)"
    # A long idle_minutes while viewers come and go (a step can go over a
    # minute without a signed-in viewer); idle_check sets 1 for its own runs.
    quick_tunnel 30 || stream_finish "${t0}"
    step_time

    say "4. TURN"
    turn_in
    step_time

    say "5. a public stream of ${WS}, taken off the screen"
    public_start "${WS}" || { stream_leftovers "${WS}"; stream_finish "${t0}"; }
    step_time

    say "6. viewers, from the laptop, through the tunnel"
    if [[ -n "${S_VIEW}" ]]; then viewers_check; else skip "no browser viewer (WADOS_STREAM_VIEWER)"; fi
    step_time

    say "7. adopted after wadd restarts"
    adopt_check
    step_time

    say "8. stopped"
    needles_keep "${WS}"
    stop_check "${WS}"
    step_time

    say "8b. on the screen, taken off it and DELETEd: three times, no wadd restart between"
    delete_repeats "${WS}"
    step_time

    say "9. no secrets in the logs"
    needles_check
    step_time

    if [[ "${STREAM_IDLE:-1}" == 1 ]]; then
        say "10. idle, three times: watched, it stays; left alone, it ends"
        idle_check "${WS}"
        step_time
    fi

    secure_stub
    stream_finish "${t0}"
}

no_secrets_check() {
    local list priv pub ps us secure_why pid0 pid1 cf menu krun
    PRIVATE_NEEDS_TAILNET="$(reason PRIVATE_NEEDS_TAILNET)"
    PUBLIC_NEEDS_TUNNEL="$(reason PUBLIC_NEEDS_TUNNEL)"
    SECURE_NEEDS_VM="$(reason SECURE_NEEDS_VM)"
    SECURE_PUBLIC_LATER="$(reason SECURE_PUBLIC_LATER)"
    pid0="$(as_root 'systemctl show -p MainPID --value wadd' 2> /dev/null)"
    if ! list="$(get /v1/streams)"; then
        fail "GET /v1/streams: ${CODE} ${BODY}"
        return
    fi
    ok "GET /v1/streams answers 200: $(jq -c '{streams: (.streams | length), available}' <<< "${list}")"
    priv="$(jq -r '.available.private.reason // "available"' <<< "${list}")"
    pub="$(jq -r '.available.public.reason // "available"' <<< "${list}")"
    if [[ "${priv}" == "${PRIVATE_NEEDS_TAILNET}" ]]; then ok "private: off, \"${priv}\""; else fail "private: ${priv} (want \"${PRIVATE_NEEDS_TAILNET}\")"; fi
    if [[ "${pub}" == "${PUBLIC_NEEDS_TUNNEL}" ]]; then ok "public: off, \"${pub}\""; else fail "public: ${pub} (want \"${PUBLIC_NEEDS_TUNNEL}\")"; fi
    krun="$(as_root 'command -v krun; test -w /dev/kvm && echo kvm' 2> /dev/null | tr '\n' ' ')"
    ps="$(jq -r '.available["private-secure"].reason // "available"' <<< "${list}")"
    us="$(jq -r '.available["public-secure"].reason // "available"' <<< "${list}")"
    if [[ "${krun}" == *krun*kvm* ]]; then
        # A VM runtime here: private-secure says the tailnet's reason;
        # public-secure the tunnel's, or that it comes later (wad-proto's).
        if [[ "${ps}" == "${PRIVATE_NEEDS_TAILNET}" && ( "${us}" == "${PUBLIC_NEEDS_TUNNEL}" || "${us}" == "${SECURE_PUBLIC_LATER}" ) ]]; then
            ok "secure: krun is here (${krun% }): private-secure gives the tailnet's reason, public-secure \"${us}\""
        else
            fail "secure, with krun here: private-secure \"${ps}\", public-secure \"${us}\""
        fi
    elif [[ "${ps}" == "${SECURE_NEEDS_VM}" && "${us}" == "${SECURE_NEEDS_VM}" ]]; then
        ok "secure: off, \"${ps}\" (no krun in this image)"
    else
        fail "secure: private-secure \"${ps}\", public-secure \"${us}\" (krun: ${krun:-none})"
    fi
    local mode want got
    for mode in public private public-secure private-secure; do
        case "${mode}" in
            public) want="${pub}" ;;
            private) want="${priv}" ;;
            public-secure) want="${us}" ;;
            private-secure) want="${ps}" ;;
        esac
        call POST "/v1/wadspaces/${WS}/stream" "{\"mode\": \"${mode}\", \"takeover\": true}"
        got="$(jq -r .message <<< "${BODY}" 2> /dev/null)"
        if [[ "${CODE}" == 409 && "${got}" == "${want}" ]]; then
            ok "starting ${mode}: 409, \"${got}\""
        else
            fail "starting ${mode}: ${CODE} ${BODY} (want 409 \"${want}\")"
        fi
    done
    call GET "/v1/streams/${WS}/share"
    if [[ "${CODE}" == 404 ]]; then ok "no share without a stream (404)"; else fail "GET /v1/streams/${WS}/share: ${CODE} ${BODY}"; fi
    call DELETE "/v1/wadspaces/${WS}/stream"
    if [[ "${CODE}" == 404 ]]; then ok "nothing to stop (404)"; else fail "DELETE /v1/wadspaces/${WS}/stream: ${CODE} ${BODY}"; fi
    [[ "$(get /v1/streams | jq '.streams | length')" == 0 ]] && ok "and no stream was left in the list" || fail "streams: $(get /v1/streams)"
    pid1="$(as_root 'systemctl show -p MainPID --value wadd' 2> /dev/null)"
    if [[ -n "${pid0}" && "${pid0}" != 0 && "${pid0}" == "${pid1}" && "$(as_root 'systemctl is-active wadd' 2> /dev/null)" == active ]]; then
        ok "wadd keeps running, the same process (pid ${pid1}, $(as_root 'systemctl show -p NRestarts --value wadd') restarts)"
    else
        fail "wadd: pid ${pid0} → ${pid1}, $(as_root 'systemctl is-active wadd' 2>&1)"
    fi
    if [[ "$(shell session)" == ok* ]]; then ok "the shell keeps running ($(as_sam 'pgrep -x -u sam wados-shell' | head -1))"; else fail "the shell doesn't answer"; fi
    cf="$(as_root 'systemctl show cloudflared -p ActiveState -p ConditionResult -p UnitFileState | sort | tr "\n" " "; test -e /etc/cloudflared/token && echo token || echo no-token' 2> /dev/null)"
    if [[ "${cf}" == "ActiveState=inactive ConditionResult=no "*"no-token" ]]; then
        ok "cloudflared.service stays off: no /etc/cloudflared/token, its condition (${cf})"
    else
        fail "cloudflared.service: ${cf}"
    fi
    as_root 'test -e /etc/wados/turn.toml' && fail "/etc/wados/turn.toml is there" || ok "no /etc/wados/turn.toml"

    # The shell: its view of the modes, its Stream menu, the sheet with its buttons off.
    note "wadsctl stream: $(shell stream)"
    menu="$(shell stream menu "${WS}")"
    note "wadsctl stream menu ${WS}: ${menu}"
    if [[ "$(shell stream)" == "ok can= |"* || "$(shell stream)" == "ok can=" ]]; then
        ok "the shell can start no mode (wadsctl stream: can= nothing)"
    else
        fail "the shell's modes: $(shell stream)"
    fi
    if [[ "${menu}" == *"off(${PUBLIC_NEEDS_TUNNEL})"* && "${menu}" == *"off(${PRIVATE_NEEDS_TAILNET})"* ]]; then
        ok "its Stream menu has the modes off, with their reasons"
    else
        fail "its Stream menu doesn't give the reasons: ${menu}"
    fi
    shell page home > /dev/null
    sleep 2
    local mode d
    for mode in public private; do
        if [[ "$(shell stream sheet "${WS}" "${mode}")" == ok* ]]; then
            sleep 3
            d="$(shell dialog)"
            s_shot "2-stream-sheet-${mode}.png"
            if [[ "${d}" == *"go(off)"* ]]; then ok "the ${mode} sheet's Stream button is off (wadsctl dialog: ${d})"; else fail "the ${mode} sheet's Stream button isn't off: ${d}"; fi
            shell dialog close > /dev/null
            sleep 1
        else
            fail "wadsctl stream sheet ${WS} ${mode}: $(shell stream sheet "${WS}" "${mode}")"
        fi
    done
    restart_wadd
}

# ------------------------------------------------------------ the tunnel

# The quick tunnel, as a transient unit; its host into wadd.toml (and
# [streaming] idle_minutes = $1), wadd restarted.
quick_tunnel() {
    local idle="$1" deadline code
    as_root "systemctl stop ${S_UNIT} 2> /dev/null; systemd-run --quiet --collect --unit ${S_UNIT} --property=TimeoutStopSec=10 \
        /usr/bin/cloudflared --no-autoupdate tunnel --metrics 127.0.0.1:20242 --url http://localhost:8085" 2>&1 | indent
    S_TUNNEL_ON=1
    QHOST=""
    for _ in $(seq 60); do
        QHOST="$(as_root "journalctl -u ${S_UNIT} -o cat --no-pager" 2> /dev/null | grep -o 'https://[a-z0-9-]*\.trycloudflare\.com' | grep -v '//api\.' | head -1)"
        QHOST="${QHOST#https://}"
        [[ -n "${QHOST}" ]] && break
        sleep 1
    done
    if [[ -z "${QHOST}" ]]; then
        fail "no trycloudflare.com host from cloudflared within 60 s"
        as_root "journalctl -u ${S_UNIT} -o cat --no-pager | tail -n 15" 2>&1 | indent
        return 1
    fi
    if wait_for 30 eval '[[ "$(as_root "curl -s -m 3 http://127.0.0.1:20242/ready" 2> /dev/null | jq -r .readyConnections 2> /dev/null)" -ge 1 ]]'; then
        ok "the quick tunnel is up: ${QHOST} ($(as_root "journalctl -u ${S_UNIT} -o cat --no-pager | grep -oE 'Initial protocol [a-z0-9]+|Registered tunnel connection[^ ]* [^ ]*' | head -2 | tr '\n' ' '"))"
    else
        fail "the quick tunnel's /ready: $(as_root 'curl -s -m 3 http://127.0.0.1:20242/ready' 2>&1)"
    fi
    if as_root bash -s -- "${QHOST}" "${idle}" << 'EOF'
set -e
cat > /etc/wados/wadd.toml << TOML
# tools/vm/check-stream.sh (the installer's, on a real machine).
[machine]
id = "wados-stream"

# A quick tunnel (cloudflared tunnel --url), which the check runs: tests only.
[streaming]
host = "$1"
tunnel = "external"
idle_minutes = $2
TOML
EOF
    then
        ok "wadd.toml: [streaming] host = \"${QHOST}\", tunnel = \"external\", idle_minutes = ${idle}"
    else
        fail "writing wadd.toml"
    fi
    restart_wadd
    local pub
    pub="$(get /v1/streams | jq -c '.available.public' 2> /dev/null)"
    if [[ "${pub}" == '{"available":true}' ]]; then ok "public is available (${pub})"; else fail "public: ${pub}"; fi
    note "the shell: $(shell stream)"
    # The laptop reaches the proxy through Cloudflare (a new host may take a moment).
    deadline=$(($(now) + 90))
    while (($(now) < deadline)); do
        code="$(curl -s -m 15 -o /dev/null -w '%{http_code}' "https://${QHOST}/" 2> /dev/null)"
        [[ "${code}" != 000 && "${code}" != 530 && "${code}" != 502 && "${code}" != 1033 ]] && break
        sleep 3
    done
    local page
    page="$(curl -s -m 15 "https://${QHOST}/ws-00000000000000000000000000000000/")"
    if [[ "${code}" == 404 ]] && grep -q "$(reason EXPIRED_TITLE "${ROOT}/apps/wadd/src/proxy/pages.rs")" <<< "${page}"; then
        ok "through the tunnel, wadd's proxy answers: / and an unknown guid get the expired page (404, \"This link has expired\")"
    else
        fail "through the tunnel: / gives ${code}; an unknown guid: $(head -c 200 <<< "${page}")"
    fi
    note "cloudflared to the proxy: $(as_root "ss -tnpH state established '( dport = :8085 )' | grep -o '\[::1\]:8085\|127.0.0.1:8085' | sort | uniq -c | tr '\n' ' '" 2> /dev/null)"
}

turn_in() {
    if [[ -z "${S_TURN}" ]]; then
        skip "TURN: no turn.toml in the VM (no --turn, or no secrets): the stream should say turnDown"
        return
    fi
    # The pair, from the secrets to the VM's file, through SSH's stdin only.
    if python3 -I "${S_SECRET}" toml | as_root 'umask 077; install -d -m 0755 /etc/wados && cat > /etc/wados/turn.toml.new && chmod 0600 /etc/wados/turn.toml.new && mv /etc/wados/turn.toml.new /etc/wados/turn.toml'; then
        ok "Cloudflare's TURN key is in the VM's /etc/wados/turn.toml ($(as_root 'stat -c "%a %U" /etc/wados/turn.toml'), [cloudflare.turn] only, through SSH's stdin)"
    else
        fail "putting turn.toml in the VM"
        S_TURN=""
    fi
}

# ------------------------------------------------------------ a public stream

# Opens it on the screen, then streams it (public): 409 without takeover,
# then with it. Its share in S_PRIV/share.json. GUID, LINK, WEB_PORT set.
public_start() {
    local ws="$1" ph job id j info st
    call POST "/v1/wadspaces/${ws}/open" '{"display": "screen"}' > /dev/null
    for _ in $(seq 150); do
        ph="$(phase_of "${ws}")"
        [[ "${ph}" == ready || "${ph}" == error ]] && break
        sleep 2
    done
    if [[ "${ph}" == ready ]]; then ok "${ws} is on the screen (ready)"; else fail "${ws} on the screen: ${ph}"; fi
    STARTED_AT="$(started_at "${ws}")"
    sleep 3
    s_shot 5-on-the-screen.png
    call POST "/v1/wadspaces/${ws}/stream" '{"mode": "public"}'
    if [[ "${CODE}" == 409 ]]; then ok "streaming it while it's on the screen, without takeover: 409, \"$(jq -r .message <<< "${BODY}")\""; else fail "without takeover: ${CODE} ${BODY}"; fi
    if ! call POST "/v1/wadspaces/${ws}/stream" '{"mode": "public", "takeover": true}'; then
        fail "POST /v1/wadspaces/${ws}/stream {public, takeover}: ${CODE} ${BODY}"
        s_journal -u wadd
        return 1
    fi
    job="${BODY}"
    id="$(jq -r '.id // .job.id' <<< "${job}")"
    ok "a Stream job: $(job_summary "${job}")"
    if j="$(job_done "${id}" 420)"; then
        ok "the Stream job is done ($(job_summary "${j}"))"
    else
        fail "the Stream job: $(job_summary "${j}")"
        stages_of "${j}" | indent
        s_journal -u wadd -u "wadstream-${ws}" -u "wad-${ws}"
        return 1
    fi
    stages_of "${j}" | indent
    if [[ "$(started_at "${ws}")" == "${STARTED_AT}" ]]; then ok "it moved to its stream as it ran (its container started at ${STARTED_AT}, still)"; else note "its container started again for the stream ($(started_at "${ws}"))"; fi
    info="$(stream_of "${ws}")"
    note "the stream: ${info}"
    LINK="$(jq -r .link <<< "${info}")"
    GUID="$(sed -n 's|.*/ws-\([0-9a-z]*\)/$|\1|p' <<< "${LINK}")"
    if [[ "$(jq -r .live <<< "${info}")" == true && "${LINK}" == "https://${QHOST}/ws-"*/ && -n "${GUID}" ]]; then
        ok "live, public, at ${LINK} (transport $(jq -r .transport <<< "${info}"))"
    else
        fail "the stream isn't live at the quick host: ${info}"
    fi
    if [[ -n "${S_TURN}" ]]; then
        if [[ "$(jq -r .turnDown <<< "${info}")" == false ]]; then ok "turnDown: false (wadd minted TURN credentials)"; else fail "turnDown with turn.toml there: $(jq -c '{turnDown, warnings}' <<< "${info}")"; fi
    else
        if [[ "$(jq -r .turnDown <<< "${info}")" == true && "$(jq -r '.warnings | join(" ")' <<< "${info}")" == *"$(reason TURN_DOWN)"* ]]; then
            ok "turnDown, with the warning (no turn.toml): \"$(reason TURN_DOWN)\""
        else
            fail "no turn.toml, yet: $(jq -c '{turnDown, warnings}' <<< "${info}")"
        fi
    fi
    note "the shell: $(shell stream)"
    # Its share: kept here, never printed.
    if as_sam "curl -sS --unix-socket /run/wadd/wadd.sock http://wadd/v1/streams/${ws}/share" < /dev/null > "${S_PRIV}/share.json" 2> /dev/null &&
        jq -e --arg l "${LINK}" '.mode == "public" and .link == $l and (.openLink | startswith($l + "#token=")) and (.openLink | length) > ($l | length) + 7 and (.user | length) > 0 and (.password | length) > 0' "${S_PRIV}/share.json" > /dev/null; then
        ok "its share: the one-tap link (${LINK}#token=…), the link, a user ($(jq -r .user "${S_PRIV}/share.json")) and a password (…)"
    else
        fail "its share isn't right: $(jq -c '{mode, link, user, openLink: (.openLink | sub("token=.*"; "token=…")), password: (if .password then "…" else null end)}' "${S_PRIV}/share.json" 2> /dev/null)"
    fi
    jq -r '.openLink | capture("#token=(?<k>.*)$").k' "${S_PRIV}/share.json" > "${S_PRIV}/key" 2> /dev/null
    printf 'Authorization: Bearer %s\n' "$(cat "${S_PRIV}/key")" > "${S_PRIV}/bearer"
    # What wadd made for it, in the VM.
    as_root bash -s -- "${ws}" << 'EOF' 2>&1 | indent
id="$1"
d="/var/lib/wados/streams/${id}"
echo "folder: $(stat -c '%a %U:%G' "${d}" 2> /dev/null) $(ls -dZ "${d}" 2> /dev/null | awk '{ print $1 }')"
for f in "${d}"/*; do echo "  $(basename "${f}"): $(stat -c '%a %U:%G' "${f}") $(ls -Z "${f}" | awk '{ print $1 }')"; done
python3 -I - "${d}/rtc.json" << 'PY'
import json, sys
try:
    servers = json.load(open(sys.argv[1])).get("iceServers", [])
except Exception as e:
    print("rtc.json:", type(e).__name__); sys.exit()
kinds = {}
for s in servers:
    for u in ([s["urls"]] if isinstance(s.get("urls"), str) else s.get("urls", [])):
        k = u.split(":", 1)[0] + ("?" + u.split("?", 1)[1] if "?" in u else "") + (":" + u.split(":")[-1].split("?")[0])
        kinds[k] = kinds.get(k, 0) + 1
print("ICE servers (kinds only):", ", ".join(f"{k}" for k in sorted(kinds)) or "none",
      "| with credentials:", sum(1 for s in servers if s.get("username") and s.get("credential")))
PY
echo "units: $(ls /run/containers/systemd | grep -E "${id}" | tr '\n' ' ')"
echo "running: $(podman ps --format '{{.Names}}' | grep -E "${id}" | tr '\n' ' ')"
echo "secrets: $(podman secret ls --format '{{.Name}}' | grep -E "^wad_stream_" | tr '\n' ' ')"
echo "sidecar: $(systemctl show "wadstream-${id}" -p ActiveState -p BindsTo -p StopWhenUnneeded --value 2> /dev/null | tr '\n' ' ')"
echo "wadspace: $(systemctl show "wad-${id}" -p ActiveState -p BindsTo -p Requires --value 2> /dev/null | tr '\n' ' ')"
EOF
    WEB_PORT="$(as_root "podman port wadstream-${ws} 3000/tcp" 2> /dev/null | sed 's/.*://' | head -1)"
    st="$(as_root "curl -s -m 5 http://127.0.0.1:${WEB_PORT}/ws-${GUID}/api/status" 2> /dev/null)"
    note "Selkies' status: ${st:0:240}"
    note "the run: $(get /v1/runs | jq -c --arg id "${ws}" '.[] | select(.id == $id) | {phase, display}' 2> /dev/null)"
    clock_in "${ws}"
    sleep 2
    s_shot 5-streamed-screen.png
    return 0
}

# A viewer (WADOS_STREAM_VIEWER), with options from jq over the share: view
# NAME 'jq filter' (the filter makes the options from the share).
view() {
    local name="$1" filter="$2" r
    jq -c --arg out "${SHOTS}/${name}.png" "${filter} + {out: \$out}" "${S_PRIV}/share.json" |
        timeout 240 ${S_VIEW} > "${SHOTS}/${name}.json" 2> "${SHOTS}/${name}.err"
    r=$?
    VIEW="$(cat "${SHOTS}/${name}.json")"
    note "${name}: $(jq -c '{page, oneTap, card, signIn, signInPost, refused, secure, streaming, transport, candidates, held, iceServers, colours, stddev, tokenInAddress, errors, error}' <<< "${VIEW}" 2> /dev/null || head -c 300 <<< "${VIEW}")"
    [[ -s "${SHOTS}/${name}.err" ]] && head -n 3 "${SHOTS}/${name}.err" | indent
    return "${r}"
}

drawn() {
    local d
    d="$(python3 "${HERE}/screen.py" "${SHOTS}/$1.png" 2>&1)" && { printf '%s' "${d}"; return 0; }
    printf '%s' "${d}"
    return 1
}

# Selkies' mode for the stream, by the key (a Bearer header, through the tunnel).
switch_to() {
    local c
    c="$(curl -s -m 20 -X POST -H @"${S_PRIV}/bearer" -H 'Content-Type: application/json' -d "{\"mode\": \"$1\"}" "https://${QHOST}/ws-${GUID}/api/switch")"
    if grep -q '"status": "success"' <<< "${c}"; then ok "Selkies switched to $1 (the key as a Bearer header, through the tunnel)"; else fail "switching to $1: ${c:0:200}"; fi
}
mode_now() { curl -s -m 10 "https://${QHOST}/ws-${GUID}/api/status" | jq -r .current_mode 2> /dev/null; }

# The one-tap link, held while viewers are counted (wadd's and the shell's)
# and a tone plays in the wadspace: held_view NAME TRANSPORT ['extra jq'].
held_view() {
    local name="$1" want="$2" extra="${3:-}" before info vmax svmax sample tone d audio
    [[ -n "${extra}" ]] || extra='{}'
    before="$(stream_of "${WS}" | jq -r .lastActive)"
    (
        sleep 22
        as_root "podman exec -i --user abc wad-${WS} python3 - 12 440" < "${HERE}/tone.py" > "${S_PRIV}/tone" 2>&1
    ) &
    tone=$!
    (
        for _ in $(seq 40); do
            sleep 3
            printf '%s %s\n' "$(stream_of "${WS}" | jq -c '{viewers, lastActive, transport}' 2> /dev/null)" "$(shell stream | grep -o 'viewers=[0-9]*' | head -1)"
        done
    ) > "${S_PRIV}/viewers" 2> /dev/null &
    sample=$!
    if view "${name}" "{url: .openLink, expect: \"stream\", wait: 60, hold: 30} + ${extra}" && [[ "$(jq -r .transport <<< "${VIEW}")" == "${want}" ]]; then
        ok "the one-tap link streams over ${want} ($(jq -c '.held' <<< "${VIEW}"))"
    else
        fail "the one-tap link doesn't stream over ${want}: $(jq -c '{page, oneTap, switched, card, streaming, transport, sockets: [.sockets[]? | {url, frames, closed}][:4], errors, consoleErrors, error}' <<< "${VIEW}" 2> /dev/null)"
    fi
    kill "${sample}" 2> /dev/null
    wait "${tone}" 2> /dev/null
    note "the tone in the wadspace: $(cat "${S_PRIV}/tone")"
    if [[ ! -f "${SHOTS}/${name}.png" ]]; then
        fail "no screenshot (${name}.png)"
    elif d="$(drawn "${name}")"; then
        ok "its screenshot isn't blank (${d}; ${SHOTS}/${name}.png)"
    else
        fail "its screenshot is blank (${d})"
    fi
    if [[ "${want}" == webrtc ]]; then
        note "candidate pairs (local/remote): $(jq -c .candidates <<< "${VIEW}"); $(jq -c '[.rtc[]? | {state, local: .pair.local, remote: .pair.remote, audioPackets}]' <<< "${VIEW}")"
        audio="$(jq '.held.rtcAudioPackets // 0' <<< "${VIEW}")"
    else
        note "data socket frames by kind (1 audio, 3/4 video): $(jq -c '[.sockets[]? | select(.url | test("api/websockets")) | .kinds]' <<< "${VIEW}")"
        audio="$(jq '.held.socketAudioFrames // 0' <<< "${VIEW}")"
    fi
    if [[ "${audio:-0}" -gt 0 ]]; then ok "the wadspace's tone reached the viewer (${audio} audio $([[ "${want}" == webrtc ]] && echo packets || echo frames) while it was held)"; else fail "no audio reached the viewer while the tone played"; fi
    if [[ "$(jq -r .oneTap <<< "${VIEW}")" == 204 && "$(jq -r .tokenInAddress <<< "${VIEW}")" == false ]]; then
        ok "the card traded the key for the cookie (204), and the address bar keeps no token ($(jq -r .address <<< "${VIEW}"))"
    else
        fail "the one-tap flow: POST $(jq -r .oneTap <<< "${VIEW}"), token in the address bar: $(jq -r .tokenInAddress <<< "${VIEW}")"
    fi
    if [[ "$(jq '[.errors[]? | select(startswith("401"))] | length' <<< "${VIEW}")" == 0 ]]; then
        ok "no 401 while it streamed (on the HttpOnly cookie alone: every asset and socket under /ws-<guid>/)"
    else
        fail "401s while it streamed: $(jq -c '[.errors[] | select(startswith("401"))]' <<< "${VIEW}")"
    fi
    note "sockets: $(jq -c '[.sockets[]? | {url, frames, closed}]' <<< "${VIEW}"); console errors: $(jq -c '.consoleErrors' <<< "${VIEW}")"
    note "viewers while it was watched (wadd; the shell): $(sort -u "${S_PRIV}/viewers" | tr '\n' ' ' | head -c 600)"
    vmax="$(grep -o '"viewers":[0-9]*' "${S_PRIV}/viewers" | cut -d: -f2 | sort -n | tail -1)"
    svmax="$(grep -o 'viewers=[0-9]*' "${S_PRIV}/viewers" | cut -d= -f2 | sort -n | tail -1)"
    if [[ "${vmax:-0}" -ge 1 ]]; then ok "wadd counted the viewer (viewers: ${vmax})"; else fail "wadd's viewers stayed 0 while it was watched"; fi
    if [[ "${svmax:-0}" -ge 1 ]]; then ok "and the shell showed it (wadsctl stream: viewers=${svmax})"; else fail "the shell's viewers stayed 0"; fi
    info="$(stream_of "${WS}")"
    if [[ "$(jq -r .lastActive <<< "${info}")" -gt "${before:-0}" ]]; then ok "lastActive moved while it was watched (${before} → $(jq -r .lastActive <<< "${info}"))"; else fail "lastActive didn't move (${before} → $(jq -r .lastActive <<< "${info}"))"; fi
}

# The user and password on the plain link's card: signs in, and streams over $2
# (or, when $3 is "record", says what happened).
sign_in_view() {
    local name="$1" want="$2" how="${3:-}"
    view "${name}" '{url: .link, user: .user, password: .password, expect: "stream", wait: 50}'
    if [[ "$(jq -r .signInPost <<< "${VIEW}")" == 303 && "$(jq -r .card <<< "${VIEW}")" == true ]]; then
        ok "the right user and password sign in (the card's POST: 303, with the cookie)"
    else
        fail "signing in: $(jq -c '{card, signIn, signInPost, refused, error}' <<< "${VIEW}")"
    fi
    if [[ "${how}" == record ]]; then
        note "${want} mode, signed in: streaming $(jq -r .streaming <<< "${VIEW}"), transport $(jq -r .transport <<< "${VIEW}"), pairs $(jq -c .candidates <<< "${VIEW}"), ICE $(jq -c '[.rtc[]? | .ice]' <<< "${VIEW}")"
    elif [[ "$(jq -r .streaming <<< "${VIEW}")" == true && "$(jq -r .transport <<< "${VIEW}")" == "${want}" ]]; then
        ok "and it streams over ${want} ($(jq -c .candidates <<< "${VIEW}"))"
    else
        fail "signed in, it doesn't stream over ${want}: $(jq -c '{streaming, transport, sockets: [.sockets[]? | {url, frames, closed}][:4], errors, consoleErrors}' <<< "${VIEW}")"
    fi
}

# Selkies refused no socket (4001: "missing or invalid token") since $1.
no_refusals() {
    local r
    r="$(as_root "journalctl --no-pager -o cat -u wadstream-${WS} --since @$1 | grep -acE 'Rejecting client|Invalid authentication token|4001'" 2> /dev/null)"
    if [[ "${r:-0}" == 0 ]]; then ok "the sidecar refused no socket while viewers came and went (no 4001, no \"Rejecting client\")"; else fail "the sidecar refused ${r} socket(s): $(as_root "journalctl --no-pager -o cat -u wadstream-${WS} --since @$1 | grep -aE 'Rejecting client|4001' | tail -n 3" 2> /dev/null | tr '\n' ' ')"; fi
}

viewers_check() {
    local t c card h ws401 t0
    t="https://${QHOST}"
    t0="$(as_root 'date +%s' 2> /dev/null)"
    # The routes, without a key.
    c="$(curl -s -m 15 -o /dev/null -w '%{http_code} %{redirect_url}' "${t}/ws-${GUID}")"
    if [[ "${c}" == "308 ${t}/ws-${GUID}/" ]]; then ok "/ws-<guid> → 308 to /ws-<guid>/"; else fail "/ws-<guid>: ${c}"; fi
    card="$(curl -s -m 15 -w '\n%{http_code}' "${t}/ws-${GUID}/")"
    if [[ "${card##*$'\n'}" == 200 && "${card}" == *"is shared with you"* && "${card}" == *'name="password"'* ]]; then
        ok "/ws-<guid>/ without a key: the sign-in card (200, a user and password form)"
    else
        fail "/ws-<guid>/ without a key: ${card##*$'\n'} $(head -c 200 <<< "${card}")"
    fi
    h="$(curl -s -m 15 -o /dev/null -w '%{http_code}' "${t}/ws-${GUID}/api/health")"
    ws401="$(curl -s -m 15 -o /dev/null -w '%{http_code}' "${t}/ws-${GUID}/api/websockets")"
    if [[ "${h}" == 200 && "${ws401}" == 401 ]]; then ok "api/health is open (200); api/websockets without a key: 401"; else fail "api/health ${h}, api/websockets ${ws401}"; fi

    say "6a. WebRTC mode (Selkies' public mode as it starts: $(mode_now))"
    if [[ -n "${S_TURN}" ]]; then
        held_view 6-webrtc-one-tap webrtc
        [[ "$(jq -r '.candidates | join(" ")' <<< "${VIEW}")" == *relay* ]] && note "(relayed through TURN, as behind QEMU's NAT it should be)" || note "(not relayed: a direct path, or STUN's)"
        if view 6-webrtc-relay-only '{url: .openLink, expect: "stream", wait: 60, relay: true}' && [[ "$(jq -r '.candidates | join(" ")' <<< "${VIEW}")" == relay/* ]]; then
            ok "TURN only (iceTransportPolicy relay): it streams through Cloudflare's TURN ($(jq -c '[.rtc[]? | .pair.local | {type, protocol, relayProtocol, url}]' <<< "${VIEW}"))"
        else
            fail "TURN only: $(jq -c '{streaming, candidates, iceServers, rtc}' <<< "${VIEW}" 2> /dev/null)"
        fi
        [[ "$(jq -r '.iceServers | join(" ")' <<< "${VIEW}")" == *turn:* ]] && ok "the page was given Cloudflare's TURN servers ($(jq -r '.iceServers | join(" ")' <<< "${VIEW}"))" || fail "no turn: server among the page's ICE servers: $(jq -c .iceServers <<< "${VIEW}")"
        sign_in_view 6-webrtc-sign-in webrtc
    else
        # No TURN (turnDown): what a viewer behind the laptop's NAT gets of a
        # sidecar behind QEMU's, through the tunnel.
        view 6-webrtc-one-tap '{url: .openLink, wait: 45}'
        if [[ "$(jq -r .oneTap <<< "${VIEW}")" == 204 && "$(jq -r .tokenInAddress <<< "${VIEW}")" == false ]]; then
            ok "the one-tap link signs in (the key for the cookie: 204; no token left in the address bar)"
        else
            fail "the one-tap link's sign-in: $(jq -c '{page, oneTap, card, tokenInAddress, error}' <<< "${VIEW}")"
        fi
        note "WebRTC without TURN: streaming $(jq -r .streaming <<< "${VIEW}"), transport $(jq -r .transport <<< "${VIEW}"), ICE servers $(jq -c .iceServers <<< "${VIEW}"), pairs $(jq -c .candidates <<< "${VIEW}"), states $(jq -c '[.rtc[]? | {state, ice}]' <<< "${VIEW}"), sockets $(jq -c '[.sockets[]? | {url, frames}]' <<< "${VIEW}"), Selkies' mode after: $(mode_now) (${SHOTS}/6-webrtc-one-tap.png)"
        if [[ "$(jq -r .streaming <<< "${VIEW}")" == true ]]; then
            ok "without TURN it still streams, over $(jq -r .transport <<< "${VIEW}") ($(jq -c .candidates <<< "${VIEW}"))"
        else
            ok "without TURN, WebRTC finds no path from the laptop to the VM behind QEMU's NAT, and Selkies doesn't fall back by itself: what turnDown warns of (recorded, not a fault)"
        fi
        sign_in_view 6-webrtc-sign-in webrtc record
    fi
    if view 6-wrong-token '{url: (.link + "#token=not-the-key"), expect: "none", wait: 20}' && [[ "$(jq -r .oneTap <<< "${VIEW}")" == 403 ]]; then
        ok "a wrong token streams nothing (its POST: 403; the card says the link is old)"
    else
        fail "a wrong token: $(jq -c '{page, oneTap, streaming, error}' <<< "${VIEW}")"
    fi
    if view 6-wrong-password '{url: .link, user: .user, password: "not-the-password", expect: "refused"}'; then
        ok "the plain link's card refuses a wrong password ($(jq -r .signIn <<< "${VIEW}"), \"That’s not it\")"
    else
        fail "a wrong password: $(jq -c '{card, signIn, refused, streaming, error}' <<< "${VIEW}")"
    fi

    say "6b. the switch to WebSockets, from the page on its cookie alone; then WebSocket mode"
    held_view 6-websockets-one-tap websockets '{switch: "websockets"}'
    if [[ "$(jq -r .switched <<< "${VIEW}")" == "200 "* && "$(mode_now)" == websockets ]]; then
        ok "the page switched Selkies to WebSockets with its cookie alone ($(jq -r .switched <<< "${VIEW}" | head -c 80))"
    else
        fail "the switch on the cookie: $(jq -r .switched <<< "${VIEW}"); Selkies' mode: $(mode_now)"
    fi
    note "wadd's transport now: $(stream_of "${WS}" | jq -r .transport)"
    transport_follows webrtc webrtc
    transport_follows websockets websocket
    sign_in_view 6-websockets-sign-in websockets
    no_refusals "${t0}"
    if [[ -n "${S_TURN}" ]]; then
        switch_to webrtc
    else
        note "(left on WebSockets: without TURN, that's how viewers get it here)"
    fi
}

adopt_check() {
    local before after
    before="$(stream_of "${WS}" | jq -c '{link, live, mode}')"
    restart_wadd
    wait_for 60 eval '[[ "$(stream_of "${WS}" | jq -r .live)" == true ]]'
    after="$(stream_of "${WS}" | jq -c '{link, live, mode}')"
    if [[ -n "${before}" && "${before}" == "${after}" ]]; then ok "after systemctl restart wadd, the stream is adopted: ${after}"; else fail "adopted? before ${before}, after ${after:-none}"; fi
    if [[ -z "${S_VIEW}" ]]; then skip "its one-tap link streams (no browser viewer)"
    elif view 7-adopted '{url: .openLink, expect: "stream", wait: 60}'; then ok "and its one-tap link still streams"; else fail "after the restart, it doesn't stream"; fi
}

# The secrets a stream has, kept in the VM (root's, 0600, in /run) to look
# for in the logs once it's gone: its key and password, the TURN key, and
# the TURN credentials minted for it.
needles_keep() {
    local out
    out="$(as_root bash -s -- "$1" << 'EOF' 2>&1
id="$1"
umask 077
f=/run/wados-check-needles
{
    podman secret inspect --showsecret --format '{{.SecretData}}' "wad_stream_key_${id}" "wad_stream_password_${id}" "wad_stream_master_${id}" 2> /dev/null
    python3 -I - "/var/lib/wados/streams/${id}/rtc.json" /etc/wados/turn.toml << 'PY'
import json, sys, tomllib
try:
    for s in json.load(open(sys.argv[1])).get("iceServers", []):
        for k in ("username", "credential"):
            if s.get(k): print(s[k])
except Exception:
    pass
try:
    t = tomllib.load(open(sys.argv[2], "rb")).get("cloudflare", {}).get("turn", {})
    for k in ("token_id", "api_token"):
        if t.get(k): print(t[k])
except Exception:
    pass
PY
} | awk 'length($0) >= 8' | sort -u >> "${f}"
sort -u -o "${f}" "${f}"
echo "kept $(wc -l < "${f}") secrets to look for (key, password, master token, TURN key and credentials)"
# While it's live: no process has one on its command line.
echo "cmdlines=$(cat /proc/[0-9]*/cmdline 2> /dev/null | tr '\0' '\n' | grep -acFf "${f}")"
EOF
)"
    jq -r '.password' "${S_PRIV}/share.json" > "${S_PRIV}/needles" 2> /dev/null
    cat "${S_PRIV}/key" >> "${S_PRIV}/needles" 2> /dev/null
    grep -v '^cmdlines=' <<< "${out}" | indent
    if [[ "${out}" == *"cmdlines=0"* ]]; then
        ok "while it's live, no process in the VM has a secret of it on its command line"
    else
        fail "a secret on a command line in the VM: $(grep '^cmdlines=' <<< "${out}")"
    fi
}

stop_check() {
    local ws="$1" ended page code left ph
    if call DELETE "/v1/wadspaces/${ws}/stream"; then
        ended="${BODY}"
        ok "stopped: $(jq -c '{stage, live, viewers}' <<< "${ended}")"
    else
        fail "DELETE /v1/wadspaces/${ws}/stream: ${CODE} ${BODY}"
    fi
    wait_for 30 eval '[[ -z "$(stream_of "${ws}")" ]]' && ok "it left the list" || fail "still listed: $(stream_of "${ws}")"
    page="$(curl -s -m 15 -w '\n%{http_code}' "https://${QHOST}/ws-${GUID}/")"
    code="${page##*$'\n'}"
    if [[ "${code}" == 404 && "${page}" == *"This link has expired"* ]]; then ok "its link, through the tunnel: the expired page (404, \"This link has expired\")"; else fail "its link after the stop: ${code} $(head -c 200 <<< "${page}")"; fi
    if [[ -z "${S_VIEW}" ]]; then
        skip "a viewer gets the expired page (no browser viewer)"
    elif view 8-expired '{url: .openLink, expect: "none", wait: 6}' && [[ "$(jq -r .page <<< "${VIEW}")" == 404 ]]; then
        ok "a viewer opening the one-tap link now gets the expired page and nothing streams (${SHOTS}/8-expired.png)"
    else
        fail "the one-tap link after the stop: $(jq -c '{page, streaming, error}' <<< "${VIEW}")"
    fi
    stream_leftovers "${ws}"
    back_on_screen "${ws}" "${STARTED_AT}"
    sleep 2
    s_shot 8-after-stop.png
}

# Nothing left of a stream of $1 (within 30 s: the containers stop first; the time is said).
stream_leftovers() {
    local left t0=$(now)
    # Up to 30 s: teardown waits for the containers to stop first (the
    # sidecar's stop alone takes a few seconds), then removes the volume.
    for _ in $(seq 15); do
        left="$(stream_left "$1")"
        [[ -z "${left}" ]] && break
        sleep 2
    done
    [[ -z "${left}" && $(($(now) - t0)) -gt 2 ]] && note "(its last leftovers went $(($(now) - t0)) s after it ended)"
    if [[ -z "${left}" ]]; then
        ok "nothing left of the stream: no sidecar (unit or container), volume, secrets or folder"
    else
        fail "$(($(now) - t0)) s after it ended, still left of it: $(tr '\n' ';' <<< "${left}")"
    fi
}
stream_left() {
    local left
    left="$(as_root bash -s -- "$1" << 'EOF' 2>&1
id="$1"
ls /run/containers/systemd 2> /dev/null | grep -E "^wadstream-${id}" | sed 's/^/unit /'
podman ps -a --format '{{.Names}}' | grep -E "^wadstream-${id}" | sed 's/^/container /'
podman volume ls --format '{{.Name}}' | grep -E "^wadstream-${id}" | sed 's/^/volume /'
podman secret ls --format '{{.Name}}' | grep -E "^wad_stream_.*${id}$" | sed 's/^/secret /'
test -e "/var/lib/wados/streams/${id}" && echo "folder /var/lib/wados/streams/${id}"
systemctl list-units --plain --no-legend --all "wadstream-${id}*" 2> /dev/null | awk '$3 != "inactive" { print "systemd " $1 " " $3 }'
# A movable wadspace back on the screen keeps its display volume (it can move again).
podman inspect "wad-${id}" --format '{{range .Mounts}}{{.Name}} {{end}}' 2> /dev/null | grep -qw "wadstream-${id}" && echo "kept-by-wadspace"
true
EOF
)"
    if [[ "${left}" == *kept-by-wadspace* ]]; then
        note "the wadspace still runs and mounts its display volume (wadstream-${1}), so that and its -volume.service stay" >&2
        left="$(grep -vE "^(kept-by-wadspace|volume wadstream-${1}$|unit wadstream-${1}\.volume$|systemd wadstream-${1}-volume\.service )" <<< "${left}")"
    fi
    printf '%s' "${left}"
}

needles_check() {
    local r
    r="$(as_root bash -s << 'EOF' 2>&1
f=/run/wados-check-needles
[ -s "${f}" ] || { echo "no secrets were kept"; exit 0; }
j="$(journalctl -b --no-pager -o export 2> /dev/null | grep -acFf "${f}")"
w="$(curl -s --unix-socket /run/wadd/wadd.sock 'http://wadd/v1/logs?n=5000' | grep -cFf "${f}")"
jobs="$(curl -s --unix-socket /run/wadd/wadd.sock http://wadd/v1/jobs | grep -cFf "${f}")"
files="$(grep -rlFf "${f}" /var/lib/wados --exclude-dir=containers 2> /dev/null | grep -v '/streams/' | tr '\n' ' ')"
echo "journal=${j} wadd-logs=${w} jobs=${jobs} state-files=[${files}]"
rm -f "${f}"
EOF
)"
    note "in the VM, lines holding a secret: ${r}"
    if [[ "${r}" == "journal=0 wadd-logs=0 jobs=0 state-files=[]" ]]; then
        ok "no key, password, master token, TURN key or TURN credential in the journal (all its fields: wadd's, cloudflared's, the sidecar's), wadd's logs or jobs, or wadd's state"
    else
        fail "a secret was found: ${r}"
    fi
    if [[ -s "${S_PRIV}/needles" ]] && grep -rqFf "${S_PRIV}/needles" "${SHOTS}" 2> /dev/null; then
        fail "the stream's key or password is in what the viewers wrote in ${SHOTS}"
    else
        ok "nor in what the viewers wrote here (${SHOTS}: tokens masked)"
    fi
}

idle_check() {
    local ws="$1" i id j info t_end page code
    if as_root "sed -i 's/^idle_minutes = .*/idle_minutes = 1/' /etc/wados/wadd.toml"; then
        ok "idle_minutes = 1 for the idle runs"
    else
        fail "setting idle_minutes = 1"
    fi
    restart_wadd
    for i in 1 2 3; do
        call POST "/v1/wadspaces/${ws}/stream" '{"mode": "public", "takeover": true}' || { fail "idle #${i}: a stream: ${CODE} ${BODY}"; continue; }
        id="$(jq -r '.id // .job.id' <<< "${BODY}")"
        j="$(job_done "${id}" 420)" || { fail "idle #${i}: its Stream job: $(job_summary "${j}")"; continue; }
        info="$(stream_of "${ws}")"
        GUID="$(jq -r .link <<< "${info}" | sed -n 's|.*/ws-\([0-9a-z]*\)/$|\1|p')"
        ok "idle #${i}: a stream, live ($(jq -c '{link, started}' <<< "${info}"))"
        if [[ "${i}" == 1 ]]; then
            as_sam "curl -sS --unix-socket /run/wadd/wadd.sock http://wadd/v1/streams/${ws}/share" < /dev/null > "${S_PRIV}/share.json" 2> /dev/null
            jq -r '.openLink | capture("#token=(?<k>.*)$").k' "${S_PRIV}/share.json" > "${S_PRIV}/key" 2> /dev/null
            printf 'Authorization: Bearer %s\n' "$(cat "${S_PRIV}/key")" > "${S_PRIV}/bearer"
            clock_in "${ws}"
            # Without TURN, viewers here get it over WebSockets.
            [[ -z "${S_TURN}" ]] && switch_to websockets
            # Watched for longer than idle_minutes (1): it must stay.
            if [[ -z "${S_VIEW}" ]]; then skip "watched for 90 s (no browser viewer): it may end, unwatched"
            elif view 10-watched '{url: .openLink, expect: "stream", wait: 60, hold: 90}'; then ok "watched for 90 s: $(jq -c .held <<< "${VIEW}")"; else fail "watching it: $(jq -c '{streaming, error}' <<< "${VIEW}")"; fi
            info="$(stream_of "${ws}")"
            if [[ "$(jq -r .live <<< "${info}")" == true ]]; then ok "still live after being watched past idle_minutes (1 min)"; else fail "it ended while it was watched: ${info:-gone}"; fi
        fi
        # Left alone: it ends by itself.
        t_end=$(now)
        if wait_for 240 eval '[[ -z "$(stream_of "${ws}")" ]]'; then
            ok "idle #${i}: left alone, it ended by itself after $(($(now) - t_end)) s (idle_minutes = 1)"
        else
            fail "idle #${i}: left alone for 240 s, it's still there: $(stream_of "${ws}")"
            continue
        fi
        for _ in 1 2 3; do
            page="$(curl -s -m 15 -w '\n%{http_code}' "https://${QHOST}/ws-${GUID}/")"
            code="${page##*$'\n'}"
            [[ "${code}" == 404 && "${page}" == *"This link has expired"* ]] && break
            sleep 4
        done
        if [[ "${code}" == 404 && "${page}" == *"This link has expired"* ]]; then ok "idle #${i}: its link is the expired page (404)"; else fail "idle #${i}: its link after the idle end: ${code} $(head -c 160 <<< "${page%$'\n'*}")"; fi
        stream_leftovers "${ws}"
        note "the wadspace after the idle end: $(get /v1/runs | jq -c --arg id "${ws}" '[.[] | select(.id == $id) | {phase, display}][0] // "idle"' 2> /dev/null)"
    done
}

secure_stub() {
    say "11. secure streams"
    # Secure streams are private for now (public-secure: SECURE_PUBLIC_LATER),
    # so they need a tailnet: tools/vm.sh tailnet-check's step 17 runs
    # tools/krun/check-secure.sh on its VM A. HOOK(secure-public): when
    # public-secure comes, a stream of it here, as in 5-9.
    skip "secure streams: private only for now, so they're checked on the tailnet (tools/vm.sh tailnet-check --streams, step 17)"
}

# ------------------------------------------------------------ stream-phone

stream_phone() {
    local turn="" t0 info j id qr stats left
    [[ "${1:-}" == --turn ]] && turn=1
    t0=$(now)
    STEP_T=$(now)
    S_TURN=""
    if [[ -n "${turn}" ]]; then
        if python3 -I "${S_SECRET}" check; then S_TURN=1; else note "--turn, but no [cloudflare.turn] in the secrets: no TURN"; fi
    else
        note "no --turn: no TURN (a phone on mobile data may get only WebSockets through the tunnel, or nothing)"
    fi
    rm -f "${SHOTS}"/phone-*
    say "the phone test: a VM, a quick tunnel, TURN, a public stream with a clock"
    stream_preflight || stream_finish "${t0}"
    stream_boot || stream_finish "${t0}"
    stream_ready "Phone Clock"
    quick_tunnel 30 || stream_finish "${t0}"
    turn_in
    if ! call POST "/v1/wadspaces/${WS}/stream" '{"mode": "public", "takeover": true}'; then
        fail "streaming ${WS}: ${CODE} ${BODY}"
        stream_finish "${t0}"
    fi
    id="$(jq -r '.id // .job.id' <<< "${BODY}")"
    if j="$(job_done "${id}" 420)"; then ok "the stream is live"; else fail "the Stream job: $(job_summary "${j}")"; stages_of "${j}" | indent; stream_finish "${t0}"; fi
    info="$(stream_of "${WS}")"
    note "$(jq -c '{link, transport, turnDown, warnings}' <<< "${info}")"
    as_sam "curl -sS --unix-socket /run/wadd/wadd.sock http://wadd/v1/streams/${WS}/share" < /dev/null > "${S_PRIV}/share.json" 2> /dev/null
    clock_in "${WS}"
    step_time

    echo
    echo "================================================================"
    echo " Open this on the phone (mobile data, Wi-Fi off):"
    echo
    echo "   $(jq -r .openLink "${S_PRIV}/share.json")"
    echo
    if command -v qrencode > /dev/null; then
        jq -r .openLink "${S_PRIV}/share.json" | qrencode -t ANSIUTF8 -m 2
    else
        echo "   (no qrencode: the link above only)"
    fi
    echo
    echo " Or the link, signing in:"
    echo "   $(jq -r .link "${S_PRIV}/share.json")"
    echo "   user: $(jq -r .user "${S_PRIV}/share.json")   password: $(jq -r .password "${S_PRIV}/share.json")"
    echo "================================================================"
    echo

    local limit=$(($(now) + ${STREAM_PHONE_WAIT:-1800})) line=""
    if [[ "${STREAM_PHONE_VIEWER:-}" == browser && -n "${S_VIEW}" ]]; then
        note "STREAM_PHONE_VIEWER=browser: the laptop's viewer (WADOS_STREAM_VIEWER) stands in for the phone"
        if view phone-stand-in '{url: .openLink, expect: "stream", wait: 60, hold: 20}'; then ok "the stand-in streams ($(jq -r .transport <<< "${VIEW}"), $(jq -c .candidates <<< "${VIEW}"))"; else fail "the stand-in doesn't stream"; fi
    else
        echo "Press Enter when you're done (at most $(((limit - $(now)) / 60)) minutes)."
        local rc
        while (($(now) < limit)); do
            read -r -t 15 line < /dev/tty 2> /dev/null
            rc=$?
            ((rc == 0)) && break
            # No terminal to read from: waits all the same, until the limit.
            ((rc > 128)) || sleep 15
            note "$(date +%T): $(stream_of "${WS}" | jq -c '{viewers, transport, lastActive, live}' 2> /dev/null || echo gone)"
        done
    fi
    say "what the stream saw"
    info="$(stream_of "${WS}")"
    note "wadd: $(jq -c '{viewers, transport, turnDown, warnings, lastActive}' <<< "${info}")"
    note "the shell: $(shell stream)"
    WEB_PORT="$(as_root "podman port wadstream-${WS} 3000/tcp" 2> /dev/null | sed 's/.*://' | head -1)"
    GUID="$(jq -r .link <<< "${info}" | sed -n 's|.*/ws-\([0-9a-z]*\)/$|\1|p')"
    stats="$(as_root "curl -s -m 5 http://127.0.0.1:${WEB_PORT}/ws-${GUID}/api/status" 2> /dev/null)"
    note "Selkies' status: ${stats:0:400}"
    as_root "journalctl -b --no-pager -o cat -u wadstream-${WS} | grep -aiE 'ice|candidate|relay|srflx|webrtc|connected|client' | tail -n 15" 2> /dev/null | indent
    call DELETE "/v1/wadspaces/${WS}/stream" && ok "stopped" || fail "stopping: ${CODE} ${BODY}"
    stream_finish "${t0}"
}

# ------------------------------------------------------------ the end

stream_finish() {
    say "the result ($(($(now) - $1)) s)"
    [[ -f "${VM}/stream-starved" ]] && note "(the laptop ran out of memory: the VM was quit, so the steps after it failed for that)"
    [[ ${FAILED} -eq 0 ]] && echo "all ok" || echo "${FAILED} failed"
    exit $((FAILED > 0))
}

# Everything this started goes, whatever happened: the quick tunnel, the
# TURN key, the VM and its layer (VM_KEEP=1: the VM stays, tunnel and all).
stream_cleanup() {
    local rc=$?
    trap - EXIT
    [[ -n "${S_WATCHDOG:-}" ]] && kill "${S_WATCHDOG}" 2> /dev/null
    if [[ "${VM_KEEP:-0}" == 1 ]] && running "${RUN}/qemu.pid"; then
        echo "kept (VM_KEEP=1): tools/vm.sh --name ${S_NAME} ssh. Its quick tunnel (${S_UNIT}) and /etc/wados/turn.toml are still there: stop them, then tools/vm.sh --name ${S_NAME} stop"
    else
        if running "${RUN}/qemu.pid"; then
            as_root "systemctl stop ${S_UNIT} 2> /dev/null; rm -f /etc/wados/turn.toml /etc/wados/turn.toml.new /run/wados-check-needles; systemctl is-active ${S_UNIT} 2> /dev/null; true" > /dev/null 2>&1
            stop > /dev/null
        fi
        rm -f "${RUN}/run.qcow2" "${RUN}/vars.fd"
        local gone=""
        [[ -n "${S_TUNNEL_ON:-}" && -n "${QHOST:-}" ]] && gone=" (${QHOST}: $(curl -s -m 10 -o /dev/null -w '%{http_code}' "https://${QHOST}/" 2> /dev/null))"
        echo "cleaned up: the quick tunnel stopped${gone}, turn.toml removed, the VM stopped and its layer thrown away"
    fi
    rm -rf "${S_PRIV}"
    # The mask's last lines, then the terminal's own.
    exec 1>&3 2>&4
    [[ -n "${S_MASK:-}" ]] && wait "${S_MASK}" 2> /dev/null
    exit "${rc}"
}
