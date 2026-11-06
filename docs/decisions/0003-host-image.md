# 0003: the host image

Phase 4. `host/Containerfile` is the machine's own OS: Fedora 44 bootc, booting through greetd into our labwc and
the shell, with wadd running the wadspaces on root's podman. `host/build.sh image` builds it as
`localhost/wados-host:latest`; `host/Containerfile.nvidia` adds NVIDIA's own driver
(`localhost/wados-host-nvidia:latest`). It's v1's host image (`wadspaces-old/host/`) ported, without sway, the
Tauri client, the HUD, Firebase, the kiosk and admin users, and what belongs to later phases.

## What's in it

On `quay.io/fedora/fedora-bootc:44`, pinned by digest (`host/build.sh` rechunks with the same one):

- **The screen:** our labwc (below), Xwayland (wlroots won't start without it; it runs only for an X client),
  greetd, GTK 4.22 with gtk4-layer-shell 1.3, libadwaita, WebKitGTK 6 (the browser lands in phase 5), foot, gvfs,
  dbus-tools (labwc updates the D-Bus activation environment with it), wlr-randr.
- **Graphics and media:** Mesa's GL, Vulkan (ANV, RADV, NVK) and VA-API (in `mesa-dri-drivers` on Fedora 44),
  Intel's media driver, `vainfo`, GStreamer's good, bad-free, libav and openh264 plugins (WebKitGTK plays through
  them).
- **Sound:** PipeWire, its PulseAudio socket and WirePlumber, as user units enabled for everyone
  (`systemctl --global`). The shell runs `wpctl`.
- **Networks:** NetworkManager with Wi-Fi (and `nmtui`), BlueZ, podman, Tailscale 1.104.5 and cloudflared
  2026.10.7.
- **Firmware:** v1's (Intel, AMD and NVIDIA GSP graphics; Intel, SOF and Cirrus sound; Intel, Realtek, Atheros,
  Broadcom, MediaTek and the Surface's Marvell Wi-Fi).
- **Fonts:** Inter and Space Grotesk from `assets/fonts` (in `/usr/share/fonts/wados`; `assets/fonts` links
  there), and Noto Sans, Serif, Sans Mono and Color Emoji.
- **WadOS's own:** `wadd`, `wados-shell` and `wadsctl` in `/usr/bin`, the clipboard bridge at
  `/usr/libexec/wados/wadspaces-clipboard` (where `apps/wadd/src/clip.rs` looks), and `assets/` at
  `/usr/share/wados/assets` (where wad-gtk, the shell and wadd look).
- **Not in it:** `xdg-desktop-portal`. The shell opens its own file dialog (GTK's, in process) and files (GIO),
  and nothing else on the machine needs a portal. labwc's Fedora package requires `xdg-desktop-portal-wlr`; ours
  doesn't.

## labwc: ours, as an RPM

The image's first stage rebuilds Fedora's own `labwc-0.9.6-1.fc44` source package (its signature checked against
Fedora 44's key) with `host/labwc/*.patch` added as `Patch1001…`, against Fedora 44's wlroots0.19, as
`tools/build-labwc.sh` does for a laptop. The result is `labwc-0.9.6-1.fc44.wados1`: dnf installs what it needs
to run, `rpm -V` knows its files, and Fedora's unpatched labwc never goes in. The build checks the release is
ours and that no portal came along.

## How the session starts

1. **greetd** is the display manager (`display-manager.service`), on tty1 (getty@tty1 is masked). Its drop-in
   (`usr/lib/systemd/system/greetd.service.d/10-wados.conf`) runs `/usr/libexec/wados/greetd-config` first. That
   looks up uid 1000 (the machine's person, made by the installer in phase 9; there's no `wad` or `admin` user)
   and writes `/run/wados/greetd.toml`:
   - with a uid 1000: `initial_session` is `/usr/libexec/wados/session` as them (greetd's autologin, through
     the full `greetd` PAM stack: SELinux user, loginuid, a logind session);
   - always: `default_session` is agreety, the login prompt, as `greetd`. With no uid 1000 it's all there is.

   It removes greetd's runfile, so a greetd started again logs the person in again. **SELinux:** greetd runs as
   `xdm_t`, which can't read `/run`'s own type (`var_run_t`), so WadOS's module gives that one file
   `xdm_var_run_t` and the script restores its label.
2. **The session script** sets `WADOS_SESSION=machine`, `XCURSOR_THEME=WadOS` (size 24), and pixman/cairo
   rendering when there's no render node (v1's: nomodeset, or a card no driver runs). It runs
   `labwc -m -s wados-shell` into the journal (`journalctl -t labwc`) and starts it again if it ends. Five quick
   ends in a row (under 30 s each) and it gives up, so greetd shows the login prompt. `-m` (`--merge-config`,
   phase 8): the person's `~/.config/labwc` (Settings ▸ Mouse & Touchpad and ▸ Keyboard write its `rc.xml` and
   `environment`) is read over `/etc/xdg/labwc`, whose keybinds stay; without it labwc reads only the first
   `rc.xml` it finds, so a person's would drop every `wadsctl` keybind.
3. **The shell locks before anything else shows** (it does unless `WADOS_NO_LOCK` is set; the script unsets
   it). Its password check is PAM service `wados-lock` (`/etc/pam.d/wados-lock`): `system-auth`'s auth and
   account, so pam_unix checks the person's own password through `unix_chkpwd` (setuid on Fedora 44), with
   `pam_faildelay`'s two seconds after a wrong one. There's no `pam_faillock`: its tally is root's to write,
   and the shell isn't root.
4. labwc reads `/etc/xdg/labwc/rc.xml` (the shell's keybinds; it names the theme), and the person's
   `~/.config/labwc` over it. `/etc/systemd/logind.conf.d/10-wados.conf` leaves the power key to the shell.

The disk is LUKS-encrypted by the installer (phase 9), which is why autologin is acceptable.

## wadd as root

- **`wadd.socket`** listens at `/run/wadd/wadd.sock`, `root:wados 0660`. wadd decides per request by
  `SO_PEERCRED` (`apps/wadd/src/access.rs`): root, and members of `[daemon] allow_groups` (`wados`, `wheel`). The
  socket's group is `wados`, which the person is in (the installer puts them in `wados` and `wheel`; the VM's
  test user too). A `wheel`-only user can't connect; there's none by design.
- **`wadd.service`** is `Type=notify` (wadd says READY), root, with root's podman (`podman.socket`,
  `/run/podman/podman.sock`) and systemd's manager for the wadspaces' quadlets. It's sandboxed as v1's was:
  `ProtectSystem=strict`, home read-only, `PrivateTmp`, `NoNewPrivileges`; writable are `/var/lib/wados`
  (`StateDirectory`), `/run/containers/systemd` (the units), `/run/wados/screen` (`RuntimeDirectory`, kept
  across restarts because running wadspaces have their folders mounted) and `/etc/cloudflared`. Binding
  PulseAudio's folder in PID 1's mounts (`mounts.rs`) should still work from its namespace, since it's a
  `setns` (the VM checks it).
