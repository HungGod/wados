# tools/vm.sh check: the host image end to end, in the VM (sourced by
# tools/vm.sh, whose helpers it uses). Every step says ok or FAIL, and the
# check goes on after a FAIL where it can. Screenshots: .build/vm/shots/.
#
#   1. it boots from a fresh layer over wados.qcow2; root's SSH works
#   2. sam (uid 1000), made as the installer will (phase 9); greetd logs
#      them in: labwc, the shell, and the screen locked
#   3. a wrong password doesn't unlock it; sam's does (typed through QMP),
#      and the shell's screen shows (tools/vm/screen.py: not blank); the
#      desktop's icon is below the bar; a second Lock and unlock
#   4. a wadspace with the terminal and a web app, through wadd's socket, as
#      sam: built (it runs the base as it is), on the screen, its window
#      there; a tone played in it reaches the machine's PipeWire, and the
#      VM's sound card (.build/vm/sound.wav)
#   5. WadBrowser in sam's session: a page served in the VM, drawn
#      (screen.py --ink); a second wadbrowser hands its page over (still one
#      process); a web app's window (wadspaces-webapp-<id>); and in the
#      wadspace, the browser and its web app's launcher
#   6. the machine's session (WADOS_SESSION=machine): the power key opens
#      the shell's menu, the volume keys, the brightness (none in a VM), the
#      idle lock, logind's lock-session
#   7. Restart, through the shell and logind: it comes back to the lock screen
#   8. Power off, through the shell and logind: QEMU ends
#   9. booted again: suspend locks the screen within logind's 5 s, a key
#      wakes it, and QMP's quit ends it (VM_KEEP=1: it keeps running)

ok() { printf 'ok    %s\n' "$*"; }
fail() { printf 'FAIL  %s\n' "$*"; FAILED=$((FAILED + 1)); }
note() { printf '      %s\n' "$*"; }
indent() { sed 's/^/      | /'; }
step_time() { printf '      (%d s)\n' "$(($(now) - STEP_T))"; STEP_T=$(now); }

as_root() { ssh_to "$(cat "${VM}/ssh.port")" root "$@"; }
as_sam() { ssh_to "$(cat "${VM}/ssh.port")" sam "$@"; }
# What the shell says (wadsctl, in sam's session).
shell() { as_sam "XDG_RUNTIME_DIR=/run/user/1000 wadsctl $*" 2> /dev/null; }
# wadd's API through its socket, as sam: api METHOD PATH [BODY].
api() {
    local body="${3:-}"
    if [[ -n "${body}" ]]; then
        as_sam "curl -sS --unix-socket /run/wadd/wadd.sock -X $1 -H 'content-type: application/json' --data-binary @- http://wadd$2" <<< "${body}"
    else
        as_sam "curl -sS --unix-socket /run/wadd/wadd.sock -X $1 http://wadd$2" < /dev/null
    fi
}

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
locked_is() { [[ "$(shell locked)" == "ok $1" ]]; }
vm_down() { ! as_root true 2> /dev/null; }
vm_up() { as_root true 2> /dev/null; }
qemu_gone() { ! running "${VM}/qemu.pid"; }

shot_to() {
    if qmp shot "${SHOTS}/$1" 2> /dev/null; then note "screenshot: ${SHOTS}/$1"; else note "(no screenshot: $1)"; fi
}

# What a failure leaves to look at.
journal() {
    as_root "journalctl -b --no-pager -o short-monotonic $* | tail -n 25" 2> /dev/null | indent
}

# Types sam's password on the lock screen; true once it's unlocked.
unlock() {
    qmp keys "${SAM_PASSWORD}" && qmp key ret && wait_for 30 locked_is false
}

