# A headless labwc, for probes and screenshots (no window on your desktop).
# Source it:
#
#   . tools/headless.sh
#   start_labwc [outputs]   sets hl_rt (its runtime dir), hl_display, hl_pid;
#                           `outputs` HEADLESS-n of them (default 1)
#   in_labwc cmd...         runs cmd as a client of it
#   shot OUTPUT file.png    grim of one output
#   stop_labwc
#
# The config is host/etc/xdg/labwc (the machine's), so keybinds and rules are
# the real ones; HL_CONFIG for another, HL_LABWC for another labwc. Logs go
# to $hl_rt/labwc.log. Put target/debug on PATH so its keybinds find wadsctl,
# and run GTK clients with GSK_RENDERER=cairo (there's no GPU).

hl_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
# HL_NAME: a session of its own (several at once, each its own folder).
hl_rt=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/wados-headless${HL_NAME:+-$HL_NAME}
# The patched labwc (tools/build-labwc.sh) when it's built, else the system's
# (which can't tell wadspaces' windows apart: docs/decisions/0001-labwc.md).
hl_labwc=labwc
[ -x "$hl_root/.build/labwc/labwc" ] && hl_labwc=$hl_root/.build/labwc/labwc

start_labwc() {
  local outputs=${1:-1}
  rm -rf "$hl_rt"
  mkdir -m 700 -p "$hl_rt"
  env -u WAYLAND_DISPLAY -u DISPLAY XDG_RUNTIME_DIR="$hl_rt" WLR_BACKENDS=headless WLR_RENDERER=pixman \
    WLR_HEADLESS_OUTPUTS="$outputs" WLR_LIBINPUT_NO_DEVICES=1 \
    "${HL_LABWC:-$hl_labwc}" -C "${HL_CONFIG:-$hl_root/host/etc/xdg/labwc}" >"$hl_rt/labwc.log" 2>&1 &
  hl_pid=$!
  for _ in $(seq 100); do [ -S "$hl_rt/wayland-0" ] && break; sleep 0.05; done
  hl_display=wayland-0
  [ -S "$hl_rt/wayland-0" ] || { echo "labwc didn't start:"; cat "$hl_rt/labwc.log"; return 1; }
}

in_labwc() { env -u DISPLAY XDG_RUNTIME_DIR="$hl_rt" WAYLAND_DISPLAY="$hl_display" "$@"; }

shot() { in_labwc grim -o "$1" "$2"; }

stop_labwc() {
  kill "$hl_pid" 2>/dev/null || true
  wait "$hl_pid" 2>/dev/null || true
  rm -rf "$hl_rt"
}
