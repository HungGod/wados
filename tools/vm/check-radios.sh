# tools/vm.sh radios-check: the network and Bluetooth (phase 8) through
# the VM's own wadd (the image's, on /run/wadd/wadd.sock, [radios] on by
# default for a machine), against its real NetworkManager and BlueZ
# (sourced by tools/vm.sh, whose helpers it uses).
#
#   1. it boots (a fresh layer) and wadd answers on its socket (its log if not)
#   2. GET /v1/network: the wire (QEMU's virtio-net) plugged, with
#      NetworkManager's address on it (and nmcli agrees)
#   3. Wi-Fi, when the VM's kernel has mac80211_hwsim (kernel-modules-internal):
#      two radios, NetworkManager's hotspot on the second; a wrong password
#      is 401 and leaves no profile, the right one joins, airplane mode turns
#      Wi-Fi off and back on, Forget forgets it
#   4. Bluetooth: GET /v1/bluetooth answers; with a controller (QEMU has
#      none unless one's passed in), off and on through wadd, as
#      bluetoothctl sees it; without one, wadd says it's off and turning it
#      on is refused (409)
#
# Real Wi-Fi radios and Bluetooth pairing need real hardware (the laptop).

radios_check() {
    set +e
    FAILED=0
    ok() { printf 'ok    %s\n' "$*"; }
    fail() { printf 'FAIL  %s\n' "$*"; FAILED=$((FAILED + 1)); }
    skip() { printf 'skip  %s\n' "$*"; }
    as_root() { ssh_to "$(cat "${VM}/ssh.port")" root "$@"; }
    # wadd's API as root (its socket's owner): the body, then the status on a line of its own.
    api() {
        local body="${3:-}"
        if [[ -n "${body}" ]]; then
            as_root "curl -sS -m 30 --unix-socket /run/wadd/wadd.sock -X $1 -w '\n%{http_code}' \
                -H 'content-type: application/json' --data-binary @- http://wadd$2" <<< "${body}"
        else
            as_root "curl -sS -m 30 --unix-socket /run/wadd/wadd.sock -X $1 -w '\n%{http_code}' http://wadd$2" < /dev/null
        fi
    }
    status() { printf '%s' "${1##*$'\n'}"; }
    wadd_log() { as_root "journalctl -b --no-pager -u wadd -u wadd.socket | tail -n 30" 2> /dev/null | sed 's/^/      | /'; }
    # until SECONDS COMMAND…: it succeeds within SECONDS.
    until_ok() {
        local end=$(($(now) + $1))
        shift
        while (($(now) < end)); do
            "$@" && return 0
            sleep 2
        done
        return 1
    }
    answers() { [[ "$(status "$(api GET /v1/health)")" == 200 ]]; }

    say "1. boot, and wadd"
    boot --fresh && wait_ssh "$(cat "${VM}/ssh.port")" root 420 "${VM}/qemu.pid" || { fail "the VM didn't come up"; return 1; }
    if until_ok 120 answers; then
        ok "wadd answers on /run/wadd/wadd.sock"
    else
        fail "wadd doesn't answer on /run/wadd/wadd.sock"
        wadd_log
        stop
        return 1
    fi

    say "2. the wire"
    local n
    wired() { n="$(api GET /v1/network)"; grep -q '"plugged":true' <<< "${n}" && grep -q '"ethernet":{[^}]*"ip":"' <<< "${n}"; }
    if until_ok 60 wired; then
        ok "plugged, with an address: $(grep -o '"ethernet":{[^}]*}' <<< "${n}")"
        local ip
        ip="$(grep -o '"ethernet":{[^}]*}' <<< "${n}" | grep -o '"ip":"[^"]*"' | cut -d'"' -f4)"
        if as_root "nmcli -g IP4.ADDRESS dev show" 2> /dev/null | grep -q "^${ip}/"; then
            ok "NetworkManager has ${ip} too"
        else
            fail "NetworkManager doesn't have ${ip}"
        fi
    else
        fail "the wire: $(status "${n}") ${n%$'\n'*}"
        wadd_log
    fi

    say "3. Wi-Fi (mac80211_hwsim)"
    if as_root "modprobe mac80211_hwsim radios=2" 2> /dev/null; then
        sleep 3
        as_root "nmcli dev wifi hotspot ifname wlan1 ssid wados-test password 'correct horse'" > /dev/null
        in_range() { api GET /v1/network | grep -q '"ssid":"wados-test"'; }
        until_ok 30 in_range && ok "the hotspot's in range" || fail "no hotspot in range"
        n="$(api POST /v1/network/wifi/connect '{"ssid":"wados-test","password":"not the one"}')"
        [[ "$(status "${n}")" == 401 ]] && ok "a wrong password: 401" || fail "a wrong password: ${n}"
        as_root "nmcli -t -f NAME con show" | grep -qx wados-test && fail "a wrong password left a profile"
        n="$(api POST /v1/network/wifi/connect '{"ssid":"wados-test","password":"correct horse"}')"
        grep -q '"connected":"wados-test"' <<< "${n}" && ok "joined" || fail "joining: ${n}"
        n="$(api POST /v1/network/airplane '{"on":true}')"
        grep -q '"wifiOn":false' <<< "${n}" && ok "airplane mode: Wi-Fi off" || fail "airplane mode: ${n}"
        n="$(api POST /v1/network/airplane '{"on":false}')"
        grep -q '"wifiOn":true' <<< "${n}" && ok "airplane mode off: Wi-Fi on" || fail "airplane mode off: ${n}"
        n="$(api POST /v1/network/wifi/forget '{"ssid":"wados-test"}')"
        [[ "$(status "${n}")" == 200 ]] && ok "forgotten" || fail "forget: ${n}"
    else
        skip "no mac80211_hwsim in this kernel (kernel-modules-internal)"
    fi

    say "4. Bluetooth"
    n="$(api GET /v1/bluetooth)"
    if [[ "$(status "${n}")" != 200 ]]; then
        fail "GET /v1/bluetooth: ${n}"
        wadd_log
    elif ! as_root "timeout 10 bluetoothctl list" 2> /dev/null | grep -q '^Controller'; then
        ok "answers: ${n%$'\n'*}"
        grep -q '"on":false' <<< "${n}" && ok "no controller: off" || fail "no controller, but: ${n%$'\n'*}"
        [[ "$(status "$(api POST /v1/bluetooth/power '{"on":true}')")" == 409 ]] && ok "turning it on: 409" \
            || fail "turning on a missing controller wasn't refused"
        skip "no Bluetooth controller in this VM: power isn't tried"
    else
        powered() { as_root "timeout 10 bluetoothctl show" 2> /dev/null | grep -q "Powered: $1"; }
        n="$(api POST /v1/bluetooth/power '{"on":false}')"
        grep -q '"on":false' <<< "${n}" && until_ok 10 powered no && ok "off (bluetoothctl agrees)" || fail "off: ${n}"
        n="$(api POST /v1/bluetooth/power '{"on":true}')"
        grep -q '"on":true' <<< "${n}" && until_ok 10 powered yes && ok "on (bluetoothctl agrees)" || fail "on: ${n}"
        n="$(api POST /v1/bluetooth/scan '{"on":true}')"
        grep -q '"scanning":true' <<< "${n}" && ok "looking for devices" || fail "scan: ${n}"
        api POST /v1/bluetooth/scan '{"on":false}' > /dev/null
    fi

    stop
    printf '\n%s\n' "$([[ ${FAILED} -eq 0 ]] && echo "radios-check: all ok" || echo "radios-check: ${FAILED} failed")"
    [[ ${FAILED} -eq 0 ]]
}