check() {
    set +e
    FAILED=0
    SLEPT=0
    SHOTS="${VM}/shots"
    mkdir -p "${SHOTS}"
    rm -f "${SHOTS}"/*.png
    local t0 port
    t0=$(now)
    STEP_T=$(now)

    say "1. it boots, and root's SSH works"
    stop > /dev/null
    boot --fresh
    port="$(cat "${VM}/ssh.port")"
    if wait_ssh "${port}" root 420 "${VM}/qemu.pid"; then
        ok "root's SSH after $(($(now) - STEP_T)) s ($(as_root 'sed -n "s/^PRETTY_NAME=//p" /etc/os-release' | tr -d '"'))"
        as_root 'bootc status --format=humanreadable 2> /dev/null | sed -n "1,3p"; echo "system: $(systemctl is-system-running)"; systemctl --failed --no-legend --plain' 2>&1 | indent
    else
        fail "no SSH within 420 s (${VM}/serial.log)"
        tail -n 30 "${VM}/serial.log" | indent
        shot_to 0-no-ssh.png
        finish "${t0}"
        return
    fi
    step_time

    check_user
    check_unlock
    check_wadspace
    check_browser
    check_session
    check_restart
    check_power_off
    check_suspend
    finish "${t0}"
}

check_user() {
    say "2. sam (uid 1000), made as the installer will; greetd logs them in, locked"
    if as_root bash -s -- "${SAM_PASSWORD}" << 'EOF'
set -e
groups=wheel
getent group wados > /dev/null && groups+=,wados
getent passwd 1000 > /dev/null || useradd -m -u 1000 -U -G "${groups}" -c Sam sam
echo "sam:$1" | chpasswd
install -d -m 0700 -o sam -g sam ~sam/.ssh
install -m 0600 -o sam -g sam /root/.ssh/authorized_keys ~sam/.ssh/authorized_keys
# A file on the desktop: an icon to find in a screenshot (step 3).
install -d -m 0755 -o sam -g sam ~sam/Desktop
echo "WadOS in a VM" | install -m 0644 -o sam -g sam /dev/stdin ~sam/Desktop/Notes.txt
systemctl reset-failed greetd 2> /dev/null || true
systemctl restart greetd
EOF
    then
        ok "sam made, and greetd started again"
    else
        fail "making sam, or starting greetd again"
        journal -u greetd
    fi
    if as_sam true 2> /dev/null; then ok "sam is uid $(as_sam id -u), in $(as_sam id -Gn | tr ' ' ',')"; else fail "sam can't SSH in"; fi
    if wait_for 240 locked_is true; then
        ok "labwc and the shell are up in sam's session, and it's locked (wadsctl locked: $(shell locked))"
    else
        fail "the shell didn't say it's locked (wadsctl locked: $(shell locked || true))"
        journal -u greetd -t labwc -t wados-session
        as_root 'loginctl list-sessions --no-legend; pgrep -a labwc; pgrep -a wados-shell' 2>&1 | indent
    fi
    note "session: $(shell session)"
    sleep 3
    shot_to 1-locked.png
    step_time
}

check_unlock() {
    say "3. unlock: a wrong password first, then sam's"
    qmp keys 'not-the-password'
    qmp key ret
    # pam_faildelay: two seconds before the answer.
    sleep 6
    if locked_is true; then ok "a wrong password doesn't unlock it"; else fail "not locked after the wrong password ($(shell locked))"; fi
    shot_to 2-wrong.png
    if unlock; then
        ok "sam's password unlocks it (wadsctl locked: $(shell locked))"
    else
        fail "still locked after sam's password ($(shell locked || echo 'no answer: is the shell running?'))"
        journal -t labwc -t unix_chkpwd
    fi
    sleep 5
    shot_to 3-unlocked.png
    # Unlocked, the shell's own screen shows: the launcher, its bar.
    local drawn
    if drawn="$(python3 "${HERE}/screen.py" "${SHOTS}/3-unlocked.png" 2>&1)"; then
        ok "the shell's screen shows once it's unlocked (${drawn})"
    else
        fail "the screen is blank once it's unlocked (${drawn}; wadsctl page: $(shell page))"
    fi
    check_desktop_below_bar
    # Locked and unlocked again (Lock, as the start menu does it).
    if [[ "$(shell lock)" == ok* ]] && wait_for 20 locked_is true && unlock; then
        sleep 5
        shot_to 3-unlocked-again.png
        if drawn="$(python3 "${HERE}/screen.py" "${SHOTS}/3-unlocked-again.png" 2>&1)"; then
            ok "locked and unlocked again (wadsctl lock), and the screen is drawn (${drawn})"
        else
            fail "blank after the second unlock (${drawn})"
        fi
    else
        fail "a second lock and unlock: $(shell locked)"
    fi
    step_time
}

# The desktop keeps below the bar (its reserve, 78 px: docs/decisions/0001)
# once it's unlocked: where its icon is drawn, found as what changes when
# it's selected.
check_desktop_below_bar() {
    local key want box top
    shell page desktop > /dev/null
    sleep 4
    key="$(shell desk icons | sed -E 's/^ok //; s/@.*//; s/ ;.*//')"
    if [[ -z "${key}" || "${key}" == error* ]]; then
        fail "no icon on the desktop to find ($(shell desk icons))"
        return
    fi
    want="$(shell desk where "${key}")"
    shell desk select none > /dev/null
    sleep 1
    qmp shot "${SHOTS}/3-desktop.png"
    shell desk select all > /dev/null
    sleep 1
    qmp shot "${SHOTS}/3-desktop-selected.png"
    shell desk select none > /dev/null
    note "screenshots: ${SHOTS}/3-desktop.png, 3-desktop-selected.png"
    # Its first column: the clock and the rest of the bar are to the right.
    box="$(python3 "${HERE}/screen.py" "${SHOTS}/3-desktop.png" "${SHOTS}/3-desktop-selected.png" 0 0 400 1079)"
    top="$(awk '{ print $2 }' <<< "${box}")"
    if [[ "${top}" =~ ^[0-9]+$ ]] && ((top >= 78)); then
        ok "the desktop's icon (${key}) is drawn from y=${top}, below the bar's 78 px (box ${box}; the shell has its middle at ${want#ok })"
    else
        fail "the desktop's icon (${key}): ${box} (the shell has it at ${want#ok }): under the bar?"
    fi
    shell page home > /dev/null
    sleep 2
}

