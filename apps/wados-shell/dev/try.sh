#!/usr/bin/env bash
# The shell for real on a headless labwc the Surface Book 2's size (3000 x
# 2000 at 200%: 1500 x 1000 points), on a wadd of its own holding the
# spike's populated sample (tools/seed-library.py), with screenshots:
#
#   apps/wados-shell/dev/try.sh [shots dir]     (default .build/shell)
#   TRY_KEEP=1 …                                 leave it running; prints how to reach it
#   TRY_OUTPUTS=2 …                              a second display (1920 x 1080 beside it)
#   TRY_NAME=home …                              a session of its own (several can run at once)
#   TRY_FULLSCREEN=1 …                           also open a wadspace full screen (needs the base image)
#
# Shots: launcher.png, desktop.png (and display-2.png, fullscreen.png).
set -euo pipefail
root=$(cd "$(dirname "$0")/../../.." && pwd)
cd "$root"
shots=${1:-$root/.build/shell}
mkdir -p "$shots"
cargo build -q -p wadd -p wados-shell -p wadsctl
export PATH="$root/target/debug:$PATH"
export HL_NAME=${TRY_NAME:-try}
. tools/headless.sh
tmp=$(mktemp -d /tmp/wados-shell-try.XXXXXX)
outputs=${TRY_OUTPUTS:-1}
name=${TRY_NAME:-try}
export HL_NAME=$name
opened=""

cleanup() {
  set +e
  [[ -n "${TRY_KEEP:-}" ]] && return
  in_labwc wadsctl quit >/dev/null 2>&1
  [[ -n "${wadd_pid:-}" ]] && kill "$wadd_pid" 2>/dev/null && wait "$wadd_pid" 2>/dev/null
  # Only what this run opened (other sessions may be running theirs).
  for id in $opened; do
    systemctl --user stop "wad-$id.service" >/dev/null 2>&1
    rm -f "${XDG_RUNTIME_DIR}/containers/systemd/wad-$id.container"
  done
  [[ -n "$opened" ]] && systemctl --user daemon-reload 2>/dev/null
  stop_labwc
  rm -rf "$tmp"
}
trap cleanup EXIT

python3 tools/seed-library.py "$tmp/state" >/dev/null
cat >"$tmp/wadd.toml" <<TOML
[machine]
id = "m-surface"
name = "Surface"
[daemon]
screen_dir = "$tmp/screen"
TOML
start_labwc "$outputs"
in_labwc wlr-randr --output HEADLESS-1 --custom-mode 3000x2000 --scale 2 >/dev/null 2>&1 || true
if [[ "$outputs" -ge 2 ]]; then
  in_labwc wlr-randr --output HEADLESS-2 --custom-mode 1920x1080 --pos 1500,0 >/dev/null 2>&1 || true
fi
in_labwc wlr-randr --output HEADLESS-1 --pos 0,0 >/dev/null 2>&1 || true

export WADD_SOCKET=$tmp/wadd.sock
wadd serve --user --config "$tmp/wadd.toml" --socket "$WADD_SOCKET" --state-dir "$tmp/state" >"$tmp/wadd.log" 2>&1 &
wadd_pid=$!
for _ in $(seq 50); do [[ -S "$WADD_SOCKET" ]] && break; sleep 0.1; done

in_labwc env WADD_SOCKET="$WADD_SOCKET" WADOS_NO_LOCK=1 GSK_RENDERER=cairo WADOS_PRIMARY=HEADLESS-1 \
  wados-shell >"$tmp/shell.log" 2>&1 &
for _ in $(seq 100); do in_labwc wadsctl ping >/dev/null 2>&1 && break; sleep 0.1; done
for _ in $(seq 50); do in_labwc wadsctl wadd 2>/dev/null | grep -q "connected=true" && break; sleep 0.1; done
echo "shell: $(in_labwc wadsctl wadd)"
sleep 1
shot HEADLESS-1 "$shots/launcher.png"
in_labwc wadsctl page desktop >/dev/null
sleep 1.5
shot HEADLESS-1 "$shots/desktop.png"
[[ "$outputs" -ge 2 ]] && shot HEADLESS-2 "$shots/display-2.png"

if [[ -n "${TRY_FULLSCREEN:-}" ]] && podman image exists localhost/wados-base:44; then
  # A wadspace with only the command line runs the base as it is.
  id=$(curl -s --unix-socket "$WADD_SOCKET" -H 'content-type: application/json' \
    -d "{\"op\":{\"op\":\"space.create\",\"name\":\"Try $name\"}}" http://wadd/v1/ops | python3 -c 'import json,sys; print(json.load(sys.stdin)["created"])')
  opened="$id"
  curl -s --unix-socket "$WADD_SOCKET" -H 'content-type: application/json' \
    -d "{\"op\":{\"op\":\"apps.add\",\"ws\":\"$id\",\"app\":\"terminal\"}}" http://wadd/v1/ops >/dev/null
  curl -s --unix-socket "$WADD_SOCKET" -X POST http://wadd/v1/wadspaces/$id/build >/dev/null
  curl -s --unix-socket "$WADD_SOCKET" -H 'content-type: application/json' -d '{"display":"screen"}' \
    http://wadd/v1/wadspaces/$id/open >/dev/null
  for _ in $(seq 60); do
    in_labwc wadsctl wm list | grep -q "wadspace $id .*fullscreen" && break
    sleep 0.5
  done
  echo "windows: $(in_labwc wadsctl wm list)"
  sleep 1
  shot HEADLESS-1 "$shots/fullscreen.png"
fi
echo "shots in $shots"
if [[ -n "${TRY_KEEP:-}" ]]; then
  echo "kept: XDG_RUNTIME_DIR=$hl_rt WAYLAND_DISPLAY=$hl_display WADD_SOCKET=$WADD_SOCKET; logs in $tmp"
fi
