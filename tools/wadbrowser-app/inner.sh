#!/usr/bin/env bash
# tools/wadbrowser-app.sh's scenario, inside its D-Bus session and labwc. Prints PASS or
# FAIL per check; a summary last.
set -u
U=http://127.0.0.1:$PORT
LOG=$RUN/http.log
SHOTS=$RUN/shots
fails=0
# check DESCRIPTION COMMAND [ARGS...]
check() {
  local what=$1
  shift
  if "$@"; then echo "PASS  $what"; else echo "FAIL  $what"; fails=$((fails + 1)); fi
}
# soon SECONDS COMMAND [ARGS...]: COMMAND succeeds within SECONDS (polled every 0.2 s).
soon() {
  local n=$(($1 * 5))
  shift
  for _ in $(seq "$n"); do "$@" && return 0; sleep 0.2; done
  return 1
}
not() { ! "$@"; }
is() { [ "$1" = "$2" ]; }
tops() { "$TOPS" 2>/dev/null; }
# win APP_ID TITLE-PREFIX: such a window is open.
win() { tops | grep -F "app_id=\"$1\"" | grep -qF "title=\"$2"; }
titled() { tops | grep -qF "title=\"$1\""; }
nwin() { tops | grep -cF "app_id=\"$1\""; }
ntitled() { tops | grep -cF "title=\"$1\""; }
served() { grep -qF "GET $1" "$LOG" 2>/dev/null; }
nserved() { grep -cF "GET $1" "$LOG" 2>/dev/null; }
logged() { grep -qE "$1" "$LOG" 2>/dev/null; }
mine() { pgrep -P $$ -x wadbrowser | sort -n | tr '\n' ' ' | sed 's/ $//'; }
nmine() { pgrep -P $$ -x wadbrowser | wc -l; }
shot() { grim -o HEADLESS-1 "$SHOTS/$1.png" && echo "      shot $SHOTS/$1.png"; }
wb() { "$WB" "$@"; }

# ------------------------------------------------------------ one browser
"$WB" "$U/a.html" >"$RUN/primary.log" 2>&1 &
P=$!
check "the first launch opens its page" soon 40 served /a.html
check "its window has WadBrowser's app_id" soon 15 win io.wados.Browser "Page A"
shot 01-first

t0=$(date +%s%N)
wb "$U/b.html"
code=$?
t1=$(date +%s%N)
echo "      the second launch took $(((t1 - t0) / 1000000)) ms"
check "the second launch exits 0" is "$code" 0
check "its page opens in the running browser" soon 15 served /b.html
check "there's still one browser process (pid $P)" is "$(mine)" "$P"
check "still one window" is "$(nwin io.wados.Browser)" 1
check "the new tab is in front (its title is the window's)" soon 15 win io.wados.Browser "Page B – WadBrowser"
shot 02-second-tab

wb --new-window "$U/c.html"
check "--new-window: exits 0" is $? 0
two() { is "$(nwin io.wados.Browser)" 2; }
check "--new-window: a second window" soon 15 two

# --------------------------------------------------------------- web apps
wb --app mock --name "Mock App" --url "$U/app/?wander=1" --icon "$RUN/icon.png"
check "--app: exits 0" is $? 0
check "--app: its own app_id and name" soon 15 win wadspaces-webapp-mock "Mock App"
sleep 0.8
shot 03-app
check "--app: it goes to another site's page" soon 15 served /elsewhere.html
sleep 1.5
shot 04-offsite
wb --app mock --name "Mock App" --url "$U/app/"
check "--app again: exits 0" is $? 0
sleep 1
check "--app again: its window is reused" is "$(nwin wadspaces-webapp-mock)" 1
check "still one browser process" is "$(mine)" "$P"

# ------------------------------------------------------------------ Focus
wb --no-urlbar "$U/a.html?focus"
check "--no-urlbar: exits 0" is $? 0
check "--no-urlbar: its page" soon 15 served "/a.html?focus"
sleep 1.5
shot 05-focus

# --default: what wadbrowser.conf says (the user's own here: focus). The
# Focus window (showing Page A) takes the link; a Full one would leave it.
mkdir -p "$RUN/config/wadbrowser"
printf 'default = focus\n' >"$RUN/config/wadbrowser/wadbrowser.conf"
check "the Focus window shows Page A" titled "Page A – WadBrowser"
wb --default "$U/b.html?default"
check "--default: exits 0" is $? 0
check "--default (focus): its page" soon 15 served "/b.html?default"
check "--default (focus): in the Focus window" soon 15 not titled "Page A – WadBrowser"
rm "$RUN/config/wadbrowser/wadbrowser.conf"

