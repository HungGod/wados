# WadOS

The software that runs a WadSpaces machine: a Fedora 44 bootc computer that boots into desktops you design. Each wadspace is a container with its own apps, files and sign-ins, and you run it full screen, in a window, or streamed to another device. v2 rebuilds the `wadspaces-v2-spike` design for real: the launcher and the desktop, the Builder, streams and the phone remote.

**Status: phase 4.** wadd builds and runs wadspaces. The shell runs on it: the launcher, the desktop, the bar and its tray, Super+Tab, Alt+Space, toasts with Undo, the reveal bar, the other displays' bars, and the Builder. The host image (Fedora 44 bootc, our labwc, greetd into the shell, wadd as root) boots in a VM to the lock screen, unlocks, and runs a wadspace with sound (`tools/vm.sh check`). Next is WadBrowser (phase 5).

## Layout

| Path | What it is |
|---|---|
| `crates/wad-model` | The v6 model: wadspaces, the catalog, wallpapers, and the pure logic that edits them (the spike's `src/model` and `src/sim/ops`), applied by wadd for every surface. |
| `crates/wad-gen` | What wadd generates from a wadspace: its packages (Fedora recipes, needs, CI's waves), its set (key, state, Containerfile), its extras folder and its quadlet units. Its bin `wados-pkgs` gives `images/build.sh` and CI the same answers. |
| `crates/wad-proto` | wadd's API as types: errors, events, edits and their results, runs, builds, jobs, the screen. |
| `crates/wad-wadd-client` | wadd from a program on the machine: typed calls over its socket, and its event stream, followed and reconnected. |
| `crates/wad-config` | wadd's configuration: the image's file, then the machine's (`/etc/wados`), merged key by key. No secrets. |
| `crates/wad-podman`, `wad-systemd` | Podman's libpod REST API and systemd over D-Bus (v1's). |
| `crates/wad-clip`, `wad-session` | The clipboard bridge and `wadspaces-session` (v1's), built into the base image. |
| `crates/wad-gtk` | WadSpaces' look in GTK4, dark only: the spike's tokens as CSS custom properties, Inter and Space Grotesk, lucide icons (`assets/lucide`), springs, and the session lock (gtk4-layer-shell's, through our FFI). |
| `crates/wad-wayland` | A Wayland socket the compositor serves to one sandboxed wadspace (`wp_security_context_v1`), plus probes (`examples/`). |
| `apps/wadd` | The machine's daemon. It keeps the library (edits, Undo, events), builds wadspaces (packages, chains, sets, jobs) and runs them on the screen through their own sockets. It serves `/v1` on `/run/wadd/wadd.sock`; `dev/try.sh` shows all of it. |
| `apps/wados-shell` | The shell on labwc. The launcher and the desktop are a scroll apart. The bar, the overlays and the lock screen are layer-shell surfaces, and the Builder is a window (`src/builder/`: its board of stacks and its sidebar). Drags between them go through `src/dnd.rs`. It handles window management with foreign-toplevel and answers `wadsctl`. |
| `apps/wadsctl` | Tells the shell what to do: what labwc's keybinds run, and what tests drive it with. |
| `apps/selkies-desktop` | The panel and Start menu inside each wadspace (C, v1's). |
| `images/` | The wadspace images on Fedora 44: the base, the builder, `wadspaces-pkg` (packages captured and assembled on rpm), the feature scripts; `build.sh` (packages and sets through `wados-pkgs`) and `check.sh`. |
| `host/` | The machine's OS image (`docs/decisions/0003`): Fedora 44 bootc, our labwc (`labwc/`: three patches, built as an RPM), greetd into the shell, wadd as root, PipeWire, NetworkManager, BlueZ, Tailscale and cloudflared; `Containerfile.nvidia` for NVIDIA's driver. `host/build.sh image` builds it (`localhost/wados-host`); `tools/vm.sh` boots it in a VM and checks it. |
| `site/` | wadspaces.com, the landing page on Cloudflare Pages (`site/README.md` has the domain's layout). |
| `.github/workflows/` | CI, with only GitHub's token. `ci.yml` runs fmt, clippy, the tests and the site's build. `images.yml` builds and checks the builder and the base, and pushes them from main as `ghcr.io/hunggod/wados-{builder,base}:44`. `packages.yml` then builds every package in waves (`package.yml`) and publishes them with the index machines read. `host.yml` builds the host image and lints it, without pushing it. |
| `assets/` | The spike's fonts, marks, app and web icons, the players' pictures, the art wallpapers (`scripts/art.py` makes them), and lucide's icons (`tools/lucide.mjs`). |
| `docs/decisions/` | What was found and decided: labwc (`0001`), building and running a wadspace (`0002`). |
| `tools/` | `headless.sh` (a headless labwc for probes and screenshots), `build-labwc.sh` (labwc with our patches), `probe-labwc.sh` (what we need from labwc, as checks; `drag-source.py` is its other app), `seed-library.py` (the spike's sample library for a test wadd), `lucide.mjs`. |

## Develop

On Fedora 44:

```bash
sudo dnf install gtk4-devel gtk4-layer-shell-devel libadwaita-devel webkitgtk6.0-devel pam-devel \
  labwc grim wlr-randr wtype foot wayland-utils
cargo test --workspace
tools/build-labwc.sh                  # labwc with host/labwc/*.patch, into .build/labwc
```

Run the shell in a labwc window on your desktop (labwc runs nested inside KDE or GNOME):

```bash
cargo build -p wados-shell -p wadsctl
PATH=$PWD/target/debug:$PATH .build/labwc/labwc -C host/etc/xdg/labwc -s 'wados-shell --builder'
```

That shell is the real thing: it uses your home (its desktop is your `~/Desktop`, its layout `~/.config/wados/desk.json`). To try it safely, with screenshots, use `try.sh`. It gives the shell a wadd of its own seeded with the spike's sample library, a home of its own with the spike's sample Desktop, and a session bus of its own with no desktop portals, so a file dialog or an app it opens stays in its headless session:

```bash
apps/wados-shell/dev/try.sh                       # .build/shell/{launcher,desktop}.png
TRY_OUTPUTS=2 TRY_FULLSCREEN=1 TRY_KEEP=1 apps/wados-shell/dev/try.sh   # a second display, a wadspace full screen, left running
```

Or headless by hand:

```bash
. tools/headless.sh
PATH=$PWD/target/debug:$PATH HL_CONFIG=host/etc/xdg/labwc start_labwc 1
in_labwc env GSK_RENDERER=cairo WADOS_NO_LOCK=1 target/debug/wados-shell &
in_labwc target/debug/wadsctl page desktop
shot HEADLESS-1 desktop.png
stop_labwc
```

`WADOS_NO_LOCK=1` starts the shell unlocked. Without it, the shell locks first, as the machine does. `WADOS_PAM_SERVICE` names the PAM service it checks passwords with (`wados-lock` on the machine; `login` on a laptop).

## Secrets

None are in this repo, in a build's context, or in an image's layers. Only the installer stick will carry the Cloudflare tunnel and TURN credentials: `host/build.sh installer` (phase 9) reads them from `${WADOS_SECRETS:-~/.config/wados/secrets}`. Everything builds and runs without them: public streams are simply off.