- **`/usr/lib/wados/wadd.toml`** is a v2 config with no Cloud or Keys section. It says again the defaults the
  image's own files depend on (the socket, state, keys, projects, units and screen folders, podman's socket,
  the allowed groups, tailscaled's socket, cloudflared's metrics), so a change to wad-config's defaults can't
  quietly part them from the units.
- **Users** (`usr/lib/sysusers.d/wados.conf`, also made at build so a disk image's test user can join `wados`):
  group `wados`, and `wadspaces-clip`, locked, which the clipboard bridges run as (`clip.rs`: never root). The
  wadspaces' id range (`wadspaces:1000000:65536`, wad-gen's `USERNS_BASE`) is in `/etc/subuid` and `/etc/subgid`,
  so useradd never hands it out (its ranges start at 524288 and may reach it).
- **Folders** (`usr/lib/tmpfiles.d/wados.conf`): `/var/lib/wados` and its `keys` and `projects` (0700 root;
  wadd makes each key's folder, uid 1000's), `/run/containers/systemd`, `/run/wados`, `/etc/wados`,
  `/etc/cloudflared` (0700), and tailscaled's.
- **cloudflared.service** runs only with `/etc/cloudflared/token` (the installer's, phase 9), with
  `--no-autoupdate` and its metrics on `127.0.0.1:20241`. **tailscaled** is enabled.

## SELinux

Enforcing. `usr/share/selinux/packages/wados/wados.cil` is v1's `wadspaces.cil`, loaded with `semodule -i` and
checked with `semodule -l` and `matchpathcon`:

- `wadspaces_sock_t`, which PipeWire's PulseAudio socket (`native`, made by the person's `unconfined_t` in
  `user_tmp_t`) is born as; `container_t` may write it and connect to the session. wadd labels it so too, and
  asks the policy first (`selinux.rs`: `PULSE_SOCKET`, `pulse_allowed`). The type's name is wadd's, so it stays.