check_wadspace() {
    say "4. a wadspace with the terminal and a web app, on the screen, with sound"
    local screen id b st phase opened wins
    if [[ "$(api GET /v1/health)" == ok ]]; then ok "wadd answers sam on /run/wadd/wadd.sock"; else fail "wadd doesn't answer sam"; journal -u wadd -u wadd.socket; fi
    screen="$(api GET /v1/screen)"
    note "screen: ${screen}"
    if [[ "$(jq -r .from <<< "${screen}" 2> /dev/null)" == shell ]]; then ok "the shell told wadd where its screen is"; else fail "wadd hasn't the shell's screen"; fi
    id="$(api POST /v1/ops '{"op": {"op": "space.create", "name": "VM Terminal"}}' | jq -r .created 2> /dev/null)"
    [[ -n "${id}" && "${id}" != null ]] || { fail "no wadspace made"; step_time; return; }
    api POST /v1/ops "{\"op\": {\"op\": \"apps.add\", \"ws\": \"${id}\", \"app\": \"terminal\"}}" > /dev/null
    # A web app too (WadBrowser comes with it): it arrives at start, never builds (step 5 looks for its launcher).
    api POST /v1/ops "{\"op\": {\"op\": \"builder.addWebApp\", \"ws\": \"${id}\", \"name\": \"VM page\", \"url\": \"http://127.0.0.1:8799/\"}}" > /dev/null
    b="$(api POST "/v1/wadspaces/${id}/build")"
    if [[ "$(jq -r .state.built <<< "${b}")" == built && "$(jq -r .job <<< "${b}")" == null ]]; then
        ok "${id}: built (nothing to build: it runs ${BASE_IMAGE} as it is)"
    else
        fail "building ${id}: ${b}"
    fi
    api POST "/v1/wadspaces/${id}/open" '{"display": "screen"}' | jq -c '{phase, message, display}' | indent
    opened=$(now)
    for _ in $(seq 300); do
        st="$(api GET /v1/runs | jq -c --arg id "${id}" '.[] | select(.id == $id)')"
        phase="$(jq -r .phase <<< "${st}")"
        [[ "${phase}" == ready || "${phase}" == error ]] && break
        sleep 2
    done
    if [[ "${phase}" == ready ]]; then
        ok "${id} is ready after $(($(now) - opened)) s (its desktop is up)"
    else
        fail "${id}: ${st}"
        journal -u wadd -u "wad-${id}"
    fi
    as_root "grep -E '^(Environment=(WLR_RENDERER|PULSE_SERVER)|UIDMap|SecurityLabelLevel)' /run/containers/systemd/wad-${id}.container | tr '\n' ' '; echo" 2> /dev/null | indent
    for _ in $(seq 20); do
        wins="$(shell wm list)"
        grep -q "wadspace ${id} " <<< "${wins}" && break
        sleep 2
    done
    note "wm list: ${wins}"
    if grep -q "wadspace ${id} " <<< "${wins}"; then ok "its window is on the screen (io.wadspaces.${id})"; else fail "no window for ${id}"; fi
    sleep 4
    shot_to 4-wadspace.png
    api GET '/v1/logs?n=400' | jq -r '.[].message' 2> /dev/null | grep -E "^${id}: " | sed 's/^/      log: /'

    local from played heard
    from="$(stat -c %s "${VM}/sound.wav" 2> /dev/null || echo 0)"
    played="$(as_root "podman exec -i --user abc wad-${id} python3 - 3 440" < "${HERE}/tone.py" 2>&1)"
    note "in it: ${played}"
    sleep 2
    if [[ "${played}" == played* ]]; then ok "it played a tone through its PulseAudio socket (PULSE_SERVER)"; else fail "it couldn't play: ${played}"; fi
    if heard="$(python3 "${HERE}/wav.py" "${VM}/sound.wav" "${from}")"; then
        ok "the VM's sound card played it: ${heard}"
    else
        fail "nothing heard in sound.wav: ${heard}"
        as_sam 'XDG_RUNTIME_DIR=/run/user/1000 wpctl status 2> /dev/null | sed -n "/Sinks:/,/Sources:/p"' | indent
    fi
    as_root "journalctl -b --no-pager -g 'avc:  denied' | grep -c container_t" 2> /dev/null | sed 's/^/      SELinux denials for container_t this boot: /'
    step_time
}

