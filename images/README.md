# Images

What wadspaces run on, all Fedora 44. `builder` and `base` are built here; a
wadspace's image is a set: the base with its packages copied on and
assembled, then its own build steps.

| Image | What it is |
|---|---|
| `localhost/wados-builder:44` (`builder/`) | Fedora 44 with Rust 1.99. Builds `wadbrowser` (`apps/wadbrowser`), `wadspaces-clipboard` (`crates/wad-clip`), `wadspaces-session` (`crates/wad-session`) and `selkies-desktop` (`apps/selkies-desktop`) into `.build/images/bin`. |
| `localhost/wados-base:44` (`base/`) | The desktop: labwc, Xwayland, foot in tmux, selkies-desktop's icons and panel, WadBrowser, dark only, drawn as one window on the compositor at `/run/wadspaces-display`. Also the build helpers: `wadspaces-feature`, `wadspaces-dnf`, `wadspaces-vscode-extensions`, `wadspaces-pkg` and `wadspaces-webapp`. Built on linuxserver's `baseimage-fedora:44`, pinned by digest (s6-overlay v3, user `abc`, home `/config`). |
| `localhost/wados-pkg-<id>` | A package: what one install added to the base, `FROM scratch` (`wadspaces-pkg capture`). It holds the install's files, plus fragments under `/usr/share/wadspaces/pkg/<id>/`: the RPM headers it added (`rpmdb`, listed in `rpms`), the lines it added to passwd, group, shadow and shells, and its alternatives. CI publishes packages as `ghcr.io/hunggod/wados-pkg-<id>:<abi>` (`.github/workflows/packages.yml`, planned by `wados-pkgs`). |
| `localhost/wados-chain:<sha12>` | The base with packages copied on, one per image, named by the base and the packages so far. |
| `localhost/wados-set-<key>:latest` | The top chain after `wadspaces-pkg assemble`, which imports every package's RPM headers into rpm's database, merges the text fragments and runs the cache triggers. So `rpm -qa`, `rpm -V` and `dnf install` work on top of it. |

```bash
images/build.sh                      # the builder, then the base
images/build.sh pkg nodejs           # a feature's package (wadspaces-feature nodejs), captured on the base
images/build.sh pkg rpm-krita        # an RPM app's (wadspaces-dnf krita --desktop org.kde.krita.desktop krita)
images/build.sh set rpm-krita nodejs # a set
images/check.sh                      # all of it, checked, and the set's desktop on a headless labwc
```

The features still to port are listed in `base/TODO-features.md`. The
stream sidecar comes in a later phase.

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
| The display it's on | `/run/wadspaces-extra/display`: `screen` (or no file) means `/run/wadspaces-display`, and `stream` means `/run/wadspaces-stream` (a later phase). `/run/wadspaces-extra/screens` gives the number of labwc outputs. |
| Start-time extras | `/run/wadspaces-extra/`: `layout.json`, `projects.json`, `signins.json`, `webapps.json` and `wadbrowser.conf`. |
| Projects, sign-ins | `/run/wadspaces-copies/<id>/` (wadd's copies of the projects) and `/run/wadspaces-signins/<id>/` (saved sign-ins). |
| Secrets, at run time only | `/run/secrets/github_token`, a podman secret for git and `gh`. |
| Sound, GPU | `PULSE_SERVER` set to the machine's PipeWire socket. `/dev/dri`; without it, labwc renders with pixman. |
| Look | `WADSPACES_TITLEBARS=1` gives windows title bars. |

The base's labels: `io.wados.base=1`, `io.wados.fedora=44`,
`io.wadspaces.abi=fedora44.1`, `io.wadspaces.projects=3`,
`io.wadspaces.extras=1` and `io.wadspaces.display=2`. Packages and sets keep
v1's label keys: `io.wadspaces.pkg`, `.pkg.needs`, `.pkg.version`, `.pkg.abi`,
`.pkg.content`, `.chain`, `.from`, `.layer`, `.pkgs` and `.assembly`.