- greetd's runtime config as `xdm_var_run_t` (above).

What wad-gen's units rely on is the stock policy: each wadspace runs as `container_t` with categories of its
own (`SecurityLabelLevel`), its screen folder and socket labelled for containers by wadd, `/dev/dri` through
`container_use_dri_devices` (on by default on Fedora 44), and `SecurityLabelDisable` for a wadspace with a mount.
Whether a wadspace plays sound through it on the machine is the VM's check.

## The Surface, and booting

- v1's fixes: `usr/lib/modprobe.d/wados-surface.conf` (the GPIO and DMA drivers before the controller hub) and
  `usr/lib/dracut/dracut.conf.d/60-wados-surface.conf`. The initramfs is made again and checked for the
  Surface's modules, i915's `kbl_dmc` firmware, `virtio_scsi` (a VM's disk; virtio-blk and virtio-pci are
  built into Fedora 44's kernel), XFS and `systemd-cryptsetup` (the LUKS disks of phase 9). The `qemu` dracut
  module is no longer left out.
- Kernel arguments `usbcore.autosuspend=-1`; GRUB sets the screen's own mode (`gfxpayload=auto`) through
  bootupd's static config; `bootc install` defaults to XFS; NetworkManager's Wi-Fi power save is off (mwifiex).
- **Updates are wadd's (phase 8):** the base enables `bootc-fetch-apply-updates.timer`, which stages *and reboots*;
  this image masks it. wadd checks every 12 hours and stages what it finds; the person restarts into it
  (`apps/wadd/src/updates.rs`). The image's version is `org.opencontainers.image.version` (`host/build.sh`).
- `CtrlAltDelBurstAction=none`, `ctrl-alt-del.target` masked (Ctrl+Alt+Delete is the shell's).
- sshd is the base's, enabled, for keys only (`PermitRootLogin no`, no passwords).
- The hostname is `wadspaces` until the installer names the machine.

## The theme: `WadOS`

`/usr/share/themes/WadOS/openbox-3/themerc` is labwc's theme in the spike's dark tokens, for what labwc
decorates itself (foot's windows) and its own menus and boxes. The cursor theme of the same name
(`/usr/share/icons/WadOS`) is drawn from `host/cursors/*.svg` (the arrow, the hand and the I-beam, night with a
light edge) and rendered in a build stage (`rsvg-convert`, `xcursorgen`) at 24 to 96 px; every other cursor is
Adwaita's. labwc takes it from `XCURSOR_THEME`; GTK from `/etc/xdg/gtk-4.0/settings.ini`, which also prefers
dark. foot's colours are v1's, and its windows are `io.wados.Terminal`.

## The variants

- **host:** everything above. The open drivers, so nouveau and NVK on an NVIDIA card.
- **nvidia:** v1's `Containerfile.nvidia` on Fedora 44: RPM Fusion's `akmod-nvidia`, the open kernel modules
  (`MODULE_VARIANT=kernel-open`), built by akmods for the image's own kernel (kernel-devel from the repos, else
  Koji) and signed with the MOK key, then NVIDIA's libraries, `libva-nvidia-driver`, egl-wayland and egl-gbm.
  RPM Fusion's repos are removed again. nouveau is kept away by kernel arguments. Checked: the module is for the
  image's kernel, its license is the open one's, and its signer is the key's certificate's name. The public
  certificate is in the image (`/usr/share/wados/mok/wados-mok.der`) for a machine to enroll (phase 8).
- **The MOK key** comes from `${WADOS_SECRETS:-~/.config/wados/secrets}/mok/` (`wadspaces.priv` and
  `wadspaces.der`, or one `.priv` and one `.der`), handed to podman as build secrets: the signing step sees them
  in `/run/secrets`, gives akmods a group-readable copy and removes it in the same step, so it's in no layer.
  `build.sh` looks only at the files' names. **Without it the variant is skipped**, with a message; nothing
  makes a key.
- **`--test-key`** makes a throwaway key in `.build/mok-test/` and says loudly that it's a test key. The image
  it signs is `localhost/wados-host-nvidia:test-key` (never `:latest`), labelled `io.wados.mok=test`, and
  `push` refuses it (by the flag and by the label, before it logs in).
- **Built with a test key (phase 4):** RPM Fusion's akmod-nvidia 615.78.08 built for 7.2.14-200.fc44,
  signed `WadOS TEST module key (never push)`, license `Dual MIT/GPL`. The image is 4.30 GB (65 layers) and
  passes lint. No `.priv` is in it, the build stage's `private/` is empty, and the log holds no key. The run
  took 15 min 22 s with the host image cached. Without a key it's skipped with the message
  (16 s).

## Building

`host/build.sh image`:

1. **The binaries:** `localhost/wados-host-builder:44` (`host/builder`: fedora:44, gtk4-devel,
   gtk4-layer-shell-devel, pam-devel, Rust 1.99) runs `cargo build --release --locked -j2` with the checkout
   read-only and cargo's registry and target dir under `.build/host`, then strips them into `.build/host/bin`
   (symbols kept in `.build/host/debug`).
2. **The image:** `podman build` with `host/` as the context (its `.containerignore` keeps any key out), plus
   `bin` and `assets` contexts. One stage builds labwc, one the cursors.
3. **Rechunked** with Fedora 44's `rpm-ostree compose build-chunked-oci` (64 layers by package; WadOS's own
   files, marked `user.component=wados`, in one), unless `RECHUNK=0`. A rootfs has no labels, so the built
   image's are passed on again.

`host/build.sh qcow2|iso|raw` runs bootc-image-builder with sudo, as v1. Its config is generated in
`.build/bib/config.toml`: user `sam`, uid 1000, in `wheel` and `wados`, with `WADOS_VM_PASSWORD` or a random
password it prints, and `WADOS_VM_SSH_KEY` if given; root's 60 GiB. `push` sends `ghcr.io/hunggod/wados-host[-nvidia]`.
`install`, `update` and `installer` say they're phase 9.

The checks in the build: our labwc's release; no portal; the initramfs's modules; Inter and Space Grotesk in
fontconfig; the module loaded and greetd's file labelled; the users made; every binary loads (`ldd -r`: no
missing library or symbol) and `wadd version`, `wadsctl --help`, `labwc --version`, labwc starting headless
with `/etc/xdg/labwc` and logging nothing, `foot --check-config`,
Tailscale's and cloudflared's versions run; the tools are there (`nmcli`, `wpctl`, `bluetoothctl`, `skopeo`,
`bootc`, …); the firmware is there; the theme, cursors and assets are there; rc.xml names the theme and logind
leaves the power key; `wadd.toml` parses; `greetd-config` writes a right config with and without a uid 1000;
and `bootc container lint`.