# In sam's session, as the shell starts things (a user unit, so it outlives ssh).
in_session() {
    local unit=$1
    shift
    as_sam "XDG_RUNTIME_DIR=/run/user/1000 systemd-run --user --quiet --collect --unit ${unit} --setenv=WAYLAND_DISPLAY=wayland-0 $*"
}
browser_procs() { as_sam 'pgrep -x -u sam wadbrowser | wc -l' 2> /dev/null; }

check_browser() {
    say "5. WadBrowser: on the machine (a page, a second launch handed over, a web app's window), and in a wadspace"
    local wins drawn n ws launchers
    # Pages served in the VM itself: nothing from the internet. Huge black
    # text on white, so a page that's drawn shows (screen.py --ink).
    as_sam 'mkdir -p /tmp/wb && for p in index:WadOS-VM-page two:Second-page; do
        printf "<!doctype html><title>%s</title><body style=\"margin:0;background:#fff;color:#000;font:bold 160px sans-serif\">%s</body>\n" "${p#*:}" "${p#*:}" > "/tmp/wb/${p%%:*}.html"; done' 2> /dev/null
    in_session wb-pages python3 -m http.server 8799 --bind 127.0.0.1 --directory /tmp/wb 2> /dev/null
    in_session wb-browser wadbrowser http://127.0.0.1:8799/index.html 2> /dev/null
    for _ in $(seq 30); do
        wins="$(shell wm list)"
        grep -q 'io.wados.Browser "[^"]*WadOS-VM-page' <<< "${wins}" && break
        sleep 2
    done
    note "wm list: ${wins}"
    if grep -q 'io.wados.Browser "[^"]*WadOS-VM-page' <<< "${wins}"; then
        ok "WadBrowser opened a page (io.wados.Browser, its title the page's)"
    else
        fail "no WadBrowser window with the page"
        as_sam 'XDG_RUNTIME_DIR=/run/user/1000 journalctl --user -u wb-browser --no-pager | tail -n 20' 2> /dev/null | indent
    fi
    sleep 3
    shot_to 5-browser.png
    if drawn="$(python3 "${HERE}/screen.py" --ink "${SHOTS}/5-browser.png" 2>&1)"; then ok "the page is drawn (${drawn})"; else fail "the page isn't drawn: ${drawn}"; fi

    if as_sam 'XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-0 timeout 20 wadbrowser http://127.0.0.1:8799/two.html' 2> /dev/null; then
        ok "a second wadbrowser handed its page over and exited 0"
    else
        fail "the second wadbrowser failed"
    fi
    sleep 3
    wins="$(shell wm list)"
    n="$(browser_procs)"
    if [[ "${n}" == 1 ]] && grep -q 'Second-page' <<< "${wins}"; then
        ok "still one browser, and it shows the second page (a new tab)"
    else
        fail "browsers: ${n}; wm list: ${wins}"
    fi
    shot_to 5-browser-second.png
    if drawn="$(python3 "${HERE}/screen.py" --ink "${SHOTS}/5-browser-second.png" 2>&1)"; then ok "the second tab is drawn (${drawn})"; else fail "the second tab isn't drawn: ${drawn}"; fi

    as_sam 'XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-0 timeout 20 wadbrowser --app vmpage --name "VM page" --url http://127.0.0.1:8799/index.html' 2> /dev/null
    for _ in $(seq 15); do
        wins="$(shell wm list)"
        grep -q 'wadspaces-webapp-vmpage' <<< "${wins}" && break
        sleep 2
    done
    if grep -q 'wadspaces-webapp-vmpage' <<< "${wins}"; then ok "a web app's window (wadspaces-webapp-vmpage)"; else fail "no web app window: ${wins}"; fi
    sleep 2
    shot_to 5-web-app.png
    if drawn="$(python3 "${HERE}/screen.py" --ink "${SHOTS}/5-web-app.png" 2>&1)"; then ok "the web app's page is drawn (${drawn})"; else fail "the web app's page isn't drawn: ${drawn}"; fi
    as_sam 'XDG_RUNTIME_DIR=/run/user/1000 systemctl --user stop wb-browser wb-pages; pkill -x -u sam wadbrowser; true' 2> /dev/null
    for _ in $(seq 10); do [[ "$(browser_procs)" == 0 ]] && break; sleep 1; done
    if [[ "$(browser_procs)" == 0 ]]; then ok "closed"; else fail "wadbrowser is still running"; fi

    # In a wadspace: the browser is in the base image, and a web app gets its launcher.
    ws="$(api GET /v1/wadspaces | jq -r '.[] | select(.name == "VM Terminal") | .id' 2> /dev/null | head -1)"
    if [[ -n "${ws}" ]]; then
        note "in ${ws}: $(as_root "podman exec --user abc wad-${ws} wadbrowser --version" 2>&1 | head -1)"
        launchers="$(as_root "podman exec wad-${ws} sh -c 'ls /usr/share/applications/ | grep -E \"^(io.wados.Browser|wadspaces-webapp-)\"'" 2> /dev/null | tr '\n' ' ')"
        if grep -q 'wadspaces-webapp-' <<< "${launchers}" && grep -q 'io.wados.Browser' <<< "${launchers}"; then
            ok "the wadspace has WadBrowser and its web app's launcher (${launchers})"
        else
            fail "launchers in ${ws}: ${launchers:-none}"
        fi
    else
        fail "no VM Terminal wadspace to look in"
    fi
    step_time
}

