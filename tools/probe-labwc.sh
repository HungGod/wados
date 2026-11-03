#!/bin/bash
# Phase 0's probes as checks (docs/decisions/0001-labwc.md), against the
# patched labwc (tools/build-labwc.sh) in a headless session. Each prints
# ok or FAIL; the script exits 1 if any failed.
#
#   cargo build -p wad-wayland --examples -p wados-shell -p wadsctl
#   tools/probe-labwc.sh
set -uo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
export PATH="$root/target/debug:$PATH"
ex=$root/target/debug/examples
. tools/headless.sh
fails=0
check() { if eval "$2"; then echo "ok    $1"; else echo "FAIL  $1"; fails=$((fails + 1)); fi; }

nested_rt=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/wados-probe-nested
nested_cfg=$(mktemp -d)
shell_log=${PROBE_LOG:-$nested_cfg/shell.log}
echo '<?xml version="1.0"?><labwc_config></labwc_config>' >"$nested_cfg/rc.xml"
pids=()
cleanup() {
  for p in "${pids[@]}"; do kill "$p" 2>/dev/null; done
  in_labwc wadsctl quit >/dev/null 2>&1
  stop_labwc
  [ "$fails" -gt 0 ] && for f in "$nested_cfg"/nested-*.log; do echo "--- $f:"; grep -v DEBUG "$f" | tail -5; done
  [ "$fails" -gt 0 ] && { echo "--- the shell's log:"; sed "s/\x1b\[[0-9;]*m//g" "$shell_log" | grep -v "^$" | tail -25; }
  rm -rf "$nested_rt" "$nested_cfg"
}
trap cleanup EXIT

# A wadspace: a nested labwc on a security-context socket of its own.
wadspace() {
  local id=$1
  "$ex/listen" "$hl_rt/wayland-0" "$hl_rt/ws-$id" "io.wadspaces.$id" >/dev/null 2>&1 &
  pids+=($!)
  sleep 0.4
  mkdir -m 700 -p "$nested_rt/$id"
  env -u DISPLAY XDG_RUNTIME_DIR="$nested_rt/$id" WAYLAND_DISPLAY="$hl_rt/ws-$id" WLR_BACKENDS=wayland \
    WLR_RENDERER=pixman labwc -C "$nested_cfg" >"$nested_cfg/nested-$id.log" 2>&1 &
  pids+=($!)
}

globals() { grep -oE "interface: '[a-z0-9_]+'" | sed "s/interface: //; s/'//g" | sort -u; }

start_labwc 2 || exit 1
in_labwc env WADOS_NO_LOCK=1 GSK_RENDERER=cairo WADOS_PRIMARY=HEADLESS-1 WADOS_PAM_SERVICE=login \
  wados-shell --builder >"$shell_log" 2>&1 &
pids+=($!)
for _ in $(seq 50); do in_labwc wadsctl ping >/dev/null 2>&1 && break; sleep 0.2; done
# The desktop: on the launcher, its chrome holds the keyboard and covers the windows.
in_labwc wadsctl page desktop >/dev/null
sleep 1.2

# 1. A wadspace's socket hides what's privileged, and nothing offers shortcut inhibit.
"$ex/listen" "$hl_rt/wayland-0" "$hl_rt/ws-globals" io.wadspaces.globals >/dev/null 2>&1 &
pids+=($!)
sleep 0.4
sandboxed=$(env -u DISPLAY XDG_RUNTIME_DIR="$hl_rt" WAYLAND_DISPLAY=ws-globals wayland-info 2>/dev/null | globals)
everyone=$(in_labwc wayland-info 2>/dev/null | globals)
for g in zwlr_screencopy_manager_v1 ext_image_copy_capture_manager_v1 zwlr_data_control_manager_v1 \
  ext_data_control_manager_v1 zwlr_foreign_toplevel_manager_v1 ext_foreign_toplevel_list_v1 zwlr_layer_shell_v1 \
  zwp_virtual_keyboard_manager_v1 zwlr_virtual_pointer_manager_v1 ext_session_lock_manager_v1 \
  zwlr_output_manager_v1 zwp_input_method_manager_v2 wp_security_context_manager_v1; do
  check "a wadspace can't see $g" "! grep -qx $g <<<\"\$sandboxed\""
