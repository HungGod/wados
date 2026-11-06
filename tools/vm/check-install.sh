# tools/vm.sh installer-check [--plain] [--no-secrets]: the installer stick,
# end to end (sourced by tools/vm.sh, whose helpers it uses). tools/vm.sh
# stick makes the stick. Cloudflare is tools/vm/cloudflare-mock.py on this
# laptop, with fake keys: nothing reaches Cloudflare or touches DNS.
#
#   1. the stick boots to WadSpaces Live: no lock, the installer open
#   2. wadd offers only the blank NVMe disk, and refuses bad requests
#   3. the install, through wadd's API, as the installer does it
#   4. what's on the disk, read from the stick: the machine's files, and
#      neither of the stick's Cloudflare keys anywhere
#   5. the disk boots alone: a wrong disk password, then the right one
#   6. first boot: the name, the account, the address, the modes; the
#      password unlocks the lock screen, and Set up WadSpaces opens
#   7. Set up to its end; started again, nothing of first boot comes back
#
# --plain: no disk password. --no-secrets: a stick without Cloudflare's keys.

MOCK_PORT=18555
ACCOUNT=sam
ACCOUNT_PASS='Account-44!'
DISK_PASS='Disk-pass-44!'
WADD='curl -sS -m 30 --unix-socket /run/wadd/wadd.sock'

ok() { printf 'ok    %s\n' "$*"; }
fail() { printf 'FAIL  %s\n' "$*"; FAILED=$((FAILED + 1)); }
note() { printf '      %s\n' "$*"; }
as_root() { ssh_to "$(cat "${VM}/ssh.port")" root "$@"; }
as_person() { ssh_to "$(cat "${VM}/ssh.port")" "${ACCOUNT}" "$@"; }
expect() { # what, wanted, got
    if [[ "$3" == *"$2"* ]]; then ok "$1"; else fail "$1: wanted “$2”, got “${3:0:300}”"; fi
}
absent() { # what, unwanted, got
    if [[ "$3" != *"$2"* ]]; then ok "$1"; else fail "$1: has “$2”"; fi
}
wait_for() {
    local deadline=$(($(now) + $1))
    shift
    while (($(now) < deadline)); do
        "$@" 2> /dev/null && return 0
        sleep 2
    done
    return 1
}
ssh_up() { as_root true; }
live_session() { as_root 'pgrep -u live -x wados-shell > /dev/null'; }
installer_open() { as_root 'pgrep -u live -x wados-installer > /dev/null'; }

api_post() { # path, json on stdin
    as_root "${WADD} -X POST -H 'content-type: application/json' --data-binary @- http://wadd$1"
}

stop_mock() {
    if [[ -n "${MOCK_PID:-}" ]]; then
        kill "${MOCK_PID}" 2> /dev/null || true
        wait "${MOCK_PID}" 2> /dev/null || true
    fi
    MOCK_PID=""
}