check_session() {
    say "6. the machine's session: power key, volume, brightness, idle, lock-session"
    local s v0 v1 b sid
    s="$(shell session)"
    note "session: ${s}"
    if [[ "${s}" == *"machine=true"* ]]; then ok "the shell knows it's the machine's session"; else fail "not the machine's session: ${s}"; fi
    if [[ "${s}" =~ idle_lock=([0-9]+s) ]]; then ok "the idle lock is on (after ${BASH_REMATCH[1]})"; else fail "no idle lock: ${s}"; fi
    if [[ "${s}" == *"inhibitor=held"* ]]; then ok "it holds logind's sleep inhibitor"; else fail "no sleep inhibitor: ${s}"; fi

    # The power key: logind ignores it (HandlePowerKey=ignore); labwc runs wadsctl power.
    qmp key power
    sleep 3
    s="$(shell power state)"
    if [[ "${s}" == "ok open"* ]] && ! qemu_gone && as_root true 2> /dev/null; then
        ok "the power key opens the shell's power menu, and nothing powers off (${s})"
    else
        fail "the power key: ${s:-no answer} (QEMU $(qemu_gone && echo gone || echo running))"
    fi
    shot_to 5-power-menu.png
    shell power close > /dev/null

    # The volume keys, with real PipeWire.
    local vol='XDG_RUNTIME_DIR=/run/user/1000 wpctl get-volume @DEFAULT_AUDIO_SINK@'
    v0="$(as_sam "${vol}" 2> /dev/null)"
    if [[ "${v0}" == "Volume: 1.00"* ]]; then shell volume down > /dev/null; else shell volume up > /dev/null; fi
    sleep 2
    v1="$(as_sam "${vol}" 2> /dev/null)"
    if [[ -n "${v0}" && "${v0}" != "${v1}" ]]; then ok "the volume keys change PipeWire's volume (${v0} → ${v1})"; else fail "the volume didn't change (${v0:-none} → ${v1:-none})"; fi

    b="$(shell brightness)"
    if [[ "${b}" == "error this screen has no backlight"* ]]; then ok "brightness: no backlight in a VM, as expected (${b})"; else note "brightness: ${b}"; fi

    # logind's Lock.
    sid="$(as_root "loginctl list-sessions --no-legend | awk '\$3 == \"sam\" && \$4 == \"seat0\" { print \$1; exit }'" 2> /dev/null)"
    if [[ -n "${sid}" ]] && as_root "loginctl lock-session ${sid}" && wait_for 20 locked_is true; then
        ok "loginctl lock-session ${sid} locks the shell"
    else
        fail "loginctl lock-session ${sid:-(sam has no seat session)}: $(shell locked)"
    fi
    if unlock; then ok "unlocked again"; else fail "couldn't unlock again"; fi

    note "session: $(shell session)"
    step_time
}

