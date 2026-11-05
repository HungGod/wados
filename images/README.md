# Images

What wadspaces run on, all Fedora 44. `builder`, `base` and `stream` are
built here; a wadspace's image is a set: the base with its packages copied
on and assembled, then its own build steps.

| Image | What it is |
|---|---|
| `localhost/wados-builder:44` (`builder/`) | Fedora 44 with Rust 1.99. Builds `wadbrowser` (`apps/wadbrowser`), `wadspaces-clipboard` (`crates/wad-clip`), `wadspaces-session` (`crates/wad-session`) and `selkies-desktop` (`apps/selkies-desktop`) into `.build/images/bin`. |
| `localhost/wados-base:44` (`base/`) | The desktop: labwc, Xwayland, foot in tmux, selkies-desktop's icons and panel, WadBrowser, dark only, drawn as one window on the compositor at `/run/wadspaces-display`. Also the build helpers: `wadspaces-feature`, `wadspaces-dnf`, `wadspaces-vscode-extensions`, `wadspaces-pkg` and `wadspaces-webapp`. Built on linuxserver's `baseimage-fedora:44`, pinned by digest (s6-overlay v3, user `abc`, home `/config`). |
| `localhost/wados-stream:44` (`stream/`) | The stream sidecar, beside a wadspace while it's streamed: upstream Selkies 2.2.1 (its wheel, which brings its web client, and pixelflux 2.2.3 and pcmflux 2.2.1) in `/opt/selkies`, every wheel pinned by version and sha256 (`stream/requirements.txt`, written by `stream/lock.sh`), and PulseAudio. No desktop of its own: the wadspace's draws on Selkies' compositor. On linuxserver's `baseimage-fedora:44`, the base's digest. See [The stream sidecar](#the-stream-sidecar). |
| `localhost/wados-pkg-<id>` | A package: what one install added to the base, `FROM scratch` (`wadspaces-pkg capture`). It holds the install's files, plus fragments under `/usr/share/wadspaces/pkg/<id>/`: the RPM headers it added (`rpmdb`, listed in `rpms`), the lines it added to passwd, group, shadow and shells, and its alternatives. CI publishes packages as `ghcr.io/hunggod/wados-pkg-<id>:<abi>` (`.github/workflows/packages.yml`, planned by `wados-pkgs`). |
| `localhost/wados-chain:<sha12>` | The base with packages copied on, one per image, named by the base and the packages so far. |
| `localhost/wados-set-<key>:latest` | The top chain after `wadspaces-pkg assemble`, which imports every package's RPM headers into rpm's database, merges the text fragments and runs the cache triggers. So `rpm -qa`, `rpm -V` and `dnf install` work on top of it. |

```bash
images/build.sh                      # the builder, the base, then the stream sidecar
images/build.sh --only stream        # just the sidecar
images/build.sh pkg nodejs           # a feature's package (wadspaces-feature nodejs), captured on the base
images/build.sh pkg rpm-krita        # an RPM app's (wadspaces-dnf krita --desktop org.kde.krita.desktop krita)
images/build.sh set rpm-krita nodejs # a set
images/check.sh                      # all of it, checked, and the set's desktop on a headless labwc
images/stream/try-stream.sh          # a wadspace streamed on this laptop, to a browser (see below)
images/stream/lock.sh                # pin Selkies again (after changing its version there)
images/base/try-projects.sh          # the base's wadspaces-projects on this laptop: names, copies, launchers
```

The features still to port are listed in `base/TODO-features.md`.

