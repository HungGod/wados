# Features still to port

Phase 1 ported `cpp`, `rust`, `nodejs`, `python` and `vscode`
(`root/usr/local/lib/wadspaces/features/`). Krita isn't a feature: it is the
RPM app `rpm-krita` (`wadspaces-dnf krita --desktop org.kde.krita.desktop
krita`). Each script's `need` lines must match `crates/wad-gen/src/recipes.rs`,
which has a test that checks them. The table lists v1's other features
(`wadspaces-old/images/base/root/usr/local/lib/wadspaces/features/`), for later
phases. `firebase` and `go` (retired from the catalog) are not coming back.

| Feature | v1 | On Fedora 44 |
|---|---|---|
| `git` | git + git-lfs | No wad-gen recipe yet. The base has git-core; add `git-lfs` if a recipe asks for it |
| `electron-deps` | Electron's libraries on a minimal Debian | Only needed for AppImages and tarballs, since Fedora's RPMs bring their own: nss, gtk3, alsa-lib, mesa-libgbm, libsecret, libnotify, libXScrnSaver, cups-libs, at-spi2-atk |
| `chrome` | Google's .deb + `--no-sandbox` wrapper | Google's RPM repository (dl.google.com/linux/chrome/rpm), the same wrapper (`wadspaces-wrap-electron`) |
| `claude-code` | `npm install -g` + its icon | needs `nodejs`; bring v1's `usr/share/wadspaces/claude-code.png` back with it |
| `obsidian` | AppImage, extracted to /opt | the same, `need electron-deps` |
| `tiled` | AppImage | Fedora's `tiled` (`rpm-tiled`) |
| `libresprite` | AppImage | the same AppImage |
| `audacity` | AppImage | Fedora's `audacity` (`rpm-audacity`), or the AppImage |
| `openshot` | Debian's openshot-qt | not in Fedora (RPM Fusion): decide |
| `steam` | Valve's .deb, i386 graphics | RPM Fusion nonfree with i686 Mesa: decide |
| `dolphin` | linuxserver's build, staged by build.sh | Fedora's `dolphin-emu` (`rpm-dolphin`, with no staging) |
| `android` | Temurin, Gradle, the SDK's command-line tools | the same downloads; Fedora's `java-latest-openjdk` could replace Temurin |
| `android-studio` | Google's tarball in /opt | the same |
| `gcloud` | Google Cloud's apt repository | Google Cloud's yum repository |
| `hplip` | hplip + cups-client | Fedora's `hplip` + `cups-client` |

Also waiting on the browser phase (`HOOK(browser)` in `Containerfile` and
`init-wadspaces-workspace`): `wadspaces-webapp` (web apps' launchers) and
the browser itself.
