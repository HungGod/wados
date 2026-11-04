#!/usr/bin/env bash
# WadBrowser's engine, end to end, in a headless labwc (tools/headless.sh)
# against a local server (tools/wadbrowser-e2e-server.py): no real sites,
# never your HOME.
#
#   tools/wadbrowser-e2e.sh [OUT]     (cargo build -p wadbrowser first)
#   E2E_ONLY="popup key" tools/wadbrowser-e2e.sh    some of the checks
#
# OUT (default /tmp/wadbrowser-e2e) holds each run's HOME and logs. The
# checks:
#   cookies   a persistent cookie survives a restart; a private window's doesn't
#   filter    the content filter compiles once (its time logged) and the next
#             start only loads it; a blocked image is blocked and counted, an
#             allowed one isn't, the hiding rule hides
#   popup     a popup keeps window.opener: it posts to its opener and closes,
#             and its tab goes
#   key       a page on a key's site moves into the key's session, shown by
#             a cookie only that session has (WADBROWSER_SIGNINS); on the
#             machine (no signins.json) the keys and their folders are wadd's
#             (a fake wadd on WADD_SOCKET: tools/wadbrowser-e2e-wadd.py), and
#             a tab on the key's site uses that folder's cookies
#   headers   DNT and Sec-GPC on a load the browser starts
#   download  saved under WebKit's suggested name; no error page for the
#             load it interrupted
#   error     a real failure gets an error page, at the address that failed
#   pixels    a first tab, a second launch's tab and an --app window draw
#             their page (huge black text on white, counted in grim shots)
#
# Runs are without a settings portal (GDK_DEBUG=no-portals), as in a
# wadspace with none: GTK then has no font DPI, which once left every view
# after the first drawn at a zoom of NaN (pixels catches it).
#
# Pages report through WADBROWSER_TEST_LOG (titles, tabs, the filter). It
# stops everything it starts, by PID. Exit status: the number of failures.
set -u
W=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
OUT=${1:-${TMPDIR:-/tmp}/wadbrowser-e2e}
BIN=${WADBROWSER_BIN:-$W/target/debug/wadbrowser}
PORT=${E2E_PORT:-8791}
[ -x "$BIN" ] || { echo "no $BIN: cargo build -p wadbrowser"; exit 1; }
rm -rf "$OUT"
mkdir -p "$OUT"
fails=0
want() { [ -z "${E2E_ONLY:-}" ] || [[ " $E2E_ONLY " == *" $1 "* ]]; }
# How long a page may take (a loaded laptop starts WebKit slowly).
T=${E2E_WAIT:-45}
pass() { echo "PASS $*"; }
fail() { echo "FAIL $*"; fails=$((fails + 1)); }

# The test lists: one network rule, one hiding rule.
LISTS=$OUT/lists.txt
printf '! WadBrowser e2e\n/wb-ad-banner.\n##.wb-ad-box\n' >"$LISTS"

export HL_NAME=${HL_NAME:-engine}
# A scratch config with Alt+F4 (Close), else the machine's.
if [ -z "${HL_CONFIG:-}" ]; then
  HL_CONFIG=$OUT/labwc
  mkdir -p "$HL_CONFIG"
  sed 's#</keyboard>#<keybind key="A-F4"><action name="Close"/></keybind></keyboard>#' \
    "$W/host/etc/xdg/labwc/rc.xml" >"$HL_CONFIG/rc.xml"
fi
export HL_CONFIG
# shellcheck source=/dev/null
. "$W/tools/headless.sh"
if ! start_labwc; then
  # A loaded laptop can take longer than headless.sh waits.
  for _ in $(seq 300); do [ -S "$hl_rt/wayland-0" ] && break; sleep 0.1; done
  [ -S "$hl_rt/wayland-0" ] || { stop_labwc; exit 1; }
fi

python3 "$W/tools/wadbrowser-e2e-server.py" "$PORT" "$OUT/server.log" &
http_pid=$!
wadd_pid=
cleanup() {
  kill "$http_pid" 2>/dev/null
  wait "$http_pid" 2>/dev/null
  if [ -n "$wadd_pid" ]; then
    kill "$wadd_pid" 2>/dev/null
    wait "$wadd_pid" 2>/dev/null
  fi
  # What the runs' buses started (portals) and left: found by their HOME.
  for d in /proc/[0-9]*; do
    if tr '\0' '\n' <"$d/environ" 2>/dev/null | grep -q "^HOME=$OUT/"; then
      kill "${d#/proc/}" 2>/dev/null
    fi
  done 2>/dev/null
  stop_labwc
}
trap cleanup EXIT
sleep 0.5
S=http://127.0.0.1:$PORT