install_check() {
    local plain="" secrets=1 a
    for a in "$@"; do
        case "${a}" in
            --plain) plain=1 ;;
            --no-secrets) secrets="" ;;
            *) die "installer-check: unknown option ${a}" ;;
        esac
    done
    FAILED=0
    [[ -f "${VM}/stick.qcow2" ]] || die "no ${VM}/stick.qcow2: tools/vm.sh stick"
    [[ -f "${VM}/wados.qcow2" ]] || die "no ${VM}/wados.qcow2: tools/vm.sh disk"
    local stick="${VM}/stick-run.qcow2" target="${VM}/target.qcow2" shots="${VM}/install-shots"
    mkdir -p "${shots}"
    python3 "${HERE}/cloudflare-mock.py" "${MOCK_PORT}" &
    MOCK_PID=$!
    trap 'set +e; stop_mock; stop' EXIT
    stop > /dev/null
    rm -f "${stick}" "${target}"
    qemu-img create -q -f qcow2 -F qcow2 -b "${VM}/stick.qcow2" "${stick}"
    qemu-img create -q -f qcow2 "${target}" 40G

    say "1. the stick boots to WadSpaces Live"
    VM_DISK="${stick}" VM_TARGET="${target}" boot --fresh
    wait_for 240 ssh_up || { fail "the stick's SSH didn't come up"; return 1; }
    if wait_for 90 live_session; then ok "the shell runs as live, with no password to type"; else fail "no shell as live"; fi
    expect "the session is WadSpaces Live" WADOS_SESSION=live \
        "$(as_root 'tr "\0" "\n" < /proc/$(pgrep -u live -x wados-shell)/environ')"
    if wait_for 30 installer_open; then ok "the installer opens by itself"; else fail "the installer didn't open"; fi
    expect "its person is locked" 'L' "$(as_root 'passwd -S live | cut -d" " -f2')"
    sleep 3
    qmp shot "${shots}/1-live.png" 2> /dev/null && note "screenshot: ${shots}/1-live.png"
    # A real keyboard's Enter: Get started, then on from the name field.
    qmp key ret && sleep 2
    qmp shot "${shots}/1-computer.png" 2> /dev/null
    qmp key ret && sleep 2
    qmp shot "${shots}/1-account.png" 2> /dev/null && note "screenshots: ${shots}/1-computer.png, 1-account.png"
    if [[ -z "${secrets}" ]]; then
        as_root 'rm -f /var/lib/wados/installer/cloudflare.toml; systemctl restart wadd'
        sleep 3
    fi

    say "2. wadd's offer"
    local info
    info="$(as_root "${WADD} http://wadd/v1/install")"
    expect "the blank disk is offered" WADVM0001 "${info}"
    absent "the stick isn't offered" '"name":"vda"' "${info}"
    if [[ -n "${secrets}" ]]; then expect "it'll have a public address" '"public":true' "${info}"; else expect "no public address" '"public":false' "${info}"; fi
    local by_id size
    by_id="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["disks"][0]["byId"])' "${info}")"
    size="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["disks"][0]["size"])' "${info}")"
    local disk_pass="\"diskPassword\":\"${DISK_PASS}\","
    [[ -n "${plain}" ]] && disk_pass=""
    local req="{\"name\":\"studio\",\"username\":\"${ACCOUNT}\",\"password\":\"${ACCOUNT_PASS}\",${disk_pass}\"byId\":\"${by_id}\",\"size\":${size},\"serial\":\"WADVM0001\"}"
    expect "a kept name is refused" 'kept for wadspaces.com' "$(api_post /v1/install <<< "${req/studio/www}")"
    expect "a disk that changed is refused" "isn't here any more" "$(api_post /v1/install <<< "${req/WADVM0001/OTHER}")"

    say "3. the install"
    local job t0
    t0=$(now)
    job="$(api_post /v1/install <<< "${req}" | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')"
    [[ -n "${job}" ]] || { fail "the install didn't start"; return 1; }
    local status="" j
    for _ in $(seq 180); do
        j="$(as_root "${WADD} http://wadd/v1/jobs/${job}")"
        status="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["job"]["status"])' "${j}" 2> /dev/null)"
        [[ "${status}" == done || "${status}" == error || "${status}" == cancelled ]] && break
        sleep 5
    done
    if [[ "${status}" == done ]]; then
        ok "installed ($(($(now) - t0)) s)"
    else
        fail "the install ended ${status}: ${j:0:600}"
        as_root "journalctl -u wados-install -n 30 --no-pager" | sed 's/^/      | /'
        return 1
    fi
    expect "the job's folder is gone" 0 "$(as_root 'ls /run/wados/install | wc -l')"
    local seen
    seen="$(curl -s "http://127.0.0.1:${MOCK_PORT}/__seen")"
    if [[ -n "${secrets}" ]]; then
        expect "the tunnel was made with the creator's token" 'Bearer fake-tunnel-key' "${seen}"
        expect "one proxied record" '"content": "t1.cfargotunnel.com"' "${seen}"
        expect "ingress goes to wadd's proxy" 'http://localhost:8085' "${seen}"
    else
        expect "Cloudflare wasn't called" '[]' "${seen}"
    fi

    say "4. what's on the disk"
    local read_disk
    read_disk="$(cat << 'SCRIPT'
set -euo pipefail
d=/dev/nvme0n1p4
m=/run/wados/check-target
if [[ "$1" == encrypted ]]; then
    cryptsetup isLuks "$d" && echo "luks: yes"
    printf %s "$2" | cryptsetup open --key-file - "$d" check-root
    d=/dev/mapper/check-root
