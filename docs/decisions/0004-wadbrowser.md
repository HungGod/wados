# 0004: WadBrowser

Phase 5. WadBrowser is our fork of [Cactus](https://github.com/yohannes-mesay/cactus-browser) (commit 8ae5a66,
MPL-2.0; its LICENSE is in each of the five folders): GTK 4, libadwaita and WebKitGTK 6, in Rust. It replaces
v1's Tauri WadBrowser on the machine and in every wadspace, and draws the v2 spike's browser
(`wadspaces-v2-spike/src/apps/browser/`).

## Where it is

| Crate | What |
|---|---|
| `crates/wadbrowser-core` | The pure parts: the command line (`cli.rs`), `wadbrowser.conf` (`conf.rs`), instances, sites and modes (`site.rs`, `modes.rs`), sign-in windows (`signin.rs`), profiles and keys (`profiles.rs`), per-site zoom, Cactus's store and prefs, a small wadd client (`wadd.rs`), and the `cactus://` pages (`internal.rs`, with `res/` for their CSS, script, fonts and site icons). |
| `crates/wadbrowser-privacy` | The ad blocker: EasyList and EasyPrivacy, turned into WebKit's content-filter rules (`content.rs`), the social blocker, the per-site allowlist, the counts. |
| `crates/wadbrowser-webkit` | WebKit: a session per profile, views (one UserContentManager each), the filters' store, the `cactus://` scheme, the shield's state, permissions, downloads, zoom, tab sleeping. |
| `crates/wadbrowser-ui` | The window (`window.rs`, `window/chrome.rs` the tab bar and header, `window/wire.rs` what the widgets do, `window/views.rs` the views' glue), the menus, shield, profile chip and dialogs, and `examples/gallery.rs` (each widget and page with sample data, for screenshots). |
| `apps/wadbrowser` | The program; `data/` has `io.wados.Browser.desktop` and its icon. |

Versions are the workspace's: gtk4 0.11, libadwaita 0.9 (`v1_9`), webkit6 0.6.1 (`v2_52`; Fedora 44 has WebKitGTK
2.54), adblock 0.13 (`content-blocking`).

## How it starts

```
wadbrowser [URL...]                        tabs and an address bar (Full)
wadbrowser --no-urlbar|--focus [URL...]    Focus: tabs, no address bar
wadbrowser --default [URL...]              what wadbrowser.conf says (the link handler)
wadbrowser --private [URL...]              a private window
wadbrowser --app <id> --name <name> --url <start> [--icon <png>] [URL...]
                                           a web app's window: app_id wadspaces-webapp-<id>
wadbrowser --signin github|tailscale --url <url> [--code <code>]   a sign-in window (below)
wadbrowser --signin github|tailscale --show | --done
wadbrowser --instance <id> [--ephemeral] [...]
  --new-window --profile <name> --gpu auto|on|off --restore|--no-restore -v -V -h
```

- **One browser.** The first `wadbrowser` is the browser (`io.wados.Browser`); a later one hands its arguments
  over D-Bus and exits with its answer. A link opens as a new tab in the last active window of the same mode,
  privacy and profile. `--instance <id>` is a browser of its own (`io.wados.Browser.<id>`, its data under
  `wadbrowser/instances/<id>/`).
- **Unknown flags are ignored**, as in v1, so a link always opens. v1's `--setup` isn't ported: nothing writes one.
  wadd hands a wadspace what it needs in `/run/wadspaces-extra` instead (`webapps.json`, `signins.json` and
  `wadbrowser.conf`, from `crates/wad-gen/src/extras.rs`).