# run NAME HOME-NAME [ENV=V...] -- ARGS...: starts WadBrowser (its own bus,
# HOME under OUT/HOME-NAME); sets wb_pid and LOG (its test log, new each run).
run() {
  local name=$1 home=$2
  shift 2
  local envs=()
  while [ $# -gt 0 ] && [ "$1" != -- ]; do envs+=("$1"); shift; done
  shift
  local H=$OUT/$home
  mkdir -p "$H"/{home,config,data,cache,state}
  LOG=$OUT/$name.events
  : >"$LOG"
  env -u DISPLAY XDG_RUNTIME_DIR="$hl_rt" WAYLAND_DISPLAY="$hl_display" HOME="$H/home" \
    XDG_CONFIG_HOME="$H/config" XDG_DATA_HOME="$H/data" XDG_CACHE_HOME="$H/cache" XDG_STATE_HOME="$H/state" \
    GSK_RENDERER=cairo WEBKIT_SKIA_ENABLE_CPU_RENDERING=1 GDK_DISABLE=gl GTK_A11Y=none NO_AT_BRIDGE=1 \
    GDK_DEBUG=no-portals WADBROWSER_TEST_LOG="$LOG" WADBROWSER_FILTER_LISTS="$LISTS" WADBROWSER_SIGNINS="$OUT/no-signins.json" \
    "${envs[@]}" dbus-run-session -- "$BIN" --no-restore "$@" >"$OUT/$name.log" 2>&1 &
  wb_pid=$!
}

# wait_for PATTERN [SECONDS]: a line of this run's test log matching it (grep -E).
wait_for() {
  local t=$(( ${2:-20} * 10 ))
  for _ in $(seq "$t"); do
    grep -Eq "$1" "$LOG" && return 0
    kill -0 "$wb_pid" 2>/dev/null || break
    sleep 0.1
  done
  grep -Eq "$1" "$LOG"
}

# quit: Alt+F4 (the window closes, the browser with it), else its PIDs.
quit() {
  in_labwc wtype -M alt -k F4 -m alt 2>/dev/null
  for _ in $(seq 50); do kill -0 "$wb_pid" 2>/dev/null || break; sleep 0.2; done
  if kill -0 "$wb_pid" 2>/dev/null; then
    local child
    child=$(pgrep -P "$wb_pid" -x wadbrowser)
    [ -n "$child" ] && kill "$child"
    sleep 1
    kill "$wb_pid" 2>/dev/null
  fi
  wait "$wb_pid" 2>/dev/null
}

if want cookies; then
# ---------------------------------------------------------------- cookies + filter
run cookies-1 profile -- "$S/set?name=keep&value=yes"
if wait_for $'^title\tdefault\tset keep' "$T"; then pass "cookies: run 1 set a cookie"; else fail "cookies: run 1 didn't load"; fi
if wait_for '^filter-compiled' "$T"; then
  pass "filter: compiled on the first start ($(grep '^filter-compiled' "$LOG" | cut -f3) ms)"
else
  fail "filter: not compiled on the first start"
fi
quit

run cookies-2 profile -- "$S/show"
if wait_for $'^title\tdefault\tcookies: .*keep=yes' "$T"; then
  pass "cookies: the cookie survived a restart"
else
  fail "cookies: no cookie after a restart ($(grep $'^title' "$LOG" | tail -1))"
fi
if wait_for '^filter-loaded' 10 && ! grep -q '^filter-compiled' "$LOG"; then
  pass "filter: the second start only loaded it ($(grep '^filter-loaded' "$LOG" | cut -f3) ms)"
else
  fail "filter: the second start didn't load the cached filter"
fi
quit

run private-1 profile -- --private "$S/set?name=private&value=yes"
if wait_for $'^title\tprivate-[0-9]+\tset private' "$T"; then pass "cookies: a private window set one"; else fail "cookies: the private window didn't load"; fi
quit
run private-2 profile -- "$S/show"
if wait_for $'^title\tdefault\tcookies:' "$T"; then
  if grep -E $'^title\tdefault\tcookies:' "$LOG" | grep -q private=; then
    fail "cookies: a private window's cookie was kept"
  else
    pass "cookies: a private window's cookie wasn't kept"
  fi
else
  fail "cookies: /show didn't load"
fi
quit

fi

if want filter; then
# ---------------------------------------------------------------- the filter's blocking
run filter profile -- "$S/filter"
if wait_for $'^title\tdefault\tfilter blocked=' "$T"; then
  t=$(grep $'^title\tdefault\tfilter blocked=' "$LOG" | tail -1 | cut -f3)
  case "$t" in
    "filter blocked=1 allowed=1 hidden=1") pass "filter: $t" ;;
    *) fail "filter: $t" ;;
  esac