fi
mkdir -p "$m"
mount "$d" "$m"
var="$(find "$m/ostree/deploy" -mindepth 2 -maxdepth 2 -name var -type d | head -1)"
etc="$(find "$m/ostree/deploy" -mindepth 4 -maxdepth 4 -path '*/deploy/*/etc' -type d | head -1)"
echo "provision: $(cd "$var/lib/wados/provision" && ls -A | tr '\n' ' ')"
echo "keys found: $(grep -rlE 'fake-dns-key|fake-tunnel-key' "$var/lib/wados" "$etc" 2> /dev/null | wc -l)"
echo "plain password found: $(grep -rlF "$3" "$var/lib/wados/provision" 2> /dev/null | wc -l)"
stat -c 'account mode: %a' "$var/lib/wados/provision/account"
# This check's own way in: on this test disk only, never in the product.
mkdir -p "$etc/ssh/sshd_config.d"
cat > "$etc/ssh/sshd_config.d/30-wados-vm.conf" << 'CONF'
PermitRootLogin prohibit-password
AuthorizedKeysFile .ssh/authorized_keys /etc/ssh/wados-vm-keys
CONF
cp /root/.ssh/authorized_keys "$etc/ssh/wados-vm-keys" 2> /dev/null || cp /var/roothome/.ssh/authorized_keys "$etc/ssh/wados-vm-keys"
chmod 644 "$etc/ssh/wados-vm-keys"
chcon -t etc_t "$etc/ssh/wados-vm-keys" "$etc/ssh/sshd_config.d/30-wados-vm.conf"
sync
umount "$m"
[[ "$1" == encrypted ]] && cryptsetup close check-root
true
SCRIPT
)"
    local mode=encrypted out
    [[ -n "${plain}" ]] && mode=plain
    out="$(as_root bash -s -- "${mode}" "'${DISK_PASS}'" "'${ACCOUNT_PASS}'" <<< "${read_disk}" 2>&1)"
    echo "${out}" | sed 's/^/      | /'
    [[ -z "${plain}" ]] && expect "root is LUKS" 'luks: yes' "${out}"
    expect "neither of the stick's keys is on the disk" 'keys found: 0' "${out}"
    expect "the password isn't there in the clear" 'plain password found: 0' "${out}"
    expect "the account's file is root's" 'account mode: 600' "${out}"
    expect "first boot has the account and hostname" 'account' "${out}"
    if [[ -n "${secrets}" ]]; then
        expect "first boot has the tunnel's token" 'cloudflared-token' "${out}"
        expect "first boot has the TURN key" 'turn.toml' "${out}"
    else
        absent "no tunnel's token" 'cloudflared-token' "${out}"
    fi

    say "5. the disk boots alone"
    stop > /dev/null
    stop_mock
    VM_DISK="${target}" boot
    if [[ -z "${plain}" ]]; then
        sleep 30
        qmp shot "${shots}/5-password.png" 2> /dev/null && note "screenshot: ${shots}/5-password.png"
        if ssh_up 2> /dev/null; then fail "it's up without its password"; else ok "it waits for its disk password"; fi
        qmp keys 'wrong-password' && qmp key ret
        sleep 12
        if ssh_up 2> /dev/null; then fail "a wrong password started it"; else ok "a wrong disk password doesn't start it"; fi
        qmp keys "${DISK_PASS}" && qmp key ret
    fi
    wait_for 240 ssh_up || { fail "it didn't come up"; qmp shot "${shots}/5-stuck.png"; return 1; }
    ok "it boots from its own firmware entry$([[ -z "${plain}" ]] && echo ", once the password's typed")"

    say "6. first boot"
    expect "wados-provision ran" 'Result=success' "$(as_root 'systemctl show wados-provision -p Result')"
    expect "the name" studio "$(as_root hostname)"
    expect "the account is uid 1000" "uid=1000(${ACCOUNT})" "$(as_root "id ${ACCOUNT}")"
    expect "in wheel and wados" 'wados' "$(as_root "id -nG ${ACCOUNT}")"
    expect "nothing left to provision" 'gone' "$(as_root 'test -e /var/lib/wados/provision && echo there || echo gone')"
    expect "it isn't Live" 'none' "$(as_root 'test -e /etc/wados/live && echo live || echo none')"
    expect "the id" 'id = "studio"' "$(as_root 'cat /etc/wados/wadd.toml')"
    expect "it follows the published image" 'ghcr.io/hunggod/wados-host' "$(as_root 'bootc status --format json')"
    expect "setup is at its start" 'welcome' "$(as_root 'cat /var/lib/wados/setup.json')"
    expect "Tailscale uploads no logs" 'TS_NO_LOGS_NO_SUPPORT=true' "$(as_root 'systemctl show tailscaled -p Environment')"
    if [[ -n "${secrets}" ]]; then
        expect "the public host" 'host = "studio.wadspaces.com"' "$(as_root 'cat /etc/wados/wadd.toml')"
        expect "the tunnel's token is root's alone" '600 root' "$(as_root 'stat -c "%a %U" /etc/cloudflared/token')"
        expect "the TURN key is root's alone" '600 root' "$(as_root 'stat -c "%a %U" /etc/wados/turn.toml')"
        expect "cloudflared's token is the machine's" 'fake-machine-token' "$(as_root 'cat /etc/cloudflared/token')"
    else
        expect "no tunnel's token" 'none' "$(as_root 'test -e /etc/cloudflared/token && echo there || echo none')"
    fi
    if [[ -z "${plain}" ]]; then
        expect "root is on dm-crypt" 'crypt' "$(as_root 'lsblk -sno TYPE "$(findmnt -no SOURCE /sysroot)"')"
    fi
    locked() { as_root "XDG_RUNTIME_DIR=/run/user/1000 runuser -u ${ACCOUNT} -- wadsctl locked" 2> /dev/null; }
    wait_for 60 test -n "$(locked)"
    local shell_up=""
    for _ in $(seq 30); do [[ "$(locked)" == "ok true" ]] && { shell_up=1; break; }; sleep 2; done
    if [[ -n "${shell_up}" ]]; then ok "every start lands on the lock screen"; else fail "not locked: $(locked)"; fi
    qmp shot "${shots}/6-lock.png" 2> /dev/null
    if as_root 'pgrep -f "[w]ados-settings --setup" > /dev/null'; then fail "Set up opened before the sign-in"; else ok "Set up waits for the sign-in"; fi
    sleep 1
    qmp keys "${ACCOUNT_PASS}" && qmp key ret
    local unlocked=""
    for _ in $(seq 20); do [[ "$(locked)" == "ok false" ]] && { unlocked=1; break; }; sleep 1; done
    if [[ -n "${unlocked}" ]]; then ok "the account's password unlocks it"; else fail "still locked: $(locked)"; fi
    setup_open() { as_root 'pgrep -f "[w]ados-settings --setup" > /dev/null'; }
    if wait_for 30 setup_open; then ok "Set up WadSpaces opens at the first sign-in"; else fail "Set up didn't open"; fi
    sleep 4
    qmp shot "${shots}/6-setup.png" 2> /dev/null && note "screenshot: ${shots}/6-setup.png"
    sleep 15
    qmp shot "${shots}/6-settled.png" 2> /dev/null && note "screenshot: ${shots}/6-settled.png"

    say "7. Set up to its end"
    expect "it can go a step on" '"step":"wifi"' "$(as_root "${WADD} -X PUT -H 'content-type: application/json' -d '{\"step\":\"wifi\"}' http://wadd/v1/setup")"
    expect "no wadspace until it's asked for" '"wadspaces":[]' "$(as_root "${WADD} http://wadd/v1/library")"
    expect "Install ends it" '"step":"done"' "$(api_post /v1/setup/design <<< '{"starter":true}')"
    expect "WadOS is there" '"name":"WadOS"' "$(as_root "${WADD} http://wadd/v1/library")"
    as_root 'systemctl reboot' 2> /dev/null || true
    sleep 15
    if [[ -z "${plain}" ]]; then
        sleep 15
        qmp keys "${DISK_PASS}" && qmp key ret
    fi
    wait_for 240 ssh_up || { fail "it didn't come back"; return 1; }
    expect "started again, it still has its name" studio "$(as_root hostname)"
    expect "first boot doesn't run again" 'ConditionResult=no' "$(as_root 'systemctl show wados-provision -p ConditionResult')"
    sleep 20
    qmp keys "${ACCOUNT_PASS}" && qmp key ret
    sleep 10
    if as_root 'pgrep -f "[w]ados-settings --setup" > /dev/null'; then fail "Set up opened again"; else ok "Set up doesn't open again"; fi

    stop > /dev/null
    stop_mock
    trap - EXIT
    echo
    if ((FAILED)); then
        echo "installer-check: ${FAILED} failed"
        return 1
    fi
    echo "installer-check: all ok"
}