# --------------------------------------------------------------- sign-ins
wb --signin github --url "$U/login/device?form=split" --code WDJB-MJHT
check "--signin github: exits 0" is $? 0
check "--signin github: its window" soon 15 titled "Sign in to GitHub"
check "the sign-in page came first" soon 15 served "/login?return_to"
check "split form: the code typed in, with input and change events" \
  soon 20 logged "GET /report\?form=split&value=WDJBMJHT&inputs=[1-9][0-9]*&changes=[1-9]"
shot 06-signin-split
wb --signin github --url "$U/login/device?form=single" --code ABCD-1234
check "--signin again: exits 0" is $? 0
check "--signin again: reused, with the new code" \
  soon 20 logged "GET /report\?form=single&value=ABCD-1234&inputs=[1-9][0-9]*&changes=[1-9]"
check "--signin again: still one sign-in window" is "$(ntitled "Sign in to GitHub")" 1
shot 07-signin-single
check "it never submitted" not logged /submitted
kept() { grep -q form=single "$RUN/data/wadbrowser/signins/github.json" 2>/dev/null; }
check "--signin keeps where it is" soon 10 kept
wb --signin github --show
check "--show (open): exits 0" is $? 0
wb --signin github --done
check "--done: exits 0" is $? 0
check "--done: its window is gone" soon 10 not titled "Sign in to GitHub"
check "--done: forgotten" not test -e "$RUN/data/wadbrowser/signins/github.json"
out=$(wb --signin github --show 2>&1)
code=$?
echo "      --show said: $out"
check "--show after --done: exits 1" is "$code" 1
check "--show after --done: says why" grep -q "no GitHub sign-in" <<<"$out"

wb --signin tailscale --url "$U/c.html?tailscale"
check "--signin tailscale: its window" soon 15 titled "Sign in to Tailscale"
check "--signin tailscale: it loads" soon 15 served "/c.html?tailscale"
sleep 1
# Closed by hand (Alt+F4 closes the focused window in this labwc).
wtype -M alt -k F4 -m alt
check "closed by hand" soon 10 not titled "Sign in to Tailscale"
before=$(nserved "/c.html?tailscale")
wb --signin tailscale --show
check "--show (closed): exits 0" is $? 0
check "--show (closed): open again" soon 15 titled "Sign in to Tailscale"
again() { [ "$(nserved "/c.html?tailscale")" -gt "$before" ]; }
check "--show (closed): where it was" soon 15 again
shot 08-tailscale-again
wb --signin tailscale --done
check "--done (tailscale): gone" soon 10 not titled "Sign in to Tailscale"

# --------------------------------------------------- the rest of the line
v=$(wb --version)
check "--version, with the browser running: $v" grep -q '^wadbrowser ' <<<"$v"
wb --app "Not An Id" >/dev/null 2>&1
check "a bad --app is refused (exit 2) before it reaches the browser" is $? 2
wb --ozone-platform=wayland --no-sandbox "$U/b.html?flags" 2>/dev/null
check "other browsers' flags are ignored: exits 0" is $? 0
check "other browsers' flags are ignored: the link still opens" soon 15 served "/b.html?flags"

"$WB" --instance second "$U/a.html?instance" >"$RUN/instance.log" 2>&1 &
I=$!
check "--instance: a browser of its own (pid $I)" soon 40 served "/a.html?instance"
check "--instance: two browser processes now" is "$(nmine)" 2
check "--instance: its own data" test -d "$RUN/data/wadbrowser/instances/second"
wb --instance second "$U/b.html?instance"
check "--instance: a later launch exits 0" is $? 0
check "--instance: and reaches it" soon 15 served "/b.html?instance"
check "--instance: still two processes" is "$(nmine)" 2

"$WB" --instance eph --ephemeral "$U/c.html?ephemeral" >"$RUN/ephemeral.log" 2>&1 &
E=$!
check "--ephemeral: its page" soon 40 served "/c.html?ephemeral"
check "--ephemeral: it's a run that keeps nothing" grep -q "ephemeral=true" "$RUN/ephemeral.log"
kill "$E" 2>/dev/null
wait "$E" 2>/dev/null

echo "--- windows"
tops
echo "--- the browser's processes: $(mine)"

kill "$I" "$P" 2>/dev/null
wait "$I" 2>/dev/null
wait "$P" 2>/dev/null
check "nothing of ours left running" is "$(mine)" ""
echo "--- $fails failed"