else
  fail "filter: the page didn't report"
fi
if grep -q '^/wb-ok.png' "$OUT/server.log" && ! grep -q '^/wb-ad-banner.png' "$OUT/server.log"; then
  pass "filter: the server saw the allowed image and not the blocked one"
else
  fail "filter: the server's log: $(grep -E '^/wb-' "$OUT/server.log" | tr '\n' ' ')"
fi
if wait_for $'^blocked\tdefault\t.*wb-ad-banner.png\t1$' 5; then
  pass "filter: the shield counted it, once"
else
  fail "filter: the shield didn't count it"
fi
quit

fi

if want popup; then
# ---------------------------------------------------------------- the popup
run popup popup -- "$S/opener"
if wait_for $'^title\tdefault\tpopup has opener: true' "$T"; then pass "popup: it has window.opener"; else fail "popup: no window.opener"; fi
if wait_for $'^title\tdefault\topener got: hello-from-popup' "$T"; then pass "popup: the opener got its message"; else fail "popup: the opener got nothing"; fi
if wait_for $'^tab-closed\tdefault' 10 && wait_for $'^tabs\t1$' 5 && [ "$(grep $'^tabs\t' "$LOG" | tail -1)" = $'tabs\t1' ]; then
  pass "popup: its tab is gone (tabs: $(grep $'^tabs\t' "$LOG" | cut -f2 | tr '\n' ' '))"
else
  fail "popup: its tab stayed ($(grep -E $'^(tabs|tab-|page-closed)' "$LOG" | tr '\n' ' '))"
fi
quit

fi

if want key; then
# ---------------------------------------------------------------- a key for a site
KEY=$OUT/key-k1
mkdir -p "$KEY"
L=http://localhost:$PORT
# Run 1: the browser's own windows sign in with k1 (@browser): its cookie goes into the key's folder.
printf '{"version": 2, "signins": [{"id": "k1", "name": "Key one", "web": true, "apps": ["@browser"], "dir": "%s", "paths": [], "sites": []}]}\n' "$KEY" >"$OUT/signins-browser.json"
run key-1 keyhome WADBROWSER_SIGNINS="$OUT/signins-browser.json" -- "$L/set?name=keycookie&value=k1"
if wait_for $'^title\tsignin-k1\tset keycookie' "$T"; then pass "key: the key's profile set its cookie"; else fail "key: the key's profile didn't load"; fi
quit
[ -f "$KEY/cookies.sqlite" ] && pass "key: its cookies are in the key's folder" || fail "key: no cookies.sqlite in the key's folder"
# Run 2: k1 signs in to localhost; a default tab goes there by a page's own navigation.
printf '{"version": 2, "signins": [{"id": "k1", "name": "Key one", "web": true, "apps": [], "dir": "%s", "paths": [], "sites": ["localhost"]}]}\n' "$KEY" >"$OUT/signins-site.json"
run key-2 keyhome WADBROWSER_SIGNINS="$OUT/signins-site.json" -- "$S/go?to=$L/show"
if wait_for $'^moved\tdefault\tsignin-k1\t' "$T"; then pass "key: the tab moved into the key's profile"; else fail "key: the tab didn't move"; fi
if wait_for $'^title\tsignin-k1\tcookies: .*keycookie=k1' "$T"; then
  pass "key: the key's cookie is there"
else
  fail "key: no key cookie ($(grep $'^title' "$LOG" | tail -2 | tr '\n' ' '))"
fi
quit
# Run 3: no keys: the default profile at the same page hasn't that cookie.
run key-3 keyhome -- "$L/show"
if wait_for $'^title\tdefault\tcookies:' "$T"; then
  if grep -E $'^title\tdefault\tcookies:' "$LOG" | grep -q keycookie; then
    fail "key: the default profile has the key's cookie"
  else
    pass "key: only the key's session has its cookie"
  fi
else
  fail "key: /show didn't load without keys"