**On this laptop** (4 cores, 7 GB, shared with a VM): the binaries took 11 min 48 s from cold (cargo -j2), the
labwc stage 7 min, and the image 12 min, so about half an hour in all. A rebuild with warm caches, the
initramfs included, then rechunked, took 6 to 7 min. The labwc stage is cached by podman like any layer: a new
labwc build (a removed cache) makes a new RPM, and every layer after it runs again. The image is **3.20 GB** rechunked (65 layers), 3.63 GB as built; the base is 1.95 GB.
The biggest of what's added: Mesa's Vulkan drivers and LLVM (llvmpipe), WebKitGTK, the GPUs' and Wi-Fi's
firmware, Tailscale. The binaries: wadd 12.4 MB, wados-shell 13.8 MB, wadsctl 0.3 MB, the bridge 0.7 MB.

## What was found

- **greetd can't read a file in `/run`:** it runs as `xdm_t`, which reads `xdm_var_run_t` there but not
  `var_run_t`. Hence the module's file context for `/run/wados/greetd.toml` and the script's `restorecon`.
- **Fedora's labwc requires `xdg-desktop-portal-wlr`**, which brings the portal and its pipeline. Our RPM drops
  the requirement; the build checks neither portal is installed.
- **wlroots won't start without Xwayland** when labwc is built with it (`wlr_xwayland_create` fails and labwc
  exits), so Xwayland is installed; it starts only when an X client connects.
