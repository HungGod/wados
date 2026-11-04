# tools/vm.sh tailnet-check [--plain]: two WadOS machines on a test tailnet
# (phase 6), sourced by tools/vm.sh, whose helpers it uses. The tailnet is
# tools/tailnet.sh's: Headscale 0.29.7 on the laptop, never the real one.
# Every step says ok or FAIL, and the check goes on after a FAIL where it can.
# Screenshots: .build/vm/tailnet-shots/.
#
# The plain tailnet (Tailscale 1.104.5 on the host image, without wadd):
#   0. Headscale (TLS from a throwaway CA, its own DERP, a policy with
#      Taildrive for sam), and the device: a userspace tailscaled in its pod
#   1. A and B boot from fresh layers over wados.qcow2 (TAILNET_VM_MEM each)
#   2. each made ready: its name (wados-a, wados-b), hs.test and the test CA,
#      nothing but the laptop and the tailnet (nftables: no internet, no
#      Tailscale servers), sam (uid 1000), tailscaled sending no logs
#   3. A and B join as sam (user-owned, untagged): with a pre-auth key, or
#      A through wadd's login job (below)
#   4. the device joins as eve, interactively: `tailscale up --json` gives an
#      AuthURL, `headscale auth register` approves it, and it's Running
#   5. A and B see each other (status, ping through the DERP); WhoIs on each
#      says the other is sam, untagged; the device is eve's
#   6. Taildrive: A shares sam's home as sam; B, as sam, mounts, lists and
#      reads it with gio (dav://100.100.100.100:8080/<tailnet>/wados-a/home),
#      and writes a file A finds as sam's
# Through wadd (phase 6's API; not with --plain):
#   7. login: A's (fresh) `POST /v1/tailnet/login` job emits signIn with
#      Headscale's AuthURL; the check approves it; the job ends, Running
#   8. wadd shares sam's home itself once it's Running (step 6 then checks it)
#   9. peers: B's /v1/peers lists A; B lists A's wadspaces; Copy here (a
#      CopyWadspace job), the copy in B's library, and it builds
#  10. the device: only hello from A's :8087 (403 past it), and an ask in
#      A's /v1/tailnet; trusted on A, it lists A's wadspaces; B follows
#  11. GitHub, a mock only (tools/vm/github-mock.py on the laptop, [github]
#      web and api in wadd.toml): B's Github job emits signIn with the
#      device page and code; the check approves the code; the token is the
#      podman secret github_token, /v1/github says whose, the repos list
#  12. projects, on B: a wadspace with a folder and a file of B's, a folder
#      and a file of A's (over Taildrive) and the mock's repository. Its
#      open job has a stage for each; inside, as abc, B's folder and A's
#      (rclone's mount, wadtaildrive-*) are read and written, the copies
#      are there, the clone is uid 1000's; stopped, A's folder is unmounted;
#      /var/lib/wados, keys/ and projects/ stay 0711 across a wadd restart
#  13. signing out of GitHub deletes the secret
#  14. no SELinux denials on either, all along
#  15. A off: B's wadspace opens without A's folder, and says why
# Then everything goes: the VMs, the pod and its volume, the CA and keys,
# the mock and its repository.
#
# TAILNET_VM_MEM (MB, 1536: the session takes about 750 MB, and free page
# reporting gives back what Linux frees), VM_KEEP=1 (leave the VMs and
# Headscale running).

TN="${ROOT}/tools/tailnet.sh"
HS_URL=https://hs.test:8443

ok() { printf 'ok    %s\n' "$*"; }
fail() { printf 'FAIL  %s\n' "$*"; FAILED=$((FAILED + 1)); }
note() { printf '      %s\n' "$*"; }
indent() { sed 's/^/      | /'; }
step_time() { printf '      (%d s)\n' "$(($(now) - STEP_T))"; STEP_T=$(now); }

# wait_for SECONDS COMMAND…: until it succeeds.
wait_for() {
    local deadline=$(($(now) + $1))
    shift
    while (($(now) < deadline)); do
        "$@" && return 0
        sleep 2
    done
    return 1
}

# This VM, for vm.sh's own helpers (boot, stop, qmp): on a|b.
on() { VM_NAME="$1" RUN="${VM}/$1"; }
port_of() { cat "${VM}/$1/ssh.port"; }
root_on() { local n="$1"; shift; ssh_to "$(port_of "${n}")" root "$@"; }
sam_on() { local n="$1"; shift; ssh_to "$(port_of "${n}")" sam "$@"; }
qmp_on() { local n="$1"; shift; python3 "${HERE}/qmp.py" "${VM}/${n}/qmp.sock" "$@"; }
shell_on() { sam_on "$1" "XDG_RUNTIME_DIR=/run/user/1000 wadsctl ${*:2}" 2> /dev/null; }
ts_on() { local n="$1"; shift; root_on "${n}" tailscale "$@"; }
dev() { "${TN}" device "$@"; }

# wadd's API through its socket, as sam: api_on a|b METHOD PATH [BODY].
# Prints the body and succeeds on a 2xx; else prints "HTTP <code>: <body>".
api_on() {
    local n="$1" m="$2" p="$3" body="${4:-}" out code
    local curl="curl -sS --max-time 60 --unix-socket /run/wadd/wadd.sock -X ${m} -w '\n%{http_code}'"
    if [[ -n "${body}" ]]; then
        out="$(sam_on "${n}" "${curl} -H 'content-type: application/json' --data-binary @- http://wadd${p}" <<< "${body}" 2>&1)"
    else
        out="$(sam_on "${n}" "${curl} http://wadd${p}" < /dev/null 2>&1)"
    fi
    code="${out##*$'\n'}"
    out="${out%$'\n'*}"
    [[ "${out}" == "${code}" ]] && out=""
    if [[ "${code}" == 2?? ]]; then
        printf '%s' "${out}"
        return 0
    fi
    printf 'HTTP %s: %s' "${code}" "$(head -c 300 <<< "${out}")"
    return 1
}

shot_on() {
    mkdir -p "${SHOTS}"
    if qmp_on "$1" shot "${SHOTS}/$2" 2> /dev/null; then note "screenshot: ${SHOTS}/$2"; else note "(no screenshot: $2)"; fi
}

# A machine app in sam's session, as the shell starts it (a user unit), for
# a screenshot: app_shot a|b UNIT FILE ARGV…. Skipped when the image hasn't it.
app_shot() {
    local n="$1" unit="$2" file="$3"
    shift 3
    if ! root_on "${n}" "command -v $1" > /dev/null 2>&1; then
        note "(${n}: no $1 in this image: no ${file})"
        return
    fi
    sam_on "${n}" "XDG_RUNTIME_DIR=/run/user/1000 systemd-run --user --quiet --collect --unit ${unit} --setenv=WAYLAND_DISPLAY=wayland-0 $*" < /dev/null
    sleep 6
    shot_on "${n}" "${file}"
    local drawn
    drawn="$(python3 "${HERE}/screen.py" "${SHOTS}/${file}" 2>&1)" && note "(${file}: ${drawn})"
    sam_on "${n}" "XDG_RUNTIME_DIR=/run/user/1000 systemctl --user stop ${unit}" < /dev/null > /dev/null 2>&1
}

# Both VMs answer (a VM the laptop swapped out stops answering: everything
# after would only time out).
alive() {
    local n down=""
    if [[ -f "${VM}/tailnet-starved" ]]; then
        fail "the laptop ran out of memory and swap ($(cat "${VM}/tailnet-starved") MB left): the VMs were quit"
        return 1
    fi
    for n in "$@"; do
        root_on "${n}" true 2> /dev/null || root_on "${n}" true 2> /dev/null || down+=" ${n}"
    done
    [[ -z "${down}" ]] && return 0
    fail "no answer from${down} (the laptop: $(free -m | awk '/^Mem:/ { print $7 " MB available" } /^Swap:/ { print $4 " MB swap free" }' | tr '\n' ' '))"
    tail -c 600 "${VM}/${down# }/serial.log" 2> /dev/null | tr -d '\r' | indent
    return 1
}

journal_on() {
    root_on "$1" "journalctl -b --no-pager -o short-monotonic ${*:2} | tail -n 20" 2> /dev/null | indent
}