check_restart() {
    say "7. Restart through the shell (logind): it comes back to the lock screen"
    local r port
    port="$(cat "${VM}/ssh.port")"
    r="$(shell power restart)"
    note "wadsctl power restart: ${r}"
    if wait_for 90 vm_down; then
        ok "it went down (logind's Reboot, asked by the shell)"
    else
        fail "it didn't restart: $(shell session)"
        journal -t labwc -t polkitd -u systemd-logind
        step_time
        return
    fi
    if wait_ssh "${port}" root 420 "${VM}/qemu.pid" && wait_for 240 locked_is true; then
        ok "after the restart, sam's session is back and locked ($(shell locked))"
    else
        fail "after the restart: $(shell locked || echo 'no shell')"
        journal -u greetd -t labwc
    fi
    sleep 3
    shot_to 6-restarted.png
    if unlock; then ok "unlocked after the restart"; else fail "couldn't unlock after the restart"; fi
    step_time
}

check_suspend() {
    say "9. locking before suspend: within logind's 5 s, and the inhibitor back after waking"
    # A boot of its own, the last: once this VM has slept and woken, QEMU's
    # virtio-vga hangs the next boot's GPU probe, whether it restarts or
    # powers off (seen three times), so the VM ends with QMP's quit. It
    # suspends to idle (no S3: OVMF's resume from it resets the VM, vm.sh),
    # and its keyboard wakes it: Shift, which types nothing into the lock's
    # password.
    local t_go t_ok boot0 boot1 t0 lines port
    if ! running "${VM}/qemu.pid"; then
        boot > /dev/null
        port="$(cat "${VM}/ssh.port")"
        if wait_ssh "${port}" root 420 "${VM}/qemu.pid" && wait_for 240 locked_is true && unlock; then
            ok "booted again, locked, and unlocked with sam's password"
        else
            fail "booting again for the suspend: $(shell locked || echo 'no shell')"
            step_time
            return
        fi
    fi
    SLEPT=1
    boot0="$(as_root 'cat /proc/sys/kernel/random/boot_id; echo enabled > /sys/devices/platform/i8042/serio0/power/wakeup' 2> /dev/null)"
    t0="$(as_root 'date +%s' 2> /dev/null)"
    as_root 'systemctl suspend --no-block' > /dev/null 2>&1
    if wait_for 30 vm_down; then
        sleep 5
        qmp key shift
        if wait_for 90 vm_up; then
            boot1="$(as_root 'cat /proc/sys/kernel/random/boot_id' 2> /dev/null)"
            if [[ "${boot1}" == "${boot0}" ]]; then
                ok "it went to sleep, and a key woke it ($(as_root "journalctl -b --no-pager -k -o cat --since=@${t0} | grep -aE '^PM: suspend (entry|exit)' | tr '\n' ' '" 2> /dev/null))"
            else
                fail "it started again instead of waking (boot ${boot0:0:8} → ${boot1:0:8})"
            fi
        else
            fail "no SSH after the key that should wake it"
        fi
    else
        fail "it didn't go to sleep within 30 s"
    fi
    if wait_for 20 locked_is true; then ok "it's locked after waking"; else fail "not locked after waking ($(shell locked))"; fi
    # The shell's lines (journald keeps them as data: they carry colours).
    lines="$(as_root "journalctl -b -o short-unix --no-pager --since=@${t0} | grep -aE 'going to sleep, so locking first|locked: sleep can go on' | sed -E 's/\x1b\[[0-9;]*m//g'" 2> /dev/null)"
    t_go="$(grep -m1 'going to sleep' <<< "${lines}" | awk '{ print $1 }')"
    t_ok="$(grep -m1 'sleep can go on' <<< "${lines}" | awk '{ print $1 }')"
    if [[ "${t_go}" =~ ^[0-9.]+$ && "${t_ok}" =~ ^[0-9.]+$ ]]; then
        local took
        took="$(python3 -c "print(f'{${t_ok} - ${t_go}:.2f}')")"
        if python3 -c "import sys; sys.exit(0 if ${t_ok} - ${t_go} < 5 else 1)"; then
            ok "the lock showed ${took} s after logind said it's going to sleep (within its 5 s)"
        else
            fail "the lock took ${took} s: logind waits 5 s"
        fi
    else
        fail "the journal hasn't both lines (going to sleep: ${t_go:-none}, sleep can go on: ${t_ok:-none})"
        journal -u systemd-logind
    fi
    sleep 2
    if as_root 'systemd-inhibit --list --no-legend --no-pager' 2> /dev/null | grep -E 'WadSpaces' | grep -E 'sleep' | grep -q 'delay'; then
        ok "the sleep inhibitor is back after waking (WadSpaces … sleep … delay)"
    else
        fail "no WadSpaces sleep inhibitor after waking"
        as_root 'systemd-inhibit --list --no-pager' 2>&1 | indent
    fi
    shot_to 8-after-suspend.png
    if unlock; then ok "unlocked again"; else fail "couldn't unlock after the suspend"; fi
    note "session: $(shell session)"
    step_time
}

check_power_off() {
    say "8. Power off through the shell (logind)"
    local r
    r="$(shell power off)"
    note "wadsctl power off: ${r}"
    if wait_for 120 qemu_gone; then ok "it powered off (QEMU ended)"; else fail "it didn't power off: $(shell session)"; fi
    step_time
}

finish() {
    say "the result ($(($(now) - $1)) s)"
    # A VM that has slept doesn't shut down cleanly (check_suspend): its
    # layer is thrown away anyway.
    if [[ "${SLEPT:-0}" == 1 && "${VM_KEEP:-0}" != 1 ]] && running "${VM}/qemu.pid"; then
        qmp quit 2> /dev/null
    fi
    if [[ "${VM_KEEP:-0}" == 1 ]]; then
        echo "the VM keeps running (VM_KEEP=1): tools/vm.sh ssh, tools/vm.sh stop"
    else
        stop > /dev/null
    fi
    [[ ${FAILED} -eq 0 ]] && echo "all ok" || echo "${FAILED} failed"
    exit $((FAILED > 0))
}
