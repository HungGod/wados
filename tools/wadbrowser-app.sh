#!/usr/bin/env bash
# WadBrowser's launch, hand-off, modes and sign-in windows, headless
# (the engine's own checks are tools/wadbrowser-e2e.sh).
#
#   tools/wadbrowser-app.sh [OUT] [WADBROWSER]    (cargo build -p wadbrowser -p wad-wayland --examples first)
#
# A fresh HOME and XDG folders under OUT (default /tmp/wadbrowser-app), a
# headless labwc of its own (HL_NAME=wadbrowser-app), the local mock server
# (tools/wadbrowser-app/mock.py), and a D-Bus session of its own
# (dbus-run-session), so nothing reaches a real browser, bus or site. The
# scenario is tools/wadbrowser-app/inner.sh; shots in OUT/shots. Every
# process it starts it stops by PID; whatever the session's bus started and
# left behind is found by its HOME (this run's) and stopped too.
set -u
W=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
A=$W/tools/wadbrowser-app
RUN=${1:-${TMPDIR:-/tmp}/wadbrowser-app}
WB=${2:-$W/target/debug/wadbrowser}
PORT=${APP_PORT:-8771}
[ -x "$WB" ] || { echo "no $WB: cargo build -p wadbrowser"; exit 1; }
rm -rf "$RUN"
mkdir -p "$RUN"/{home,config,data,cache,shots}

# The filter lists, seeded fresh (mtime now), so nothing is downloaded.
mkdir -p "$RUN/cache/wadbrowser/filters"
printf '! probe filters\n||doubleclick.net^\n' >"$RUN/cache/wadbrowser/filters/list-0.txt"
printf '! probe filters\n' >"$RUN/cache/wadbrowser/filters/list-1.txt"
# A web app's picture.
python3 -I - "$RUN/icon.png" <<'EOF'
import struct, sys, zlib
w = h = 32
raw = b"".join(b"\0" + bytes([200, 120, 40, 255]) * w for _ in range(h))
def chunk(t, d):
    return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)
png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")
open(sys.argv[1], "wb").write(png)
EOF

if ss -ltn | grep -q ":$PORT "; then
  echo "port $PORT is taken: a run still going?" >&2
  exit 1
fi

export HL_NAME=${HL_NAME:-wadbrowser-app}
# The machine's labwc config, with Alt+F4 (Close) for closing windows.
HL_CONFIG=$RUN/labwc
mkdir -p "$HL_CONFIG"
sed 's#</keyboard>#<keybind key="A-F4"><action name="Close"/></keybind></keyboard>#' \
  "$W/host/etc/xdg/labwc/rc.xml" >"$HL_CONFIG/rc.xml"
export HL_CONFIG
# shellcheck source=/dev/null
. "$W/tools/headless.sh"
start_labwc || exit 1

python3 -I "$A/mock.py" $PORT "$RUN/http.log" >"$RUN/mock.log" 2>&1 &
mock_pid=$!

# A copy of the scenario (editing it can't change a run under way), its
# output to a file (services the session's bus starts would hold a pipe
# open), and no portals (GTK would start them on this session's bus).
cp "$A/inner.sh" "$RUN/inner.sh"
in_labwc env HOME="$RUN/home" XDG_CONFIG_HOME="$RUN/config" XDG_DATA_HOME="$RUN/data" XDG_CACHE_HOME="$RUN/cache" \
  XDG_STATE_HOME="$RUN/home/.local/state" GSK_RENDERER=cairo GTK_A11Y=none NO_AT_BRIDGE=1 GDK_DEBUG=no-portals \
  WADBROWSER_TEST_LOG="$RUN/test.log" WB="$WB" RUN="$RUN" PORT=$PORT TOPS="$W/target/debug/examples/toplevels" \
  dbus-run-session -- bash "$RUN/inner.sh" >"$RUN/inner.log" 2>&1
grep -E "^(PASS|FAIL|---|      )" "$RUN/inner.log"

kill "$mock_pid" 2>/dev/null
wait "$mock_pid" 2>/dev/null
stop_labwc
# Anything the session's bus started that outlived it.
for d in /proc/[0-9]*; do
  if tr '\0' '\n' <"$d/environ" 2>/dev/null | grep -qx "HOME=$RUN/home"; then
    echo "--- left running, now stopped: ${d#/proc/} $(tr '\0' ' ' <"$d/cmdline" 2>/dev/null)"
    kill "${d#/proc/}" 2>/dev/null
  fi
done 2>/dev/null
echo "--- data folders"
find "$RUN/data" "$RUN/config" -maxdepth 4 -type d | sed "s|$RUN/||" | sort
# Exit status: the number of failed checks.
fails=$(grep -c '^FAIL' "$RUN/inner.log")
echo "--- $(grep -c '^PASS' "$RUN/inner.log") passed, ${fails} failed"
exit "$((fails > 125 ? 125 : fails))"