fi
quit
# On the machine: no signins.json, and wadd (a fake one) has k1 for
# localhost, its folder the same one (`HOOK(keys-on-machine)`).
# (Its socket in the session's runtime dir: OUT may be too long a path for one.)
WADD=$hl_rt/e2e-wadd.sock
printf '[{"id": "k1", "name": "Key one", "sites": ["localhost"], "usedBy": [], "dir": "%s"}]\n' "$KEY" >"$OUT/wadd-keys.json"
python3 -I "$W/tools/wadbrowser-e2e-wadd.py" "$WADD" "$OUT/wadd-keys.json" "$OUT/wadd.log" &
wadd_pid=$!
for _ in $(seq 50); do [ -S "$WADD" ] && break; sleep 0.1; done
# Run 4: a default tab goes to the key's site by a page's own navigation.
run key-4 keyhome WADD_SOCKET="$WADD" -- "$S/go?to=$L/show"
if wait_for $'^moved\tdefault\tsignin-k1\t' "$T"; then pass "key (machine): the tab moved into wadd's key"; else fail "key (machine): the tab didn't move"; fi
if wait_for $'^title\tsignin-k1\tcookies: .*keycookie=k1' "$T"; then
  pass "key (machine): the key's folder's cookie is there"
else
  fail "key (machine): no key cookie ($(grep $'^title' "$LOG" | tail -2 | tr '\n' ' '))"
fi
quit
# Run 5: the key's site is the first page, maybe before wadd's keys come: it moves once they do.
run key-5 keyhome WADD_SOCKET="$WADD" -- "$L/show"
if wait_for $'^title\tsignin-k1\tcookies: .*keycookie=k1' "$T"; then
  pass "key (machine): a first page on the key's site is in the key's profile"
else
  fail "key (machine): the first page isn't in the key's profile ($(grep -E $'^(title|moved)' "$LOG" | tail -3 | tr '\n' ' '))"
fi
quit
grep -q '^GET /v1/keys/web' "$OUT/wadd.log" && pass "key (machine): the keys came from wadd" || fail "key (machine): wadd wasn't asked for its keys"
kill "$wadd_pid" 2>/dev/null
wait "$wadd_pid" 2>/dev/null
wadd_pid=
rm -f "$WADD"

fi

if want headers; then
# ---------------------------------------------------------------- headers
run headers profile -- "$S/headers"
if wait_for $'^title\tdefault\theaders ' "$T"; then
  t=$(grep $'^title\tdefault\theaders ' "$LOG" | tail -1 | cut -f3)
  [ "$t" = "headers dnt=1 gpc=1" ] && pass "headers: the page's own request: $t" || fail "headers: $t"
else
  fail "headers: the page didn't load"
fi
quit
# What a page loads itself, and its own navigations: WebKitGTK gives the UI
# process no way to set their headers (its content filters' modify-headers
# rules compile but aren't applied outside web extensions), so they have
# the JS ones only. Shown, not failed.
echo "NOTE headers: a page's own image: $(grep '^/wb-ok.png' "$OUT/server.log" | head -1)"
fi

if want download; then
# ---------------------------------------------------------------- downloads, error pages
run download profile -- "$S/download"
if wait_for $'^download\t.*Finished' "$T"; then
  f=$(find "$OUT/profile" -name 'wb*test*#1.txt' | head -1)
  [ -n "$f" ] && pass "download: saved under WebKit's suggested name (${f#"$OUT"/})" || fail "download: no 'wb test #1.txt' (or WebKit's wb_test_#1.txt)"
else
  fail "download: didn't finish ($(grep $'^download' "$LOG" | tr '\n' ' '))"
fi
if grep -q '^load-failed' "$LOG"; then fail "download: the load it interrupted got an error page"; else pass "download: no error page for the load it interrupted"; fi
quit
fi
if want error; then
run error profile -- "http://127.0.0.1:$((PORT + 1))/"
if wait_for $'^load-failed\tdefault\t'"http://127.0.0.1:$((PORT + 1))/" "$T"; then pass "error: a refused connection is a failure"; else fail "error: no failure for a refused connection"; fi
if wait_for $'^title\tdefault\t.*\t'"http://127.0.0.1:$((PORT + 1))/\$" 10; then
  pass "error: its page shows at the address that failed ($(grep $'^title' "$LOG" | tail -1 | cut -f3))"
else
  fail "error: no error page ($(grep $'^title' "$LOG" | tail -1))"
fi
quit

fi

