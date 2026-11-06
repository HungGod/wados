# tools/vm.sh modes-check: focus, displays and server mode on the machine
# (sourced after tools/vm/check.sh, whose helpers it uses).
#
#   1. a wadspace, and a second: focus in the first; what leaves it is refused
#      and its keys do nothing; time up, it's a wadspace again
#   2. a second display: Open on 2; unplugged, it's in the background and says so
#   3. server mode: the session ends and the console shows; what's on the screen
#      stopped and opening is refused; Ctrl+Alt+Delete's menu; back, locked

space() { # name
    local id
    id="$(api POST /v1/ops "{\"op\": {\"op\": \"space.create\", \"name\": \"$1\"}}" | jq -r .created)"
    api POST /v1/ops "{\"op\": {\"op\": \"apps.add\", \"ws\": \"${id}\", \"app\": \"terminal\"}}" > /dev/null
    api POST "/v1/wadspaces/${id}/build" > /dev/null
    echo "${id}"
}
ready() { [[ "$(run_phase "$1")" == ready ]]; }
code() { # METHOD PATH [BODY]: the HTTP status
    as_sam "curl -s -o /dev/null -w '%{http_code}' --unix-socket /run/wadd/wadd.sock -X $1 -H 'content-type: application/json' ${3:+-d '$3'} http://wadd$2" < /dev/null
}
focus_off() { [[ "$(shell focus)" == "ok off" ]]; }
is_idle() { [[ "$(run_phase "$1")" == idle || -z "$(run_phase "$1")" ]]; }
unit_is() { [[ "$(as_root systemctl is-active "$1" 2> /dev/null)" == "$2" ]]; }
expect_is() { if [[ "$3" == "$2" ]]; then ok "$1"; else fail "$1: wanted “$2”, got “$3”"; fi; }

check_focus() {
    say "1. focus"
    local a b
    a="$(space 'Focus A')"
    b="$(space 'Focus B')"
    api POST "/v1/wadspaces/${b}/open" '{"display": "screen"}' > /dev/null
    wait_for 120 ready "${b}" && ok "B is on the screen" || fail "B didn't open"
    local on
    on="$(api POST /v1/focus "{\"ws\": \"${a}\", \"minutes\": 1}")"
    if [[ "$(jq -r .ws <<< "${on}")" == "${a}" ]]; then ok "focus started"; else fail "focus: ${on}"; fi
    wait_for 120 ready "${a}" && ok "A opened" || fail "A didn't open"
    [[ "$(api GET /v1/runs | jq -r --arg i "${b}" '.[] | select(.id == $i) | .display')" == background ]] \
        && ok "B went to the background" || fail "B wasn't sent to the background"
    [[ "$(shell focus | cut -d' ' -f2)" == on ]] && ok "the shell knows" || fail "shell: $(shell focus)"
    expect_is "stopping it is refused" 409 "$(code POST "/v1/wadspaces/${a}/stop")"
    expect_is "opening another is refused" 409 "$(code POST "/v1/wadspaces/${b}/open" '{"display": "screen"}')"
    expect_is "a window is refused" 409 "$(code POST "/v1/wadspaces/${a}/open" '{"display": "window"}')"
    expect_is "server mode is refused" 409 "$(code POST /v1/server '{"on": true}')"
    expect_is "Alt+Space does nothing" "ok focusing" "$(shell search)"
    expect_is "Super+Tab does nothing" "ok focusing" "$(shell switcher next)"
    expect_is "Ctrl+Alt+Down does nothing" "ok focusing" "$(shell windowed)"
    shot_to 10-focus.png
    if wait_for 80 focus_off; then ok "time up, focus ends"; else fail "focus didn't end: $(shell focus)"; fi
    expect_is "the wadspace stays" ready "$(run_phase "${a}")"
    expect_is "stopping is allowed again" 200 "$(code POST "/v1/wadspaces/${b}/stop")"
    FOCUS_A="${a}"
}

randr() {
    as_sam "XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=\$(ls /run/user/1000 | grep -m1 '^wayland-[0-9]*$') wlr-randr $*"
}