WadBrowser (`apps/wadbrowser`, a Cactus fork) is in the base at
`/usr/local/bin/wadbrowser`, with its launcher `io.wados.Browser.desktop`
(wad-gen's recipe names it). It's what links open in: `/etc/xdg/mimeapps.list`
(http, https, HTML) and `BROWSER` (`wadbrowser-link`) run `wadbrowser
--default`, full or Focus as the wadspace's `wadbrowser.conf` says. Each web
app in `webapps.json` gets a launcher at start (`wadspaces-webapp --id <id>
--icon <png> <name> <url>`: `wadspaces-webapp-<id>.desktop`, running
`wadbrowser --app <id> --name <name> --url <url> --icon <png>`).

## What wadd gives a wadspace

| What | Where |
|---|---|
| The display | A folder mounted read-only at `/run/wadspaces-display`, with one socket in it (`wayland-*`; `WADSPACES_WAYLAND` names it if there are several). The machine's labwc serves it through `wp_security_context_v1`, with app id `io.wadspaces.<id>`. abc must be able to connect: the socket belongs to abc's uid outside. |
| Users | Run rootless with `PUID=1000 PGID=1000` and the maps `0:1:1000`, `1000:0:1`, `1001:1001:64535` (uid and gid), so abc is you outside. |
| The display it's on | `/run/wadspaces-extra/display`: `screen` (or no file) means `/run/wadspaces-display`, and `stream` means `/run/wadspaces-stream`, its stream sidecar's display volume. svc-de links `/run/wadspaces-current` to the one in use. `/run/wadspaces-extra/screens` gives the number of labwc outputs. |
| Start-time extras | `/run/wadspaces-extra/`: `layout.json`, `projects.json`, `signins.json`, `webapps.json` and `wadbrowser.conf`. |
| Projects, sign-ins | `/run/wadspaces-copies/<id>/` (wadd's copies of the projects) and `/run/wadspaces-signins/<id>/` (saved sign-ins). |
| Secrets, at run time only | `/run/secrets/github_token`, a podman secret for git and `gh`. |
| Sound, GPU | `PULSE_SERVER` set to the machine's PipeWire socket (`/run/wadspaces-current/pulse/native` when it can be streamed: the sidecar's PulseAudio while it is). `/dev/dri`; without it, labwc renders with pixman. |
| Look | `WADSPACES_TITLEBARS=1` gives windows title bars. |

The base's labels: `io.wados.base=1`, `io.wados.fedora=44`,
`io.wadspaces.abi=fedora44.1`, `io.wadspaces.projects=3`,
`io.wadspaces.extras=1` and `io.wadspaces.display=2`. Packages and sets keep
v1's label keys: `io.wadspaces.pkg`, `.pkg.needs`, `.pkg.version`, `.pkg.abi`,
`.pkg.content`, `.chain`, `.from`, `.layer`, `.pkgs` and `.assembly`.

## The stream sidecar

`stream/` is what a streamed wadspace draws on (wad-gen's `units` and
`moving_units`, `crates/wad-gen/src/units.rs`). The two share a volume
(`wadstream-<id>`): the sidecar's runtime dir, at `/run/wadspaces-display`
in it and `/run/wadspaces-stream` in the wadspace. Selkies' own Wayland
compositor (pixelflux's) puts its socket there (`wayland-*`, the first free)
and the wadspace's labwc draws on it as one window, as it does on the
machine's labwc, so `wadspaces-session` can move a running desktop between
the two. Its PulseAudio's socket is there too (`pulse/native`): what the
wadspace plays goes to its `output` sink, whose monitor Selkies sends.

It serves one port (3000, plain HTTP; the unit publishes it on 127.0.0.1
alone) under the stream's path only (`SELKIES_SUBFOLDER=/ws-<guid>`; the
page is `/ws-<guid>/`, with the slash; anything else is 404), in Selkies'
secure mode, and it won't start without that (`init-stream-display`):

| What | Where |
|---|---|
| The master token | `SELKIES_MASTER_TOKEN`, the podman secret `wad_stream_master_<id>`: Selkies' control-plane key (`POST /api/tokens`, `/api/switch`), never a viewer's. |
| Who may watch | `SELKIES_TOKENS_FILE`, `/run/wadspaces-selkies/tokens.json`: `{"<key>": {"role": "controller", "slot": null}}`. The stream's key opens it as `/ws-<guid>/#token=<key>` (or the `selkies_token` cookie). Taken out, a key is cut off at once. |
| ICE servers (public) | `SELKIES_RTC_CONFIG_JSON`, `/run/wadspaces-selkies/rtc.json`: `{"iceServers": [...]}`, Cloudflare's STUN and the TURN credentials wadd mints. Never Cloudflare's key: `SELKIES_ENABLE_CLOUDFLARE_TURN` is off in the image. |
| The transport | Private: `SELKIES_MODE=websockets` (the page decodes with WebCodecs: a secure context). Public: `webrtc` with dual mode and one mux port, UDP and TCP (`SELKIES_WEBRTC_{UDP,TCP}_MUX_PORT`). |
| Health | `GET /ws-<guid>/api/health` and `/api/status`, no token. |

`/run/wadspaces-selkies` is wadd's folder for the stream
(`<state>/streams/<id>/`), mounted read-only. Selkies refuses a file another
user owns or can write, so both files are the machine's user's (uid 1000,
abc in the sidecar under the units' maps; root's would be nobody's there),
mode 0600, in a folder of mode 0700, each written as a new file in that
folder and renamed over the old.

The image turns off what would land in the sidecar rather than the
wadspace: file transfers, printing, the webcam, the apps catalog, commands,
Selkies' own sharing links and (until phase 10's Play together) gamepads.
It runs the GPU it's given (`/dev/dri`) for its compositor and VA-API
encoding, else software (x264).

`stream/try-stream.sh` proves it on this laptop with no wadd: wad-gen's
units for a streamed wadspace, run through quadlet's dry run, rootless; a
browser (`stream/stream-probe.mjs`, Playwright from the spike) streams the
desktop with the key, and nothing with no token or a wrong one; Selkies
answers under `/ws-test/` alone; WebRTC with dual mode streams too; and a
wadspace on a (headless) screen moves to its stream and back.