if want pixels; then
# ---------------------------------------------------------------- pixels
# Inside one bus: a browser, a second launch (a tab in front), an --app window.
H=$OUT/pixels
mkdir -p "$H"/{home,config,data,cache,state}
LOG=$OUT/pixels.events
: >"$LOG"
cat >"$OUT/pixels-inner.sh" <<'INNER'
#!/usr/bin/env bash
set -u
seen() { for _ in $(seq $((T * 10))); do grep -q "$1" "$LOG" && return 0; sleep 0.1; done; return 1; }
"$BIN" --no-restore "$S/big?first" >"$OUT/pixels.log" 2>&1 &
wb=$!
seen $'\tBIG first' && sleep 2 && grim -o HEADLESS-1 "$OUT/pixels-1-first.png"
"$BIN" "$S/big?second" >>"$OUT/pixels.log" 2>&1
seen $'\tBIG second' && sleep 2 && grim -o HEADLESS-1 "$OUT/pixels-2-second-tab.png"
"$BIN" --app pixels --name Pixels --url "$S/big?app" >>"$OUT/pixels.log" 2>&1
seen $'\tBIG app' && sleep 2 && grim -o HEADLESS-1 "$OUT/pixels-3-app.png"
kill "$wb" 2>/dev/null
for _ in $(seq 50); do kill -0 "$wb" 2>/dev/null || break; sleep 0.2; done
kill -9 "$wb" 2>/dev/null
wait "$wb" 2>/dev/null
INNER
env -u DISPLAY XDG_RUNTIME_DIR="$hl_rt" WAYLAND_DISPLAY="$hl_display" HOME="$H/home" \
  XDG_CONFIG_HOME="$H/config" XDG_DATA_HOME="$H/data" XDG_CACHE_HOME="$H/cache" XDG_STATE_HOME="$H/state" \
  GSK_RENDERER=cairo GTK_A11Y=none NO_AT_BRIDGE=1 GDK_DEBUG=no-portals \
  WADBROWSER_TEST_LOG="$LOG" WADBROWSER_FILTER_LISTS="$LISTS" WADBROWSER_SIGNINS="$OUT/no-signins.json" \
  BIN="$BIN" S="$S" LOG="$LOG" OUT="$OUT" T="$T" \
  timeout $((T * 4 + 60)) dbus-run-session -- bash "$OUT/pixels-inner.sh" >"$OUT/pixels-bus.log" 2>&1
for shot in pixels-1-first pixels-2-second-tab pixels-3-app; do
  if [ ! -f "$OUT/$shot.png" ]; then
    fail "pixels: no $shot (its page didn't load: $(grep -c . "$LOG") events)"
    continue
  fi
  # The page's area: black text on white, as a share of its pixels.
  read -r black white < <(python3 -I - "$OUT/$shot.png" <<'PY'
import struct, sys, zlib
d = open(sys.argv[1], "rb").read()
i, idat = 8, b""
while i < len(d):
    n = struct.unpack(">I", d[i:i + 4])[0]
    t, c = d[i + 4:i + 8], d[i + 8:i + 8 + n]
    i += 12 + n
    if t == b"IHDR":
        w, h, _, ct = struct.unpack(">IIBB", c[:10])
    elif t == b"IDAT":
        idat += c
raw, bpp = zlib.decompress(idat), {2: 3, 6: 4}[ct]
stride, prev, o, black, white = w * bpp, bytearray(w * bpp), 0, 0, 0
for y in range(h):
    f, line = raw[o], bytearray(raw[o + 1:o + 1 + stride])
    o += 1 + stride
    for x in range(stride):
        a = line[x - bpp] if x >= bpp else 0
        b = prev[x]
        c = prev[x - bpp] if x >= bpp else 0
        if f == 1: line[x] = (line[x] + a) & 255
        elif f == 2: line[x] = (line[x] + b) & 255
        elif f == 3: line[x] = (line[x] + (a + b) // 2) & 255
        elif f == 4:
            pa, pb, pc = abs(b - c), abs(a - c), abs(a + b - 2 * c)
            line[x] = (line[x] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
    prev = line
    if 110 <= y < h - 10:
        for x in range(60, w - 60):
            p = line[x * bpp:x * bpp + 3]
            if max(p) < 60: black += 1
            elif min(p) > 220: white += 1
print(black, white)
PY
)
  if [ "${black:-0}" -gt 5000 ] && [ "${white:-0}" -gt 100000 ]; then
    pass "pixels: $shot draws its page ($black black, $white white)"
  else
    fail "pixels: $shot is blank or scaled ($black black, $white white): $OUT/$shot.png"
  fi
done
fi

echo "--- $fails failed (logs in $OUT)"
exit "$fails"