# A job, followed until it ends: prints its last JobInfo; true if done.
job_done() {
    local n="$1" id="$2" secs="${3:-180}" j st deadline
    deadline=$(($(now) + secs))
    while (($(now) < deadline)); do
        j="$(api_on "${n}" GET "/v1/jobs/${id}")" || { printf '%s' "${j}"; return 1; }
        st="$(jq -r '.job.status // .status' <<< "${j}" 2> /dev/null)"
        case "${st}" in
            done) printf '%s' "${j}"; return 0 ;;
            error | cancelled) printf '%s' "${j}"; return 1 ;;
        esac
        sleep 2
    done
    printf 'still %s after %d s: %s' "${st}" "${secs}" "${j}"
    return 1
}
job_summary() { jq -c '(.job // .) | {kind, status, title, error, stage: (.stages[(.cursor // 0)].label? // null)}' 2> /dev/null <<< "$1" || head -c 300 <<< "$1"; }

tailnet_check() {
    set +e
    FAILED=0
    PLAIN=""
    [[ "${1:-}" == --plain ]] && PLAIN=1
    MEM="${TAILNET_VM_MEM:-1536}"
    SHOTS="${VM}/tailnet-shots"
    DEVICE_UP_PID=""
    GH_PID=""
    WATCHDOG_PID=""
    GH_PORT=0
    mkdir -p "${SHOTS}"
    rm -f "${SHOTS}"/*.png
    local t0
    t0=$(now)
    STEP_T=$(now)
    trap 'tailnet_cleanup' EXIT
    trap 'exit 130' INT TERM

    say "0. the test tailnet: Headscale on the laptop, and the device"
    # A new one each run: a new CA, keys and database.
    "${TN}" stop > /dev/null
    if ! "${TN}" start; then
        fail "Headscale didn't start"
        tailnet_finish "${t0}"
    fi
    ok "Headscale is up at ${HS_URL} (the laptop's 127.0.0.1:${TAILNET_PORT:-8443}; 10.0.2.2 from the VMs)"
    if dev start; then ok "the device's tailscaled is up (userspace, in Headscale's pod, its own socket)"; else fail "the device's tailscaled"; fi
    # GitHub, a mock: the laptop's 127.0.0.1, 10.0.2.2 from the VMs.
    GH_PORT="$(free_port)"
    GH_LOG="${VM}/tailnet-github.log"
    rm -rf "${VM}/tailnet-git"
    python3 -I "${HERE}/github-mock.py" "${GH_PORT}" "${GH_LOG}" "${VM}/tailnet-git" &
    GH_PID=$!
    if wait_for 10 curl -sf -o /dev/null "http://127.0.0.1:${GH_PORT}/mock/state"; then
        ok "GitHub's mock is up (127.0.0.1:${GH_PORT}, pid ${GH_PID})"
    else
        fail "GitHub's mock didn't start"
    fi
    step_time

    say "1. A and B boot (fresh layers, ${MEM} MB each)"
    local n free_mb
    # Two VMs beside everything else on the laptop: a run that swaps the
    # VMs out stalls them (RCU stalls, SSH gone), and fails for no reason.
    free_mb="$(awk '/^MemAvailable:/ { a = $2 } /^SwapFree:/ { s = $2 } END { print int((a + s / 2) / 1024) }' /proc/meminfo)"
    if ((free_mb < 2 * MEM + 512)); then
        note "WARNING: the laptop has about ${free_mb} MB to spare (available, and half the free swap) for 2 × ${MEM} MB"
    fi
    for n in a b; do
        on "${n}"
        stop > /dev/null
        boot --fresh
    done
    # The laptop first: when it's nearly out of memory and swap, the VMs go
    # (they'd only stall) before anything else on it is killed for them.
    rm -f "${VM}/tailnet-starved"
    (
        while sleep 5; do
            left="$(awk '/^MemAvailable:/ { a = $2 } /^SwapFree:/ { s = $2 } END { print int((a + s) / 1024) }' /proc/meminfo)"
            if ((left < 300)); then
                echo "${left}" > "${VM}/tailnet-starved"
                for v in a b; do python3 "${HERE}/qmp.py" "${VM}/${v}/qmp.sock" quit 2> /dev/null; done
                exit 0
            fi
        done
    ) &
    WATCHDOG_PID=$!
    for n in a b; do
        if wait_ssh "$(port_of "${n}")" root 420 "${VM}/${n}/qemu.pid"; then
            ok "${n}: root's SSH after $(($(now) - STEP_T)) s"
        else
            fail "${n}: no SSH within 420 s (${VM}/${n}/serial.log)"
            tail -n 20 "${VM}/${n}/serial.log" | indent
            tailnet_finish "${t0}"
        fi
    done
    step_time

    say "2. each made ready: its name, hs.test, the test CA, the laptop only, sam"
    for n in a b; do tailnet_prepare "${n}"; done
    if [[ -z "${PLAIN}" ]]; then tailnet_sessions; fi
    step_time

    say "3. A and B join the tailnet as sam"
    SAM_KEY="$("${TN}" key sam)"
    if [[ -z "${PLAIN}" ]]; then
        wadd_login a
    else
        join_with_key a
    fi
    join_with_key b
    step_time

    say "4. the device joins as eve, interactively (an AuthURL, approved by headscale auth register)"
    device_join
    step_time

    say "5. A and B see each other; WhoIs says who's calling"
    alive a b && tailnet_see
    step_time

    say "6. Taildrive: sam's home on A, browsed from B as sam"
    alive a b && tailnet_drive
    step_time

    if [[ -z "${PLAIN}" ]]; then
        say "9. peers through wadd: B lists A and its wadspaces, and copies one"
        alive a b && wadd_peers
        step_time

        say "10. the device isn't trusted: hello only, an ask; trusted on A, B follows"
        alive a b && wadd_device
        step_time

        say "11. GitHub (a mock): B signs in, and the token is the podman secret"
        alive b && wadd_github b
        step_time

        say "12. projects on B: its folder and file, A's over Taildrive, the mock's repository"
        alive a b && wadd_projects
        step_time

        say "13. GitHub: signing out"
        alive b && wadd_github_out b
        step_time
    fi

    say "14. SELinux"
    for n in a b; do
        local denials
        # (No auditd in the image: the kernel's and journald's audit lines.)
        denials="$(root_on "${n}" "journalctl -b --no-pager -o cat | grep -a 'avc:  denied'; echo asked" 2> /dev/null)"
        if [[ "${denials}" != *asked ]]; then
            fail "${n}: couldn't read its journal"
        elif denials="${denials%asked}" && [[ -z "${denials//[[:space:]]/}" ]]; then
            ok "${n}: no SELinux denials this boot ($(root_on "${n}" getenforce 2> /dev/null))"
        else
            fail "${n}: $(wc -l <<< "${denials}") SELinux denials this boot"
            head -n 10 <<< "${denials}" | indent
        fi
    done
    step_time

    if [[ -z "${PLAIN}" && -n "${WSP:-}" ]]; then
        say "15. A off: B's wadspace opens without A's folder, and says why"
        alive b && wadd_offline
        step_time
    fi
    tailnet_finish "${t0}"
}

# ------------------------------------------------------------ 2. ready

tailnet_prepare() {
    local n="$1"
    # The installer's [machine] id (its peers know it by it), and where the
    # tailnet's login is: tailscaled's prefs have none on a fresh machine.
    if root_on "${n}" bash -s -- "wados-${n}" "$(base64 -w0 < "$("${TN}" ca)")" "${SAM_PASSWORD}" "${HS_URL}" "${GH_PORT}" << 'EOF'
set -e
hostnamectl set-hostname "$1"
grep -q ' hs.test$' /etc/hosts || echo '10.0.2.2 hs.test' >> /etc/hosts
echo "$2" | base64 -d > /etc/pki/ca-trust/source/anchors/wados-test-ca.crt
update-ca-trust
# Nothing but the laptop (QEMU's 10.0.2.0/24: Headscale, its DERP, DNS) and
# the tailnet: no internet, so no Tailscale server either, whatever asks.
nft -f - << 'NFT'
table inet wados_tailnet_check
delete table inet wados_tailnet_check
table inet wados_tailnet_check {
    chain out {
        type filter hook output priority 0; policy accept;
        oif "lo" accept
        oifname "tailscale0" accept
        ip daddr { 10.0.2.0/24, 100.64.0.0/10, 127.0.0.0/8 } accept
        ip6 daddr { fd7a:115c:a1e0::/48, ::1 } accept
        counter reject
    }
}
NFT
# tailscaled: no logs to Tailscale.
sed -i 's/^FLAGS=.*/FLAGS="--no-logs-no-support"/' /etc/default/tailscaled
systemctl restart tailscaled
# sam, as tools/vm/check.sh makes them (the installer's, phase 9).
groups=wheel
getent group wados > /dev/null && groups+=,wados
getent passwd 1000 > /dev/null || useradd -m -u 1000 -U -G "${groups}" -c Sam sam
echo "sam:$3" | chpasswd
install -d -m 0700 -o sam -g sam ~sam/.ssh
install -m 0600 -o sam -g sam /root/.ssh/authorized_keys ~sam/.ssh/authorized_keys
mkdir -p /etc/wados
cat > /etc/wados/wadd.toml << TOML
# tools/vm/check-tailnet.sh's (the installer's, on a real machine).
[machine]
id = "$1"

[tailnet]
login_server = "$4"

# GitHub: the laptop's mock (tools/vm/github-mock.py), never GitHub.
[github]
web = "http://10.0.2.2:$5"
api = "http://10.0.2.2:$5/api"
TOML
systemctl restart wadd
EOF
    then
        ok "${n}: wados-${n}, hs.test and the test CA, no internet, sam, wadd's [machine] id, GitHub's mock"
    else
        fail "${n}: making it ready"
    fi
    local health
    health="$(root_on "${n}" "curl -sS -o /dev/null -w '%{http_code}' ${HS_URL}/health; echo; curl -sS -m 5 -o /dev/null https://controlplane.tailscale.com 2>&1 | head -1" 2>&1)"
    if [[ "${health}" == 200* ]]; then
        ok "${n}: Headscale answers over TLS with the test CA; the real control plane doesn't (${health#200$'\n'})"
    else
        fail "${n}: Headscale's /health: ${health}"
    fi
}