done
check "a wadspace still has xdg_wm_base and wl_seat" "grep -qx xdg_wm_base <<<\"\$sandboxed\" && grep -qx wl_seat <<<\"\$sandboxed\""
check "nobody is offered keyboard-shortcuts-inhibit" "! grep -q keyboard_shortcuts_inhibit <<<\"\$everyone\""

# 2. Which window is which wadspace (patch 0001).
wadspace vanua
in_labwc foot --app-id=io.wados.Terminal >/dev/null 2>&1 &
pids+=($!)
sleep 2
list=$(in_labwc wadsctl wm list)
check "a wadspace's window reports io.wadspaces.<id>" "grep -q 'wadspace vanua' <<<\"\$list\""
check "a host app keeps its own app id" "grep -q 'io.wados.Terminal' <<<\"\$list\""

# 3. Full screen on the display asked for (patch 0002), and back.
on() { in_labwc wadsctl wm list | tr ';' '\n' | grep "wadspace vanua"; }
in_labwc wadsctl wm fullscreen io.wadspaces.vanua HEADLESS-1 >/dev/null
sleep 0.8
check "full screen on HEADLESS-1" "on | grep -q '\[HEADLESS-1\] .*fullscreen'"
in_labwc wadsctl wm fullscreen io.wadspaces.vanua HEADLESS-2 >/dev/null
sleep 0.8
check "then on HEADLESS-2" "on | grep -q '\[HEADLESS-2\] .*fullscreen'"
in_labwc wadsctl wm unfullscreen io.wadspaces.vanua >/dev/null
sleep 0.5
check "then in a window" "! on | grep -q fullscreen"

# 4. Keybinds reach the shell, and the move-to-display chord moves a window.
in_labwc wtype -M logo -k Tab -m logo
sleep 0.4
in_labwc wtype -M alt -k space -m alt
sleep 0.4
log=$(in_labwc wadsctl log)
check "Super+Tab reaches the shell" "grep -q 'switcher next' <<<\"\$log\""
check "Alt+Space reaches the shell" "grep -q 'search' <<<\"\$log\""
term_on() { in_labwc wadsctl wm list | tr ';' '\n' | grep Terminal | grep -oE '\[[^]]*\]'; }
before=$(term_on)
in_labwc wadsctl wm activate io.wados.Terminal >/dev/null
sleep 0.3
in_labwc wtype -M logo -M ctrl -M alt -M shift -k F13 -m shift -m alt -m ctrl -m logo
sleep 0.6
after=$(term_on)
check "the chord moves a window to the other display ($before -> $after)" "[ \"$before\" != \"$after\" ]"

# 5. A drag from the Builder lands on the desktop.
in_labwc wadsctl wm close io.wados.Terminal >/dev/null
in_labwc wadsctl wm close io.wadspaces.vanua >/dev/null
sleep 1.5
builder=$(in_labwc wadsctl wm list | tr ';' '\n' | grep '"Builder"' | grep -oE '\[[^]]*\]')
if [ "$builder" = "[HEADLESS-1]" ]; then x0=1280; else x0=0; fi
# The Builder (520x420) is centred on its display; its first piece is 100 px down.
in_labwc "$ex/pointer" 2560 720 drag $((x0 + 450)) 275 $((1280 + 150)) 600
sleep 0.5
check "a piece dragged from the Builder lands on the desktop" "in_labwc wadsctl log | grep -qE 'dropped \"(app|web|key):'"

# 6. The lock screen locks, and refuses a wrong password.
in_labwc wadsctl lock >/dev/null
sleep 1
check "the screen locks" "in_labwc wadsctl locked | grep -q true"
in_labwc wtype "not-the-password"
in_labwc wtype -k Return
sleep 3.5
check "a wrong password doesn't unlock it" "in_labwc wadsctl locked | grep -q true && grep -q 'unlock refused' \"\$shell_log\""

echo
[ "$fails" -eq 0 ] && echo "all probes passed" || echo "$fails failed"
exit $((fails > 0))