- **`wadbrowser.conf`** (`default = full|focus`): `/run/wadspaces-extra/`'s, then `~/.config/wadbrowser/`'s (the
  user's wins).
- **The modes:**
  - **Full:** tabs above the header, then back, forward and reload, the address, the shield, the profile chip
    and ⋮.
  - **Focus:** the page's title in place of the address.
  - **App:** the web app's icon and name, no tabs while there's one. "Offsite · host" with "Back to <name>" once
    it leaves its site (`site.rs` is the spike's `web.ts`).
  - The app_id is set on realize and again on map (gdk4-wayland). Switching a window to App from the menu keeps
    WadBrowser's app_id, since a shown window can't change it.

## Profiles, keys and the shield

- **Sessions.** Each profile is one WebKit `NetworkSession`, its cookies in `cookies.sqlite`
  (`set_persistent_storage`; Cactus never kept them).
  - **Data:** in `~/.local/share/wadbrowser/profiles/<id>/`, the cache in `~/.cache/wadbrowser/profiles/<id>/`.
  - **Named profiles** are in `~/.config/wadbrowser/profiles.toml`.
  - **Keeping nothing:** private windows, `ephemeral-*` profiles and an `--ephemeral` run keep nothing. A key's
    profile still keeps its data in an ephemeral run (it's the key's, as in v1).
- **Keys in a wadspace.** wadd mounts each key's folder and lists them in `signins.json` (version 2: web keys
  carry `sites`).
  - A web app signs in as its key (profile `signin-<id>`, its data in the key's folder, its cache the
    wadspace's).
  - The browser's own windows sign in as the key whose `apps` has `@browser`.
  - A tab that goes to a site on a key's list moves into that key's session (`profile_for`).
- **Keys on the machine** are wadd's (the library).
  - The page menu's Saved sign-ins edits them through wadd: `secrets.saveWeb` and `secrets.removeSite`, new in
    `wad_model::edit`.
  - "Create app from this page" is `assets.webApp`, and the new tab's own tiles go with `assets.removeWebApp`
    (also new).
  - Using a key's cookies on the machine needs wadd to share its root-only folder: phase 6
    (`HOOK(keys-on-machine)`).
- **The ad blocker** is WebKit's own content blocker, not a decision per request:
  - **Compiling:** EasyList and EasyPrivacy go through adblock's `content-blocking` into WebKit's JSON, and
    `UserContentFilterStore` compiles it once (`~/.cache/wadbrowser/filters/`, keyed by a hash of the lists).
    Later starts only load it. It's all off the main thread; a page waits at most 2 s for it at start.
  - **The social blocker** ("Block all social media") is a second filter.
  - **Per site:** each view has its own UserContentManager, so a site the shield allows simply hasn't the filters.
  - **Counting:** the shield counts once per page and address. `WADBROWSER_FILTER_LISTS=a.txt:b.txt` gives other
    lists, for tests.
- **`cactus://`** is a registered scheme (secure, local, CORS-enabled, since the pages, their stylesheet and its
  fonts are on different hosts).
  - Its pages: `newtab`, `privacy`, `settings`, `history`, `about`, `blocked`, and the error page.
  - **Assets:** `cactus://res/` serves the pages' CSS, script, fonts and icons.
  - **Page actions:** a page asks the browser through the `cactus` script message handler (`PageAction`), which
    only `cactus://` pages have.
- **v1's other parts:**
  - permission prompts, remembered per site (never in a private window);
  - downloads, under WebKit's suggested name, one handler per session;
  - per-site zoom;
  - tabs that sleep after 30 minutes in the background (not one playing sound, downloading, or a popup).
- **Popups** are made with `related_view`, so `window.opener` works (OAuth), and open as a tab beside their
  opener. `window.close()` closes it.

## Sign-in windows (the shell and wadd call them in phase 6)

- **Opening:** `--signin <provider> --url <url> [--code <code>]` opens the provider's sign-in window, or reuses
  it with the new code. It's App-like: the provider's name, no tabs or address bar, the default profile.
- **Showing again and closing:** `--show` brings it up, reopening it where it was if it was closed, so Set up's
  "Show the browser" works. `--done` closes it.
- **The device code:** on `github.com/login/device`, a script fills the code in, into one `user-code` field or
  the `user-code-N` boxes. It fires `input` and `change`, keeps trying through the sign-in redirects, and never
  submits.
- **The contract** is in `crates/wadbrowser-core/src/signin.rs`.

## In the images

- **A wadspace (`images/base`):**
  - `/usr/local/bin/wadbrowser`, built in `images/builder` (`--profile image`), with its libraries (WebKitGTK 6,
    libadwaita, glib-networking);
  - `BROWSER=/usr/local/bin/wadbrowser-link` (it runs `wadbrowser --default`);
  - `/etc/xdg/mimeapps.list` sending http, https and HTML to `io.wados.Browser.desktop` (that name everywhere;
    wad-gen's recipe follows);
  - `wadspaces-webapp`, which makes each web app's launcher (`wadspaces-webapp-<id>.desktop`) from `webapps.json`
    at start.

  The base grew from 1.24 to 1.44 GB.
- **The machine (`host/`):** `/usr/bin/wadbrowser`, its desktop file and icon. The shell opens it from the bar,
  and the machine's web apps as `--app` windows from Start.

## Found on the way

- **Blank pages after the first.** With no settings portal (in a wadspace, on the machine, which has no
  xdg-desktop-portal, and in tests with `GDK_DEBUG=no-portals`), GTK's `gtk-xft-dpi` is -1. WebKitGTK then gives
  every view after the first a zoom of NaN: the page is drawn hugely scaled, and looks blank. `sane_dpi()`
  (`wadbrowser-webkit/src/view.rs`) sets GTK's default (96 dpi) when there's none. The e2e's `pixels` check
  catches it.
- **No WebKit sandbox in a wadspace.** Bubblewrap can't run inside the container ("sandboxing will be
  disabled"). WebKit runs unsandboxed there, inside the wadspace's own container; to look at again under wadd's
  root podman.
- **What WebKitGTK 6 can't do:**
  - **Headers:** DNT and Sec-GPC go only on loads the browser starts (`load_request`); WebKit's `modify-headers`
    rules compile but don't apply. A page's own loads carry only the script's `navigator.doNotTrack` and
    `globalPrivacyControl`.
  - **Counting:** the content blocker tells the UI process nothing, so the shield counts blocked subresources
    with an error listener in an isolated world, and frames at their navigation; blocked fetch/XHR aren't counted.
  - **Moving into a key's session:** with no main-frame flag on a navigation, a tab moves into a key's session at
    the response, so the first request carries the old session's cookies, then it loads again.
  - **The filter store:** `UserContentFilterStore::fetch_identifiers` has no Rust binding; the current filter's id
    is kept in `filters/current-filter`.
- **Cactus's own bugs, fixed:**
  - cookies weren't kept;
  - a download was handled once per window (and kept closed windows alive);
  - scripts were added twice per tab, and every site's hiding rules applied to every later page;
  - blocks were counted twice;
  - the filter lists downloaded on the main thread before the window showed;
  - every load failure got an error page (cancelled loads and downloads too);
  - the window title never followed the page;
  - Ctrl+W on the last tab opened a new one;
  - the new tab always searched DuckDuckGo.

## Not yet

- Tabs can't be dragged (the tab bar is the spike's, not `AdwTabBar`).
- There's no downloads panel; downloads are toasts.
- Cactus's other preferences (session restore, tab sleeping) are only in `prefs.toml`.
- The spike draws sign-in windows as Full windows; ours are App-like.
- The profile picked in "Create app" only opens it; wadd's op has no profile.

## Checks

- `tools/wadbrowser-e2e.sh` (25): cookies kept and a private window's not, the filter compiled once then loaded,
  blocking and counting, a popup with `window.opener`, a key's session, DNT/GPC, a download, an error page, and
  pages drawn (`pixels`).
- `tools/wadbrowser-app.sh` (56): the hand-off (one process), new windows, `--app` (its app_id, reuse), offsite,
  Focus, `--default`, GitHub's code filled in (both forms, after a redirect, never submitted), `--show` and
  `--done`, ignored flags, `--instance`, `--ephemeral`.
- `tools/vm.sh check` step 5: on the machine, a page drawn, a second launch handed over, a web app's window; in
  a wadspace, the browser and its web app's launcher.
- Both scripts use a headless labwc, a home and a D-Bus session of their own, and local pages only.