# Sam's session on both (the shell's sign-in, trust dialog, Files): greetd
# logs them in, locked; unlocked with their password.
tailnet_sessions() {
    local n
    for n in a b; do root_on "${n}" 'systemctl reset-failed greetd 2> /dev/null; systemctl restart greetd' > /dev/null 2>&1; done
    for n in a b; do
        if wait_for 240 eval '[[ "$(shell_on '"${n}"' locked)" == "ok true" ]]'; then
            qmp_on "${n}" keys "${SAM_PASSWORD}" && qmp_on "${n}" key ret
            if wait_for 30 eval '[[ "$(shell_on '"${n}"' locked)" == "ok false" ]]'; then
                ok "${n}: sam's session is up and unlocked"
            else
                fail "${n}: still locked after sam's password"
            fi
        else
            fail "${n}: sam's session didn't come up locked ($(shell_on "${n}" locked))"
        fi
    done
}

# The present signIn of a provider (tailscale, github), from wadd's events:
# "<url> <code>". Read in the VM, which stops at the first (curl goes at
# its next keep-alive).
sign_in_on() {
    sam_on "$1" bash -s -- "$2" << 'EOF' 2> /dev/null
timeout 90 curl -sSN --unix-socket /run/wadd/wadd.sock http://wadd/v1/events | while IFS= read -r line; do
    [[ "${line}" == "data: "* ]] || continue
    u="$(jq -r --arg p "$1" 'select(.kind == "signIn" and .provider == $p) | "\(.url) \(.code // "")"' <<< "${line#data: }" 2> /dev/null)"
    [[ -n "${u}" ]] && { echo "${u}"; exit 0; }
done
EOF
}

# ------------------------------------------------------------ 3. joining

join_with_key() {
    local n="$1" out
    out="$(ts_on "${n}" up --login-server="${HS_URL}" --auth-key="${SAM_KEY}" --hostname="wados-${n}" --timeout=90s 2>&1)"
    if [[ "$(ts_on "${n}" status --json 2> /dev/null | jq -r .BackendState)" == Running ]]; then
        ok "${n}: joined with a pre-auth key for sam ($(ts_on "${n}" ip -4 2> /dev/null))"
    else
        fail "${n}: tailscale up with a pre-auth key: ${out}"
    fi
}

# 7. A fresh machine's login through wadd: the job's signIn has Headscale's
# AuthURL, the check approves it (as the tailnet's admin would), and the
# job ends with the tailnet Running. Without it, A joins with a key.
wadd_login() {
    local n="$1" job id url auth j st tn
    say "7. login through wadd: ${n}'s Tailnet job, its signIn, approved"
    tn="$(api_on "${n}" GET /v1/tailnet)"
    note "before: $(jq -c '{state, tailnet, hostname}' <<< "${tn}" 2> /dev/null || echo "${tn}")"
    if ! job="$(api_on "${n}" POST /v1/tailnet/login '{}')"; then
        fail "${n}: POST /v1/tailnet/login: ${job}"
        note "(A joins with a pre-auth key instead)"
        join_with_key "${n}"
        return
    fi
    id="$(jq -r '.id // .job.id' <<< "${job}")"
    ok "${n}: a Tailnet job ($(job_summary "${job}"))"
    # The signIn event (the present, to a watcher that comes after it).
    url="$(sign_in_on "${n}" tailscale)"
    url="${url%% *}"
    if [[ "${url}" == "${HS_URL}/register/"* ]]; then
        ok "${n}: signIn {provider: tailscale} with Headscale's AuthURL (${url})"
    else
        fail "${n}: no signIn with Headscale's AuthURL within 90 s (${url:-none}; job: $(job_summary "$(api_on "${n}" GET "/v1/jobs/${id}")"))"
        journal_on "${n}" -u wadd -u tailscaled
        note "(A joins with a pre-auth key instead)"
        api_on "${n}" POST "/v1/jobs/${id}/cancel" '{}' > /dev/null
        join_with_key "${n}"
        return
    fi
    sleep 5
    shot_on "${n}" "${n}-7-signin.png"
    auth="${url##*/register/}"
    if "${TN}" approve "${auth}" sam > /dev/null; then ok "approved: headscale auth register --auth-id ${auth} --user sam"; else fail "headscale auth register ${auth}"; fi
    if j="$(job_done "${n}" "${id}" 120)"; then
        ok "${n}: the login job is done ($(job_summary "${j}"))"
    else
        fail "${n}: the login job: $(job_summary "${j}")"
        journal_on "${n}" -u wadd
    fi
    tn="$(api_on "${n}" GET /v1/tailnet)"
    st="$(jq -r .state <<< "${tn}" 2> /dev/null)"
    if [[ "${st}" == running && "$(jq -r .tailnet <<< "${tn}")" == hs.test && "$(jq -r .ip <<< "${tn}")" == 100.* ]]; then
        ok "${n}: /v1/tailnet says running ($(jq -c '{state, tailnet, hostname, ip}' <<< "${tn}"))"
    else
        fail "${n}: /v1/tailnet after the login: ${tn}"
    fi
    if [[ "$(ts_on "${n}" status --json 2> /dev/null | jq -r .BackendState)" != Running ]]; then
        note "(tailscaled isn't Running: A joins with a pre-auth key instead)"
        join_with_key "${n}"
    fi
    sleep 2
    shot_on "${n}" "${n}-7-signed-in.png"
}

# ------------------------------------------------------------ 4. the device

device_join() {
    local out="${SHOTS}/device-up.json" url auth st=""
    : > "${out}"
    dev ts up --json --login-server="${HS_URL}" --hostname=eve-phone --timeout=120s > "${out}" 2>&1 &
    DEVICE_UP_PID=$!
    for _ in $(seq 30); do
        url="$(grep -o '"AuthURL": *"[^"]*"' "${out}" | head -1 | sed 's/.*: *"//; s/"$//')"
        [[ -n "${url}" ]] && break
        sleep 1
    done
    if [[ "${url}" == "${HS_URL}/register/hskey-authreq-"* ]]; then
        ok "tailscale up --json gave an AuthURL (${url})"
    else
        fail "no AuthURL from the device's tailscale up --json: $(head -c 300 "${out}")"
    fi
    # Opening it approves nothing: the admin does.
    if [[ -n "${url}" ]]; then
        local page
        page="$(dev curl -s -m 10 --noproxy '*' --cacert /etc/wados-test-ca.crt "${url}" 2> /dev/null | grep -o 'headscale auth register[^<]*' | head -1)"
        note "its page says: ${page:-?}"
        auth="${url##*/register/}"
        if "${TN}" approve "${auth}" eve > /dev/null; then ok "approved for eve (headscale auth register --auth-id ${auth} --user eve)"; else fail "headscale auth register for eve"; fi
    fi
    for _ in $(seq 30); do
        st="$(dev ts status --json 2> /dev/null | jq -r .BackendState)"
        [[ "${st}" == Running ]] && break
        sleep 1
    done
    wait "${DEVICE_UP_PID}" 2> /dev/null
    DEVICE_UP_PID=""
    if [[ "${st}" == Running ]]; then
        DEVICE_IP="$(dev ts ip -4 2> /dev/null)"
        DEVICE_ID="$(dev ts status --json | jq -r .Self.ID)"
        ok "the device is Running as eve (${DEVICE_IP}, node ${DEVICE_ID})"
    else
        fail "the device: ${st:-no status}"
    fi
}

# ------------------------------------------------------------ 5. seeing each other

tailnet_see() {
    local me other s ip uid who tags pong
    A_IP="$(ts_on a ip -4 2> /dev/null)"
    B_IP="$(ts_on b ip -4 2> /dev/null)"
    A_ID="$(ts_on a status --json 2> /dev/null | jq -r .Self.ID)"
    TAILNET_NAME="$(ts_on b status --json 2> /dev/null | jq -r .CurrentTailnet.Name)"
    note "A ${A_IP} (node ${A_ID}), B ${B_IP}; the tailnet: ${TAILNET_NAME} (CurrentTailnet.Name)"
    for me in a b; do
        other=b ip="${B_IP}"
        [[ "${me}" == b ]] && other=a ip="${A_IP}"
        s="$(ts_on "${me}" status --json 2> /dev/null)"
        if jq -e --arg h "wados-${other}" '[.Peer[] | select(.HostName == $h and .Online)] | length == 1' <<< "${s}" > /dev/null 2>&1; then
            ok "${me}: tailscale status has wados-${other}, online"
        else
            fail "${me}: wados-${other} isn't online in tailscale status"
            ts_on "${me}" status 2>&1 | indent
        fi
        pong="$(ts_on "${me}" ping -c 5 --until-direct=false --timeout 10s "${ip}" 2>&1 | grep -m1 '^pong')"
        if [[ -n "${pong}" ]]; then
            ok "${me}: ${pong}$([[ "${pong}" == *DERP* ]] && echo ' (relayed)')"
        else
            fail "${me}: no pong from ${ip}"
        fi
        # MagicDNS: what peers.rs calls a peer by (http://<dnsName>:8087).
        local resolved
        resolved="$(root_on "${me}" "getent ahostsv4 wados-${other}.ts.test | awk 'NR == 1 { print \$1 }'" 2> /dev/null)"
        if [[ "${resolved}" == "${ip}" ]]; then ok "${me}: wados-${other}.ts.test is ${ip} (MagicDNS, through the system's resolver)"; else fail "${me}: wados-${other}.ts.test resolves to '${resolved}', not ${ip}"; fi
        # WhoIs: the other is the same Tailscale user, and not tagged.
        uid="$(jq -r .Self.UserID <<< "${s}")"
        who="$(ts_on "${me}" whois --json "${ip}" 2> /dev/null)"
        tags="$(jq -c '.Node.Tags // []' <<< "${who}" 2> /dev/null)"
        if [[ "$(jq -r .UserProfile.ID <<< "${who}" 2> /dev/null)" == "${uid}" && "${tags}" == "[]" ]]; then
            ok "${me}: WhoIs ${ip}: $(jq -r .UserProfile.LoginName <<< "${who}") (user ${uid}, as this machine's login), untagged, node $(jq -r .Node.StableID <<< "${who}")"
        else
            fail "${me}: WhoIs ${ip}: $(jq -c '{user: .UserProfile, tags: .Node.Tags}' <<< "${who}" 2> /dev/null || echo "${who}") (this machine's user: ${uid})"
        fi
    done
    if [[ -n "${DEVICE_IP:-}" ]]; then
        who="$(ts_on a whois --json "${DEVICE_IP}" 2> /dev/null)"
        local login
        login="$(jq -r .UserProfile.LoginName <<< "${who}" 2> /dev/null)"
        tags="$(jq -c '.Node.Tags // []' <<< "${who}" 2> /dev/null)"
        if [[ -n "${login}" && ( "${login}" != sam || "${tags}" != "[]" ) ]]; then
            ok "a: WhoIs the device: ${login}, tags ${tags}: not one of sam's machines"
        else
            fail "a: WhoIs the device: $(jq -c '{user: .UserProfile, tags: .Node.Tags}' <<< "${who}" 2> /dev/null || echo "${who}")"
        fi
        pong="$(dev ts ping -c 5 --until-direct=false --timeout 10s "${A_IP}" 2>&1 | grep -m1 '^pong')"
        if [[ -n "${pong}" ]]; then ok "the device: ${pong}"; else fail "the device: no pong from A"; fi
    fi
    bound_to_tailnet
}

# What wadd's remote API does (remote.rs), without wadd: a server on A's
# tailnet address bound to tailscale0 (SO_BINDTODEVICE), on 8099 (not
# wadd's 8087). B and the device reach it over the tailnet; A's own
# loopback doesn't (that traffic isn't tailscale0's).
bound_to_tailnet() {
    root_on a "systemd-run --quiet --collect --unit tailnet-bound python3 -c '
import socket, http.server
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        b = (\"bound to tailscale0, from \" + self.client_address[0]).encode()
        self.send_response(200); self.send_header(\"content-length\", str(len(b))); self.end_headers(); self.wfile.write(b)
    def log_message(self, *a): pass
class S(http.server.HTTPServer):
    def server_bind(self):
        self.socket.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.socket.setsockopt(socket.SOL_SOCKET, socket.SO_BINDTODEVICE, b\"tailscale0\")
        super().server_bind()
S((\"${A_IP}\", 8099), H).serve_forever()
'" 2>&1 | indent
    sleep 2
    local from_b from_dev local_
    from_b="$(root_on b "curl -sS -m 10 http://${A_IP}:8099/" 2>&1)"
    from_dev="$(dev curl -sS -m 15 "http://${A_IP}:8099/" 2>&1)"
    local_="$(root_on a "curl -sS -m 5 -o /dev/null -w '%{http_code}' http://${A_IP}:8099/" 2>&1)"
    if [[ "${from_b}" == "bound to tailscale0, from ${B_IP}" && "${from_dev}" == "bound to tailscale0, from ${DEVICE_IP:-?}" ]]; then
        ok "a: a server bound to tailscale0 (SO_BINDTODEVICE) answers B and the device over the tailnet (${from_b}; ${from_dev})"
    else
        fail "a: the tailscale0-bound server: from B '${from_b}', from the device '${from_dev}'"
        root_on a "journalctl -b --no-pager -u tailnet-bound | tail -n 5" 2>&1 | indent
    fi
    if [[ "${local_}" != 200 ]]; then ok "a: and A's own loopback doesn't reach it (${local_//$'\n'/ })"; else note "a: A itself reaches it too (through lo)"; fi
    root_on a "systemctl stop tailnet-bound" > /dev/null 2>&1
}

# ------------------------------------------------------------ 6. Taildrive

tailnet_drive() {
    local list share stamp url got back procs
    stamp="from A, $(date +%s)"
    sam_on a "printf '%s\n' '${stamp}' > ~/taildrive-check.txt" < /dev/null
    if [[ -z "${PLAIN}" ]]; then
        # 8. wadd shares the person's home itself, as them, once it's Running.
        if wait_for 60 eval '[[ "$(root_on a "tailscale drive list" 2> /dev/null | awk "\$1 == \"home\" { print \$3 }")" == sam ]]'; then
            ok "8. a: wadd shared sam's home as sam ($(root_on a 'tailscale drive list' | awk '$1 == "home"'))"
        else
            fail "8. a: wadd didn't share sam's home within 60 s ($(root_on a 'tailscale drive list' 2>&1 | tr '\n' ' '))"
            note "(shared by the check instead)"
        fi
        local tn
        tn="$(api_on a GET /v1/tailnet)"
        if [[ "$(jq -r .drive <<< "${tn}" 2> /dev/null)" == shared ]]; then
            ok "8. a: /v1/tailnet says drive: shared"
        else
            fail "8. a: /v1/tailnet's drive: $(jq -c '{drive, driveError}' <<< "${tn}" 2> /dev/null || echo "${tn}")"
        fi
    fi
    if ! root_on a "tailscale drive list" 2> /dev/null | grep -q '^home '; then
        share="$(root_on a 'runuser -u sam -- tailscale drive share home /var/home/sam' 2>&1)"
        note "a, as sam: tailscale drive share home /var/home/sam: ${share}"
    fi
    list="$(root_on a 'tailscale drive list' 2>&1)"
    if [[ "$(awk '$1 == "home" { print $2, $3 }' <<< "${list}")" == "/var/home/sam sam" ]]; then
        ok "a: sam's home is shared as sam (tailscale drive list: home /var/home/sam sam)"
    else
        fail "a: tailscale drive list: ${list}"
    fi
    url="dav://100.100.100.100:8080/${TAILNET_NAME:-hs.test}/wados-a/home"
    # gio needs the place mounted first (Files does it with mount_enclosing_volume).
    got="$(sam_on b "export XDG_RUNTIME_DIR=/run/user/1000; timeout 60 gio mount '${url}' < /dev/null 2>&1; timeout 60 gio list '${url}' 2>&1" 2>&1)"
    if grep -qx 'taildrive-check.txt' <<< "${got}"; then
        ok "b, as sam: gio list ${url} has taildrive-check.txt ($(tr '\n' ' ' <<< "${got}"))"
    else
        fail "b, as sam: gio list ${url}: ${got}"
        root_on b "curl -s -X PROPFIND -H 'Depth: 1' http://100.100.100.100:8080/${TAILNET_NAME:-hs.test}/ | grep -o '<D:href>[^<]*</D:href>'" 2>&1 | indent
    fi
    got="$(sam_on b "XDG_RUNTIME_DIR=/run/user/1000 timeout 30 gio cat '${url}/taildrive-check.txt'" 2>&1)"
    if [[ "${got}" == "${stamp}" ]]; then ok "b, as sam: gio cat reads A's file (${got})"; else fail "b: gio cat: ${got}"; fi
    sam_on b "echo 'from B' | XDG_RUNTIME_DIR=/run/user/1000 timeout 30 gio save '${url}/from-b.txt'" 2>&1 | indent
    back="$(root_on a 'stat -c "%U %a" /var/home/sam/from-b.txt && cat /var/home/sam/from-b.txt' 2>&1 | tr '\n' ' ')"
    if [[ "${back}" == "sam "*"from B "* ]]; then ok "b wrote from-b.txt; on A it's sam's (${back})"; else fail "b's file on A: ${back}"; fi
    procs="$(root_on a "ps -o user=,label=,args= -C sudo,tailscaled | grep serve-taildrive" 2>&1)"
    note "a serves it with: $(tr '\n' ';' <<< "${procs}")"
    # Files ▸ Machines on B (the Machines tile opens it so).
    [[ -z "${PLAIN}" ]] && app_shot b files-machines b-6-files-machines.png wados-files --machines
}

# ------------------------------------------------------------ 9. peers

wadd_peers() {
    local peers peer ws doc copy job j lib b built st
    if wait_for 60 eval 'api_on b GET /v1/peers > /dev/null && [[ "$(api_on b GET /v1/peers | jq -r --arg n "${A_ID}" "[.[] | select(.nodeId == \$n and .online)] | length" 2> /dev/null)" == 1 ]]'; then
        peers="$(api_on b GET /v1/peers)"
        peer="$(jq -r --arg n "${A_ID}" '.[] | select(.nodeId == $n) | .id' <<< "${peers}")"
        ok "b: /v1/peers lists A ($(jq -c --arg n "${A_ID}" '.[] | select(.nodeId == $n) | {id, name, hostname, online, dnsName, nodeId, trusted}' <<< "${peers}"))"
        [[ "${peer}" == wados-a ]] || fail "b: A's peer id is ${peer}, not its [machine] id wados-a"
    else
        fail "b: /v1/peers hasn't A within 60 s: $(api_on b GET /v1/peers)"
        journal_on b -u wadd
        peer=wados-a
    fi
    # A's side: B is one of its machines.
    local tn
    tn="$(api_on a GET /v1/tailnet)"
    if jq -e --arg ip "${B_IP}" '[.devices[] | select(.mine and .trusted)] | length >= 1' <<< "${tn}" > /dev/null 2>&1; then
        ok "a: /v1/tailnet has B as mine and trusted ($(jq -c '[.devices[] | select(.mine) | {id, name, online}]' <<< "${tn}"))"
    else
        fail "a: /v1/tailnet hasn't B as mine: ${tn}"
    fi
    # A classes B as its machine once its own background probe of B's hello
    # has answered (it retries every 15 s).
    local hello
    wait_for 45 eval '[[ "$(root_on b "curl -sS -m 10 http://wados-a.ts.test:8087/v1/remote/hello" 2> /dev/null | jq -r .trust 2> /dev/null)" == machine ]]'
    hello="$(root_on b "curl -sS -m 10 http://wados-a.ts.test:8087/v1/remote/hello" 2>&1)"
    if [[ "$(jq -r '.wadd, .trust' <<< "${hello}" 2> /dev/null | tr '\n' ' ')" == "true machine " ]]; then
        ok "b → a's :8087 hello: a wadd, and B is its machine ($(jq -c .machine <<< "${hello}"))"
    else
        fail "b → a's :8087 hello: ${hello}"
    fi

    # The machine itself reaches its tailscale0-bound port (through lo, from
    # its own tailnet address: step 5): wadd refuses it as this machine.
    local self_code
    self_code="$(root_on a "curl -s -m 10 -o /dev/null -w '%{http_code}' http://${A_IP}:8087/v1/remote/machine" 2>&1)"
    if [[ "${self_code}" == 403 ]]; then ok "a → its own :8087 (from ${A_IP}, through lo): 403, not a remote caller"; else fail "a → its own :8087: ${self_code}"; fi

    # A wadspace on A, for B to see and copy (as tools/vm/check.sh makes one).
    WS="$(api_on a POST /v1/ops '{"op": {"op": "space.create", "name": "Tailnet Terminal"}}' | jq -r .created 2> /dev/null)"
    if [[ -n "${WS}" && "${WS}" != null ]]; then
        api_on a POST /v1/ops "{\"op\": {\"op\": \"apps.add\", \"ws\": \"${WS}\", \"app\": \"terminal\"}}" > /dev/null
        ok "a: made ${WS} (Tailnet Terminal, with the terminal)"
    else
        fail "a: no wadspace made"
        return
    fi
    ws="$(api_on b GET "/v1/peers/${peer}/wadspaces")"
    if jq -e --arg id "${WS}" '[.[] | select(.id == $id)] | length == 1' <<< "${ws}" > /dev/null 2>&1; then
        ok "b: /v1/peers/${peer}/wadspaces has it ($(jq -c '[.[] | {id, name}]' <<< "${ws}"))"
    else
        fail "b: /v1/peers/${peer}/wadspaces: ${ws}"
    fi

    # Copy here.
    if ! job="$(api_on b POST "/v1/peers/${peer}/wadspaces/${WS}/copy" '{}')"; then
        fail "b: copy: ${job}"
        return
    fi
    ok "b: a CopyWadspace job ($(job_summary "${job}"))"
    if j="$(job_done b "$(jq -r '.id // .job.id' <<< "${job}")" 120)"; then
        ok "b: the copy is done ($(job_summary "${j}"))"
    else
        fail "b: the copy: $(job_summary "${j}")"
        journal_on b -u wadd
    fi
    lib="$(api_on b GET /v1/wadspaces)"
    # B's own (its machine id on it): a copy may keep A's id, since ids are per machine.
    local me
    me="$(api_on b GET /v1/machine | jq -r '.id // empty' 2> /dev/null)"
    copy="$(jq -r --arg me "${me}" '[.[] | select(.name == "Tailnet Terminal" and .machine == $me)][0].id // empty' <<< "${lib}")"
    if [[ -n "${copy}" ]]; then
        ok "b: the copy is in its library as ${copy} ($(jq -c --arg id "${copy}" '.[] | select(.id == $id) | {id, name}' <<< "${lib}" 2> /dev/null))"
    else
        fail "b: no copy of Tailnet Terminal in /v1/wadspaces: $(jq -c '[.[] | {id, name}]' <<< "${lib}" 2> /dev/null || echo "${lib}")"
        return
    fi
    b="$(api_on b POST "/v1/wadspaces/${copy}/build")"
    if [[ "$(jq -r .job <<< "${b}")" != null && -n "$(jq -r '.job // empty' <<< "${b}")" ]]; then
        j="$(job_done b "$(jq -r '.job.id // .job' <<< "${b}")" 600)"
        note "its build job: $(job_summary "${j}")"
        b="$(api_on b GET /v1/builds | jq -c --arg id "${copy}" '.[] | select(.id == $id)')"
    fi
    built="$(jq -r '.state.built // .built' <<< "${b}" 2> /dev/null)"
    if [[ "${built}" == built ]]; then ok "b: the copy builds (${copy}: built)"; else fail "b: building the copy: ${b}"; fi
    st="$(api_on a GET "/v1/wadspaces/${WS}" | jq -r .name 2> /dev/null)"
    [[ "${st}" == "Tailnet Terminal" ]] && note "A's own is still there (${WS})"
    shot_on b b-9-copied.png
}

# ------------------------------------------------------------ 10. the device

# The device's call to A's remote API, through its SOCKS5 proxy: the HTTP
# code, then the body.
device_get() {
    dev curl -sS -m 20 -w '\n%{http_code}' "http://${A_IP}:8087$1" 2>&1
}
code_of() { printf '%s' "${1##*$'\n'}"; }
body_of() { printf '%s' "${1%$'\n'*}"; }

wadd_device() {
    [[ -n "${DEVICE_ID:-}" ]] || { fail "no device on the tailnet"; return; }
    local r tn p
    r="$(device_get /v1/remote/hello)"
    if [[ "$(code_of "${r}")" == 200 && "$(body_of "${r}" | jq -r '.wadd, .trust' 2> /dev/null | tr '\n' ' ')" == "true asked " ]]; then
        ok "the device → a's hello: 200, a wadd, trust: asked"
    else
        fail "the device → a's hello: $(tr '\n' ' ' <<< "${r}")"
    fi
    for p in /v1/remote/machine /v1/remote/wadspaces "/v1/remote/wadspaces/${WS:-x}" /v1/remote/trust; do
        r="$(device_get "${p}")"
        if [[ "$(code_of "${r}")" == 403 ]]; then ok "the device → a's ${p}: 403"; else fail "the device → a's ${p}: $(tr '\n' ' ' <<< "${r}")"; fi
    done
    if wait_for 30 eval '[[ "$(api_on a GET /v1/tailnet | jq -r --arg d "${DEVICE_ID}" "[.asks[]? | select(.id == \$d)] | length" 2> /dev/null)" == 1 ]]'; then
        ok "a: /v1/tailnet asks for the device ($(api_on a GET /v1/tailnet | jq -c --arg d "${DEVICE_ID}" '.asks[] | select(.id == $d)'))"
    else
        fail "a: no ask for the device (node ${DEVICE_ID}) in /v1/tailnet: $(api_on a GET /v1/tailnet | jq -c .asks 2> /dev/null)"
    fi
    sleep 3
    shot_on a a-10-trust-ask.png
    # Trusted on A: it may list A's wadspaces, and B follows the answer.
    if tn="$(api_on a PUT "/v1/tailnet/devices/${DEVICE_ID}" '{}')"; then
        ok "a: trusted the device (PUT /v1/tailnet/devices/${DEVICE_ID})"
    else
        fail "a: PUT /v1/tailnet/devices/${DEVICE_ID}: ${tn}"
    fi
    r="$(device_get /v1/remote/hello)"
    [[ "$(body_of "${r}" | jq -r .trust 2> /dev/null)" == trusted ]] && ok "the device → a's hello: trust: trusted" || fail "the device → a's hello after the trust: $(tr '\n' ' ' <<< "${r}")"
    r="$(device_get /v1/remote/wadspaces)"
    if [[ "$(code_of "${r}")" == 200 ]] && body_of "${r}" | jq -e --arg id "${WS:-}" '[.[] | select(.id == $id)] | length == 1' > /dev/null 2>&1; then
        ok "the device → a's /v1/remote/wadspaces: 200, with ${WS}"
    else
        fail "the device → a's /v1/remote/wadspaces after the trust: $(tr '\n' ' ' <<< "${r}" | head -c 300)"
    fi
    r="$(device_get /v1/remote/trust)"
    [[ "$(code_of "${r}")" == 403 ]] && ok "the device → a's /v1/remote/trust: still 403 (my machines only)" || fail "the device → a's /v1/remote/trust: $(code_of "${r}")"
    if wait_for 60 eval '[[ "$(api_on b GET /v1/tailnet | jq -r --arg d "${DEVICE_ID}" ".devices[]? | select(.id == \$d) | .trusted" 2> /dev/null)" == true ]]'; then
        ok "b follows: its /v1/tailnet has the device trusted ($(api_on b GET /v1/tailnet | jq -c --arg d "${DEVICE_ID}" '.devices[] | select(.id == $d)'))"
    else
        fail "b doesn't follow A's answer within 60 s: $(api_on b GET /v1/tailnet | jq -c .devices 2> /dev/null)"
    fi
    shot_on a a-10-trusted.png
    # Settings ▸ Accounts on B: the device among the trusted.
    app_shot b settings-accounts b-10-settings-accounts.png wados-settings --page accounts
}

# ------------------------------------------------------------ 11. GitHub

mock() { curl -sS -m 10 "http://127.0.0.1:${GH_PORT}$1" "${@:2}"; }
secret_on() { root_on "$1" "podman secret inspect --showsecret --format '{{.SecretData}}' github_token" 2> /dev/null; }

wadd_github() {
    local n="$1" gh job id got url code j token acct repos
    gh="$(api_on "${n}" GET /v1/github)"
    note "before: ${gh}"
    if ! job="$(api_on "${n}" POST /v1/github/signin '{}')"; then
        fail "${n}: POST /v1/github/signin: ${job}"
        return
    fi
    id="$(jq -r '.id // .job.id' <<< "${job}")"
    if [[ "$(jq -r .kind <<< "${job}")" == github ]]; then ok "${n}: a Github job ($(job_summary "${job}"))"; else fail "${n}: the sign-in's job: $(job_summary "${job}")"; fi
    got="$(sign_in_on "${n}" github)"
    url="${got%% *}" code="${got#* }"
    [[ "${got}" == *" "* ]] || code=""
    if [[ "${url}" == "http://10.0.2.2:${GH_PORT}/login/device" && "${code}" == "$(mock /mock/state | jq -r .user_code)" ]]; then
        ok "${n}: signIn {provider: github} with the mock's device page and its code (${url}, ${code})"
    else
        fail "${n}: no signIn with the mock's page and code within 90 s (${got:-none}; job: $(job_summary "$(api_on "${n}" GET "/v1/jobs/${id}")"))"
        journal_on "${n}" -u wadd
        api_on "${n}" POST "/v1/jobs/${id}/cancel" '{}' > /dev/null
        return
    fi
    sleep 4
    shot_on "${n}" "${n}-11-github-signin.png"
    # The person types the code on GitHub's page (here, the check tells the mock).
    if [[ "$(mock /mock/approve -X POST --data-urlencode "user_code=${code}" | jq -r .approved)" == true ]]; then ok "the code is approved (the mock)"; else fail "the mock didn't take ${code}"; fi
    if j="$(job_done "${n}" "${id}" 60)"; then
        ok "${n}: the sign-in is done ($(job_summary "${j}"))"
    else
        fail "${n}: the sign-in: $(job_summary "${j}")"
        journal_on "${n}" -u wadd
    fi
    token="$(mock /mock/state | jq -r '.token // empty')"
    # What wadd asked of the mock: the app's client ID, its scopes, the
    # device grant, slow_down kept, and the token used for /user.
    local asked polls gap
    asked="$(jq -c 'select(.path == "/login/device/code") | .form' "${GH_LOG}" | tail -1)"
    if [[ "$(jq -r .client_id <<< "${asked}")" == "$(root_on "${n}" "sed -n 's/^client_id = \"\(.*\)\"/\1/p' /usr/lib/wados/github.toml")" && "$(jq -r .scope <<< "${asked}")" == "repo workflow read:org" ]]; then
        ok "${n}: it asked with github.toml's client ID and the first sign-in's scopes (${asked})"
    else
        fail "${n}: the device code was asked with ${asked:-nothing}"
    fi
    polls="$(jq -r 'select(.path == "/login/oauth/access_token") | .at' "${GH_LOG}")"
    gap="$(awk 'NR == 1 { a = $1 } NR == 2 { printf "%.1f", $1 - a }' <<< "${polls}")"
    note "it polled $(wc -l <<< "${polls}") times; after slow_down (interval 2 s) the next came ${gap:-?} s later"
    if [[ -n "${gap}" ]] && awk -v g="${gap}" 'BEGIN { exit !(g >= 1.9) }'; then ok "${n}: slow_down was kept (${gap} s ≥ 2 s)"; else fail "${n}: slow_down wasn't kept (the next poll ${gap:-never} s after)"; fi
    if [[ -n "${token}" && "$(secret_on "${n}")" == "${token}" ]]; then
        ok "${n}: the token is root's podman secret github_token"
    else
        fail "${n}: github_token: $(root_on "${n}" "podman secret ls --format '{{.Name}}'" 2>&1 | tr '\n' ' ')"
    fi
    # Never in a job, an event's log or wadd's journal.
    if [[ -n "${token}" ]]; then
        if api_on "${n}" GET "/v1/jobs/${id}" | grep -qF "${token}" || root_on "${n}" "journalctl -b --no-pager -u wadd | grep -qF '${token}'"; then
            fail "${n}: the token is in the job or wadd's journal"
        else
            ok "${n}: the token isn't in the job or wadd's journal"
        fi
    fi
    acct="$(api_on "${n}" GET /v1/github)"
    if [[ "$(jq -c '[.login, .name, .scopes]' <<< "${acct}" 2> /dev/null)" == '["sam-wados","Sam Wados",["repo","workflow","read:org"]]' ]]; then
        ok "${n}: /v1/github: ${acct}"
    else
        fail "${n}: /v1/github: ${acct}"
    fi
    repos="$(api_on "${n}" GET /v1/github/repos)"
    if [[ "$(jq -c '[.[] | (.fullName // .full_name // .name)]' <<< "${repos}" 2> /dev/null)" == *notes*sprites* ]]; then
        ok "${n}: /v1/github/repos: $(jq -c '[.[] | (.fullName // .full_name // .name)]' <<< "${repos}")"
    else
        fail "${n}: /v1/github/repos: $(head -c 300 <<< "${repos}")"
    fi
    shot_on "${n}" "${n}-11-github-signed-in.png"
    app_shot "${n}" settings-github "${n}-11-settings-accounts.png" wados-settings --page accounts
}

# Signing out deletes the secret.
wadd_github_out() {
    local n="$1" gh
    gh="$(api_on "${n}" DELETE /v1/github)"
    if [[ "$(api_on "${n}" GET /v1/github)" == null ]] && ! root_on "${n}" "podman secret exists github_token" 2> /dev/null; then
        ok "${n}: signed out: /v1/github is null, and the secret is gone"
    else
        fail "${n}: signing out (${gh}): /v1/github $(api_on "${n}" GET /v1/github); secret $(root_on "${n}" "podman secret exists github_token && echo still there" 2>&1)"
    fi
}

# ------------------------------------------------------------ 12. projects

# A run's phase on B (idle when there's none).
phase_on() { api_on "$1" GET /v1/runs | jq -r --arg id "$2" '[.[] | select(.id == $id) | .phase][0] // "idle"' 2> /dev/null; }
# Its newest open job (kind launch).
open_job_on() { api_on "$1" GET /v1/jobs | jq -c --arg id "$2" '[.[] | select(.kind == "launch" and .subject == $id)] | last' 2> /dev/null; }
stages_of() { jq -r '.stages[]? | "\(.label): \(.state)\(if .error then " (" + .error + ")" else "" end)"' <<< "$1" 2> /dev/null; }
in_ws() { root_on b "podman exec --user abc wad-${WSP} sh -c $(printf '%q' "$1")"; }

# Opens B's wadspace (in the background) and waits for it: prints its open job.
open_wsp() {
    local r ph=""
    r="$(api_on b POST "/v1/wadspaces/${WSP}/open" '{"display": "background"}')" || { fail "b: opening ${WSP}: ${r}"; return 1; }
    for _ in $(seq 150); do
        ph="$(phase_on b "${WSP}")"
        [[ "${ph}" == ready || "${ph}" == error ]] && break
        sleep 2
    done
    [[ "${ph}" == ready ]] || { fail "b: ${WSP} is ${ph:-?}: $(api_on b GET /v1/runs | jq -c --arg id "${WSP}" '.[] | select(.id == $id)' 2> /dev/null)"; journal_on b -u wadd; return 1; }
    ok "b: ${WSP} is ready"
}

stop_wsp() {
    api_on b POST "/v1/wadspaces/${WSP}/stop" > /dev/null
    wait_for 120 eval '[[ "$(phase_on b "${WSP}")" == idle ]]'
}

wadd_projects() {
    local doc job stages want got perms mnt units
    # The files: B's (sam's), and A's (over Taildrive, as sam).
    sam_on b 'mkdir -p ~/Projects/Here && echo "here on B" > ~/Projects/Here/here.txt && echo "a brief from B" > ~/Projects/brief.txt' < /dev/null
    sam_on a 'mkdir -p ~/Shared && echo "there on A" > ~/Shared/there.txt && echo "a note from A" > ~/a-note.txt' < /dev/null
    WSP="$(api_on b POST /v1/ops '{"op": {"op": "space.create", "name": "Tailnet Files"}}' | jq -r .created 2> /dev/null)"
    if [[ -z "${WSP}" || "${WSP}" == null ]]; then fail "b: no wadspace made"; WSP=""; return; fi
    api_on b POST /v1/ops "{\"op\": {\"op\": \"apps.add\", \"ws\": \"${WSP}\", \"app\": \"terminal\"}}" > /dev/null
    got="$(api_on b POST /v1/ops "{\"op\": {\"op\": \"space.keepFiles\", \"ws\": \"${WSP}\", \"files\": [
        {\"machine\": \"wados-b\", \"path\": \"~/Projects/Here\", \"name\": \"Here\", \"folder\": true},
        {\"machine\": \"wados-b\", \"path\": \"~/Projects/brief.txt\", \"name\": \"brief.txt\"},
        {\"machine\": \"wados-a\", \"path\": \"~/Shared\", \"name\": \"Shared\", \"folder\": true},
        {\"machine\": \"wados-a\", \"path\": \"~/a-note.txt\", \"name\": \"a-note.txt\"}]}}")" || fail "b: space.keepFiles: ${got}"
    got="$(api_on b POST /v1/ops "{\"op\": {\"op\": \"builder.add\", \"ws\": \"${WSP}\", \"section\": \"customizations\", \"item\": \"repo:sam-wados/notes\"}}")" || fail "b: builder.add repo:sam-wados/notes: ${got}"
    doc="$(api_on b GET "/v1/wadspaces/${WSP}")"
    if [[ "$(jq '.look.projects | length' <<< "${doc}" 2> /dev/null)" == 5 ]]; then
        ok "b: ${WSP} has five projects ($(jq -c '[.look.projects[] | {as, kind, machine: .source.machineId?}]' <<< "${doc}"))"
    else
        fail "b: ${WSP}'s projects: $(jq -c '.look.projects' <<< "${doc}" 2> /dev/null || echo "${doc}")"
    fi
    got="$(api_on b POST "/v1/wadspaces/${WSP}/build")"
    [[ "$(jq -r '.job.id // empty' <<< "${got}" 2> /dev/null)" ]] && job_done b "$(jq -r .job.id <<< "${got}")" 600 > /dev/null
    open_wsp || return
    job="$(open_job_on b "${WSP}")"
    stages="$(stages_of "${job}")"
    note "its open job: $(jq -r .title <<< "${job}" 2> /dev/null)"
    indent <<< "${stages}"
    for want in "Mounting Here: done" "Copying brief.txt: done" "Cloning sam-wados/notes: done" "Starting its container: done"; do
        if grep -qxF "${want}" <<< "${stages}"; then ok "b: the open job's stage \"${want}\""; else fail "b: no stage \"${want}\""; fi
    done
    for want in "Mounting Shared from .* over Taildrive: done" "Copying a-note.txt from .* over Taildrive: done"; do
        if grep -qx "${want}" <<< "${stages}"; then ok "b: the open job's stage \"$(grep -x "${want}" <<< "${stages}")\""; else fail "b: no stage like \"${want}\""; fi
    done

    # Inside, as abc: B's folder, read and written.
    got="$(in_ws 'cat /config/Desktop/Here/here.txt && echo "from inside" > /config/Desktop/Here/from-inside.txt && echo written' 2>&1)"
    if [[ "${got}" == $'here on B\nwritten' && "$(root_on b 'stat -c %U /var/home/sam/Projects/Here/from-inside.txt' 2>&1)" == sam ]]; then
        ok "b: inside, abc reads and writes B's folder (Here), and what it wrote is sam's"
    else
        fail "b: B's folder inside: ${got} ($(root_on b 'ls -ln /var/home/sam/Projects/Here' 2>&1 | tr '\n' ' '))"
    fi
    # A's folder over Taildrive (rclone, as root, in the machine's mounts).
    units="$(root_on b "systemctl list-units --plain --no-legend 'wadtaildrive-*' | awk '{ print \$1, \$3, \$4 }'" 2>&1)"
    mnt="$(root_on b "findmnt -rn -t fuse.rclone -o TARGET,SOURCE" 2>&1)"
    if [[ "${units}" == wadtaildrive-*" active running"* && "${mnt}" == /var/lib/wados/taildrive/${WSP}/* ]]; then
        ok "b: A's folder is rclone's mount (${units%% *}: ${mnt})"
    else
        fail "b: A's folder's mount: units '${units}', fuse.rclone '${mnt}'"
    fi
    got="$(in_ws 'cat /config/Desktop/Shared/there.txt && echo "from the wadspace on B" > /config/Desktop/Shared/from-wadspace.txt && echo written' 2>&1)"
    if [[ "${got}" == $'there on A\nwritten' ]] && wait_for 20 eval '[[ "$(root_on a "stat -c %U /var/home/sam/Shared/from-wadspace.txt" 2> /dev/null)" == sam ]]'; then
        ok "b: inside, abc reads and writes A's folder (Shared) over Taildrive, and on A the file is sam's"
    else
        fail "b: A's folder inside: ${got} (on A: $(root_on a 'ls -ln /var/home/sam/Shared' 2>&1 | tr '\n' ' '))"
    fi
    # The copies: B's file, and A's over WebDAV.
    got="$(in_ws 'for f in brief.txt a-note.txt; do p="$(find /config/Desktop /run/wadspaces-copies -name "$f" -type f 2> /dev/null | head -1)"; echo "$f: ${p:-none}: $(cat "$p" 2> /dev/null)"; done' 2>&1)"
    if grep -q '^brief.txt: .*: a brief from B$' <<< "${got}" && grep -q '^a-note.txt: .*: a note from A$' <<< "${got}"; then
        ok "b: inside, the copies are there: $(tr '\n' ';' <<< "${got}")"
    else
        fail "b: the copies inside: $(tr '\n' ';' <<< "${got}")"
    fi
    # The clone: uid 1000's, through the 0711 folders, with the token (the mock asked for it).
    got="$(root_on b "d=\$(find /var/lib/wados/projects/${WSP} -mindepth 1 -maxdepth 1 -type d -not -name .\\* | head -1); stat -c '%u %a %n' \"\$d\" && runuser -u sam -- git -C \"\$d\" log --oneline -1 && ls -Z \"\$d/README.md\"" 2>&1)"
    if [[ "${got}" == "1000 "*"First notes"* ]]; then
        ok "b: the clone is uid 1000's: $(tr '\n' ';' <<< "${got}")"
    else
        fail "b: the clone: $(tr '\n' ';' <<< "${got}")"
    fi
    if in_ws 'test -f /config/Desktop/notes/README.md' 2> /dev/null; then ok "b: inside, the clone is /config/Desktop/notes"; else fail "b: no /config/Desktop/notes/README.md inside"; fi
    if jq -e 'select(.path == "/sam-wados/notes.git/info/refs" and .auth)' "${GH_LOG}" > /dev/null 2>&1; then
        ok "the mock's repository was fetched with the token (git asked wadd's credential helper after a 401)"
    else
        fail "the mock's log has no authenticated fetch of sam-wados/notes.git"
    fi
    shot_on b b-12-projects.png

    # Stopped: A's folder is unmounted.
    if stop_wsp; then ok "b: ${WSP} stopped"; else fail "b: ${WSP} didn't stop ($(phase_on b "${WSP}"))"; fi
    if wait_for 30 eval '[[ -z "$(root_on b "findmnt -rn -t fuse.rclone -o TARGET; systemctl list-units --plain --no-legend \"wadtaildrive-*\" | grep -w active" 2> /dev/null)" ]]'; then
        ok "b: A's folder is unmounted, and its unit gone"
    else
        fail "b: still mounted: $(root_on b "findmnt -rn -t fuse.rclone; systemctl list-units --plain --no-legend 'wadtaildrive-*'" 2>&1 | tr '\n' ' ')"
    fi
    # 0711, and after wadd starts again.
    perms="$(root_on b 'stat -c "%a %n" /var/lib/wados /var/lib/wados/keys /var/lib/wados/projects' 2>&1 | tr '\n' ' ')"
    root_on b 'systemctl restart wadd' > /dev/null 2>&1
    wait_for 30 eval '[[ "$(api_on b GET /v1/health)" == ok ]]'
    got="$(root_on b 'stat -c "%a %n" /var/lib/wados /var/lib/wados/keys /var/lib/wados/projects' 2>&1 | tr '\n' ' ')"
    if [[ "${perms}" == "711 /var/lib/wados 711 /var/lib/wados/keys 711 /var/lib/wados/projects " && "${got}" == "${perms}" ]]; then
        ok "b: /var/lib/wados, keys/ and projects/ are 0711, and stay so across a wadd restart"
    else
        fail "b: the state folders: ${perms}; after a restart: ${got}"
    fi
}

# ------------------------------------------------------------ 15. A off

wadd_offline() {
    local job stages got
    # A goes (and with it, after this, its own checks).
    on a
    stop > /dev/null
    if wait_for 120 eval '[[ "$(api_on b GET /v1/peers | jq -r ".[] | select(.id == \"wados-a\") | .online" 2> /dev/null)" == false ]]'; then
        ok "b: /v1/peers has A offline"
    else
        fail "b: A isn't offline in /v1/peers: $(api_on b GET /v1/peers | jq -c '[.[] | {id, online}]' 2> /dev/null)"
    fi
    open_wsp || return
    job="$(open_job_on b "${WSP}")"
    stages="$(stages_of "${job}")"
    indent <<< "${stages}"
    if grep -q "Shared.*is on .*, which is offline. Folders come over Taildrive, so both machines need to be online." <<< "${job}"; then
        ok "b: it opened without A's folder, and says why ($(grep -o '[^"]*which is offline[^"]*' <<< "${job}" | head -1))"
    else
        fail "b: the open job doesn't say A is offline: ${stages}"
    fi
    got="$(in_ws 'ls -d /config/Desktop/Shared 2>&1; ls /config/Desktop' 2>&1 | tr '\n' ' ')"
    if [[ "${got}" == *"No such file"* ]]; then ok "b: inside, there's no Shared (${got})"; else fail "b: inside: ${got}"; fi
    stop_wsp > /dev/null
    local denials
    denials="$(root_on b "journalctl -b --no-pager -o cat | grep -a 'avc:  denied'; echo asked" 2> /dev/null)"
    if [[ "${denials}" == asked ]]; then ok "b: still no SELinux denials"; else fail "b: SELinux: $(head -n 5 <<< "${denials%asked}")"; fi
}

# ------------------------------------------------------------ the end

tailnet_finish() {
    say "the result ($(($(now) - $1)) s)"
    [[ -f "${VM}/tailnet-starved" ]] && note "(the laptop ran out of memory: the VMs were quit, so the steps after it failed for that)"
    [[ ${FAILED} -eq 0 ]] && echo "all ok" || echo "${FAILED} failed"
    exit $((FAILED > 0))
}

# Everything this check started goes, whatever happened (VM_KEEP=1: the VMs
# and Headscale stay, to look at).
tailnet_cleanup() {
    local rc=$?
    trap - EXIT
    [[ -n "${DEVICE_UP_PID:-}" ]] && kill "${DEVICE_UP_PID}" 2> /dev/null
    [[ -n "${WATCHDOG_PID:-}" ]] && kill "${WATCHDOG_PID}" 2> /dev/null
    if [[ -n "${GH_PID:-}" && "${VM_KEEP:-0}" != 1 ]]; then
        kill "${GH_PID}" 2> /dev/null
        wait "${GH_PID}" 2> /dev/null
        rm -rf "${VM}/tailnet-git"
    fi
    if [[ "${VM_KEEP:-0}" == 1 ]]; then
        echo "kept (VM_KEEP=1): tools/vm.sh --name a|b ssh; tools/vm.sh --name a stop; tools/vm.sh --name b stop; tools/tailnet.sh stop; kill ${GH_PID:-} (GitHub's mock)"
        exit "${rc}"
    fi
    local n
    for n in a b; do
        on "${n}"
        # Their layers are thrown away: quit, after a moment to power off.
        if running "${RUN}/qemu.pid"; then
            qmp_on "${n}" powerdown 2> /dev/null
        fi
    done
    for n in a b; do
        on "${n}"
        stop > /dev/null
        rm -f "${RUN}/run.qcow2" "${RUN}/vars.fd"
    done
    "${TN}" stop > /dev/null
    echo "cleaned up: VMs a and b stopped, Headscale's pod, volume, CA and keys removed, GitHub's mock stopped"
    exit "${rc}"
}