- **Fedora 44 has no `mesa-va-drivers` package:** VA-API is in `mesa-dri-drivers`.
- **fedora-bootc:44 enables `bootc-fetch-apply-updates.timer` and sshd.** The timer is masked (wadd decides, phase 8); sshd
  is kept, keys only.
- **`unix_chkpwd` is setuid on Fedora 44**, so the lock's PAM check works as the person. `pam_faillock` can't:
  its tally is root's.
- **virtio-blk and virtio-pci are built into Fedora 44's kernel**, so a VM's disk needs nothing from dracut;
  virtio-scsi is a module, and it's checked.
- **dracut on Fedora 44** has a `clevis-pin-file` module that needs `clevis` (left out too now), and couldn't
  copy `/root` (a link to `var/roothome`, which a container build hasn't got: made for that step).
- **`rpm-ostree compose build-chunked-oci` works on Fedora 44** (rpm-ostree 2026.3), rootless, from a mounted
  image. Two things: the OCI directory it writes into must exist first (`oci-layout`, an empty `index.json`),
  and from a `--rootfs` it keeps no labels, so `build.sh` passes the built image's with `--label`.
  `user.component` still makes WadOS's chunk.
- **foot 1.27** calls `[colors]` deprecated: it's `[colors-dark]` now.
- **A mirror's bad metadata** once failed the cloudflared step, whose dnf read the repos for nothing: that
  RPM needs nothing else, so it's installed with the repos off now.
- **labwc makes Xwayland's lazy socket** (`/tmp/.X11-unix`) even when it only starts for a check; the check
  removes it (lint's `nonempty-run-tmp`).
- **labwc's icon theme is its theme's name** unless rc.xml says `<theme><icon>`, so it looked for an icon
  theme called WadOS and found only the cursors (libsfdo: "Directories is unset"). The WadOS index is a valid
  icon theme now, with no icons of its own (Adwaita's, then hicolor's), and the build starts labwc to check it.
- **Run in the image** (labwc headless, the real shell as a uid 1000 user): the shell starts, locks first
  (`wadsctl locked`: `ok true`) and says `machine=true`.
- **`bootc container lint`:** 13 checks pass, with one warning: `/var` folders Fedora's packages ship without
  tmpfiles entries (greetd's home, libX11's compose cache, BlueZ's state). They're the packages', and harmless.
- **The pins, checked by hand:** Tailscale publishes no checksum file for its RPMs; the sha256 is the one in its
  repo's `primary.xml` (the repo's metadata is signed). cloudflared's is in its release notes and GitHub's
  asset digest, which agree.
- **The useradd ranges** start at 524288 and may reach 600100000 on Fedora 44, so reserving the wadspaces'
  range in `/etc/subuid` matters.
- **The VM found one more SELinux rule** (`tools/vm.sh check`): a wadspace's labwc maps the shared memory the
  session's labwc hands it on its socket (`user_tmp_t`, from `/dev/shm`), or it gets no GPU buffers. It's in
  `wados.cil`.
- **The VM found two bugs in the lock**, which only a session that really locks first could show:
  - Unlocking with the right password crashed the shell: `unlock()` held the lock's cell borrowed while
    gtk4-layer-shell emitted `unlocked`, whose handler takes it out.
  - After that, the screen stayed black: while gtk4-layer-shell's lock is on, it gives every other new surface
    a stub that never shows (its `lock-surface.c`), and the shell makes its launcher, desktop and bar after
    locking. The lock now notes each window of the shell's shown while it's on, and shows it again once
    unlocked (`lock.rs`), which also covers a toast or a display's bar shown during a later lock.
- **In a VM with no 3D** (virtio-gpu without virgl), a wadspace's own labwc can't make a renderer and doesn't
  fall back, so wadd sets `WLR_RENDERER=pixman` when every render node is like that.

## Later

- **Phase 8:** updates (`bootc-fetch-apply-updates`, the switch between variants, enrolling the MOK
  certificate), and `/etc/ostree` for wadd if it logs in for them.
- **Phase 9:** the installer (`install`, `update`, `installer`), provision, and the person in `wados`.
- **Phase 6:** projects. `/var/lib/wados` is root's alone (wadd makes it 0700), so if git runs as uid 1000 in
  `/var/lib/wados/projects/<id>` it can't reach it; the folder may need to move, as v1's did.
- **Phase 10:** the gamepads' udev rules.
- **Trimming:** the base carries `python3-botocore` (119 MB) that nothing here needs.