check_displays() {
    say "2. a second display"
    local a="${FOCUS_A}" second=VIRT-2
    # QEMU shows one head without a window to show it in: labwc's virtual
    # output comes and goes as a plugged display does. Keys of this check's,
    # in sam's own rc.xml, which the session merges.
    as_sam "mkdir -p ~/.config/labwc; cat > ~/.config/labwc/rc.xml" << XML
<?xml version="1.0"?><labwc_config><keyboard>
<keybind key="C-A-S-F9"><action name="VirtualOutputAdd" output_name="${second}"/></keybind>
<keybind key="C-A-S-F10"><action name="VirtualOutputRemove" output_name="${second}"/></keybind>
</keyboard></labwc_config>
XML
    # (SIGHUP is what --reconfigure sends: that needs the session's environment.)
    as_sam 'pkill -HUP -x labwc'
    sleep 2
    qmp key ctrl-alt-shift-f9
    sleep 4
    note "displays: $(shell displays)"
    if [[ "$(shell displays)" != *" 2=${second}"* ]]; then
        fail "the virtual display didn't come: $(shell displays)"
        return
    fi
    ok "a second display came, and the shell has it as display 2"
    shot_to 11-connected.png
    shell displays open "${a}" 2 > /dev/null
    sleep 3
    expect_is "it's on display 2" "ok ${second}" "$(shell displays on "${a}")"
    local wins
    wins="$(shell wm list)"
    [[ "${wins}" == *"wadspace ${a} on [${second}]"* ]] && ok "its window is there" || fail "its window: ${wins}"
    qmp key ctrl-alt-shift-f10
    sleep 4
    [[ "$(shell displays)" != *" 2="* ]] && ok "the shell saw it go" || fail "still there: $(shell displays)"
    expect_is "the wadspace keeps running" ready "$(run_phase "${a}")"
    expect_is "it's in the background" background \
        "$(api GET /v1/runs | jq -r --arg i "${a}" '.[] | select(.id == $i) | .display')"
    [[ "$(shell log)" == *"${second} unplugged"* ]] && ok "it says so" || fail "no toast: $(shell log)"
    shot_to 12-unplugged.png
}

check_server() {
    say "3. server mode"
    local a="${FOCUS_A}"
    api POST "/v1/wadspaces/${a}/open" '{"display": "screen"}' > /dev/null
    expect_is "it turns on" true "$(api POST /v1/server '{"on": true}' | jq -r .on)"
    wait_for 30 unit_is wados-console.service active && ok "the console is up" || { fail "no console"; journal -u wados-console; }
    wait_for 15 unit_is greetd.service inactive && ok "the session's gone" || fail "greetd is still up"
    sleep 2
    is_idle "${a}" && ok "what was on the screen stopped" || fail "it's still $(run_phase "${a}")"
    expect_is "opening is refused" 409 "$(as_root "curl -s -o /dev/null -w '%{http_code}' --unix-socket /run/wadd/wadd.sock -X POST -H 'content-type: application/json' -d '{\"display\":\"screen\"}' http://wadd/v1/wadspaces/${a}/open")"
    as_root 'systemctl start greetd' 2> /dev/null
    sleep 2
    unit_is greetd.service inactive && ok "greetd can't start while it's on" || fail "greetd started"
    unit_is wados-console.service active && ok "and that doesn't take the console down" || fail "the console went"
    expect_is "an inhibitor holds the lid" 1 "$(as_root 'systemd-inhibit --list --no-legend | grep -c WadSpaces')"
    sleep 2
    shot_to 13-console.png
    qmp key ctrl-alt-delete
    sleep 3
    shot_to 14-menu.png
    expect_is "Ctrl+Alt+Delete opens its menu" 1 "$(as_root 'test -e /run/wados/server/menu; echo $?')"
    unit_is wados-console.service active && ok "and doesn't restart it" || fail "the console went"
    qmp key ret
    if wait_for 60 unit_is greetd.service active; then ok "Back to the desktop starts the session"; else fail "greetd didn't start"; fi
    unit_is wados-console.service inactive && ok "the console's gone" || fail "the console stayed"
    if wait_for 90 locked_is true; then ok "it comes back locked"; else fail "not locked: $(shell locked)"; fi
    unlock && ok "it unlocks" || fail "it didn't unlock"
}

modes_check() {
    set +e
    FAILED=0
    SHOTS="${VM}/modes-shots"
    mkdir -p "${SHOTS}"
    rm -f "${SHOTS:?}"/*.png
    local t0 port
    t0=$(now)
    STEP_T=$(now)
    stop > /dev/null
    boot --fresh
    port="$(cat "${VM}/ssh.port")"
    wait_ssh "${port}" root 420 "${VM}/qemu.pid" || { fail "no SSH"; finish "${t0}"; return; }
    check_user
    unlock && ok "unlocked" || fail "didn't unlock"
    check_focus
    check_displays
    check_server
    finish "${t0}"
}
