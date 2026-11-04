# 0002: building and running a wadspace

Phase 1. `apps/wadd/dev/try.sh` runs all of it rootless, on a headless patched labwc. It builds a wadspace, opens it on the screen, checks its window is `io.wadspaces.<id>`, then stops it. `images/check.sh` proves the images and the rpm merge.

## The images: Fedora 44, v1's composable packages kept

- **The base:** `localhost/wados-base:44` on `ghcr.io/linuxserver/baseimage-fedora:44`, pinned by digest. It has s6-overlay, user `abc`, labwc, selkies-desktop and the clipboard bridge. The builder is `localhost/wados-builder:44`, on Fedora 44 with Rust 1.99.
- **Packages** are v1's, with the same `wadspaces-pkg` command line, label keys and `/usr/share/wadspaces/pkg/<id>/` layout. Only the shared file changed:
  - dpkg's status was text; rpm's database is one SQLite file.
  - Capture keeps the RPM headers a package added (`rpmdb --exportdb`, split by header) as its fragment. Assemble imports them (`rpmdb --importdb`), keeping the newer of two versions of one RPM.
  - The checks after assembly all pass: `rpm -qa`, `rpm -V`, and `dnf install` on top.
  - dnf5's own state is left out: dnf sees the packages' RPMs as installed outside it.
- **Sets** are the base, plus its packages' chain of layers, plus the wadspace's own RUN/COPY lines on top (`wad_gen::sets`). The key covers the packages, the base's image id and the commands. A wadspace whose layer differs from its last build shows "update" and keeps running its old image.
- **What rebuilds one (phase 3):** only its apps, the add-ons that are packages (VS Code's extensions, the command line's tools) and its commands. Its background, icon, name, tag, web apps, sign-ins and what it starts into go into `/run/wadspaces-extra` when it starts. The spike's simulation also counted web apps as rebuilding one; wados doesn't, because they aren't in its image. wad-model's edits (`needs_build`, the Builder's hammer) and wad-gen's set key agree, which `crates/wad-gen/tests/rebuilds.rs` checks for each kind of edit.
- **Timing on this laptop:** a set with one tool built cold in 29 s (the package, its chain and the set). Rebuilt from the cached package, it took 2.7 s.

## Running: what wadd does, and what the shell does

- **wadd starts the container** from `wad_gen::units`. It serves the container a security-context socket for it alone, with app id `io.wadspaces.<id>` (0001), at `<screen_dir>/<id>/wayland-1`, mounted at `/run/wadspaces-display`. It also writes the extras folder.
- **wadd does not place windows.** Each wadspace's run state carries its display mode (screen, window, background), and the shell places its window to match through foreign-toplevel.
- **Finding the compositor:** the shell says at start (`PUT /v1/screen/session`, an absolute socket owned by the caller's uid). Failing that, wadd uses `[display] socket`, then the newest `wayland-*` in `[display] runtime_dir`. A compositor that restarts (its socket's inode changes) gets every socket served again.
- **Ready** means the container's labwc is alive and answers on its own Wayland socket. It only makes that socket after connecting to the one wadd served it. wadd checks this from inside the container (`podman exec`), and the phases go starting → waiting → ready.
- **A race, fixed:** the Starting event woke reconcile on another thread before `open()` had registered the bring-up. Reconcile then saw no container yet, took the wadspace for stopped, and lost its display mode. Now `open()` marks it Starting and registers the task under one lock, and reconcile never takes a Starting wadspace for stopped.
- **The clipboard on the screen** goes through a bridge wadd runs on the host (labwc hides data-control from the wadspace). It reaches the container's labwc through `/proc/<pid>/root` with `RESOLVE_IN_ROOT`, so nothing planted in the container can point at the host.

## Booting it (phase 4)

`tools/vm.sh` makes the host image into a disk, boots it in QEMU with KVM, and checks it end to end (`tools/vm.sh check`, in `tools/vm/check.sh`). None of it needs root or changes the laptop.

- **A disk without root.** `bootc install to-disk` (and bootc-image-builder, `host/build.sh qcow2`'s) needs root. So `tools/vm.sh disk [IMAGE]` runs it in a throwaway builder VM:
  - The builder is Fedora Cloud Base Generic 44, downloaded once into `~/.cache/wados/vm`. Fedora's CHECKSUM file is checked with Fedora 44's key (gpgv), then the image is checked against it. cloud-init takes a NoCloud seed ISO (xorriso) with a key made into `.build/vm`. The builder's own disk is a layer over the download, kept between runs (it has podman already).
  - The images reach it through `registry:2` on 127.0.0.1, which is 10.0.2.2 from the VM. They're pushed as zstd level 1, since they're unpacked a minute later. The registry and the builder both stop at the end, whether the install worked or not.
  - In the builder (`tools/vm/install-disk.sh`): `bootc install to-disk --wipe --generic-image --skip-fetch-check --root-ssh-authorized-keys … --target-imgref <IMAGE> --karg console=tty0 --karg console=ttyS0,115200n8` onto a second disk, `wados-new.qcow2` (40 GiB, sparse). Only a whole install becomes `wados.qcow2`. F44's bootc takes all those flags. `--filesystem xfs` is only passed for an image without `/usr/lib/bootc/install` (fedora-bootc's own).
  - **The base, preloaded.** `localhost/wados-base:44` goes into the new disk's root podman storage, so a terminal-only wadspace runs without CI. It's pulled with the deployment's `/var/lib/containers` bound over the usual path, and with runtime folders of its own: podman's `db.sql` records its graph root and run root, and refuses to start on different ones. Then `restorecon -R` labels it as the machine's own storage (`container_var_lib_t`, `container_ro_file_t`).
  - **Root's SSH, on this disk only.** The image says `PermitRootLogin no` (`40-wados.conf`). The installed `/etc`, never the image, gets `30-wados-vm.conf` with `PermitRootLogin prohibit-password`: sshd takes the first value it reads.
- **Booting** (`tools/vm.sh boot`): q35 with KVM, 2.5 GB, 2 CPUs, and Fedora's OVMF (`/usr/share/edk2/ovmf`). `SECURE_BOOT=1` turns on Secure Boot with Microsoft's keys enrolled, as a new PC ships. The image boots with it as is (shim, GRUB and Fedora's kernel are signed), and the check passed with it too, apart from the restart after a suspend (below).
  - No S3: QEMU's resume from S3 through OVMF starts the VM again rather than waking it, with Secure Boot or without. So S3 is off (`ICH9-LPC.disable_s3=1`), and Linux suspends to idle (s2idle). The check enables the PS/2 keyboard as a wakeup source for that boot, then wakes the VM with Shift. The ACPI power button wakes it too, but on the lock screen it moves the keys to Power off. An RTC alarm doesn't wake s2idle here.
  - The GPU is `virtio-vga` at 1920×1080, with no 3D. The machine's labwc draws with GL through llvmpipe (kms_swrast).
  - `-display none`: QMP (`.build/vm/qmp.sock`, `tools/vm/qmp.py`) takes screenshots (`screendump`, PNG) and types keys (`send-key`, a US layout).
  - Sound: intel-hda + hda-duplex into `-audiodev wav` (`.build/vm/sound.wav`). SSH goes to a free port on 127.0.0.1.
  - It boots `run.qcow2`, a layer over `wados.qcow2`, and `check` starts from a fresh one each time.
- **Rules of the laptop:** one VM at a time (`disk` stops the test VM first), 2.5 GB each, and nothing left running.
- **The check's order** is boot, sam, unlock, a wadspace with sound, the machine's session, Restart, then Power off (QEMU must end). Then it boots the same layer again for the suspend, which ends with QMP's `quit`. The suspend is last, in a boot of its own, because once this VM has slept and woken, its next boot hangs in virtio-gpu's probe (`[drm] pci: virtio-vga detected`, then nothing), whether through Restart or Power off. That happened three times, with Secure Boot on and off, so it's QEMU's device, not WadOS: a real machine's restart after waking isn't tested here. With Secure Boot on (`SECURE_BOOT=1`), everything else passed too.
- **The shell's log lines carry colours** (tracing's ANSI codes, under `systemd-cat`), so journald keeps them as data: `journalctl -g` doesn't find them, and the check greps the text instead.

### What the VM found, and what changed

- **A GPU without 3D (QEMU's virtio GPU, no virgl).** A wadspace's labwc (wlroots' Wayland backend) is handed the machine's render node, which has no Mesa driver. It tries GLES2 and then Vulkan, gives up ("unable to create renderer") and never tries pixman, so the wadspace stops before its desktop is up. wadd now sets `software_render` (`WLR_RENDERER=pixman`) for a render node that is virtio_gpu with its virgl feature bit off (`runs.rs`: `no_3d`), as it already did for NVIDIA's driver.
- **The machine's labwc hands each client shared memory in `/dev/shm`** (`user_tmp_t`): the DMA-BUF format table, and keymaps. `container_t` couldn't map it. `host/…/wados.cil` now allows `container_t user_tmp_t (file (map read getattr))`. A wadspace reaches no such file by path: its `/dev/shm` is its own.
- **wadd's own socket** (no `wadd.socket`) was root:root 0660, so the machine's person couldn't connect. As root, wadd now gives it the first of `allow_groups` the machine has (`wados`). The image uses `wadd.socket` (root:wados), which is unchanged.
- **Unlocking crashed the shell** ("RefCell already borrowed" in `lock.rs`): the session lock's `unlocked` signal arrives during `unlock()`. Fixed in the shell.
- **The screen was black once it was unlocked:** the launcher, desktop and bar never showed, though `wadsctl page` said Launcher. While gtk4-layer-shell 1.3's lock is on, every new surface that isn't a lock surface becomes a stub for good ("non-lock surface created while screen locked, it will not displayed"), and the shell made those after it locked. Fixed in the shell: a window that maps while locked is presented again after the unlock. The check now asserts the screen is drawn after each unlock (`tools/vm/screen.py`), and that the desktop's first icon is drawn below the bar's 78 px reserve (a screenshot diff of its selection: from y=90).
- **Measured** (images c8b1fcc2fb69, then ff830017311c, whose check passed in full): root's SSH 25–33 s after QEMU starts, and the session up and locked 10–12 s after greetd starts again. A terminal-only wadspace is ready in 5–6 s, and its 3 s tone is in `sound.wav` (3.05 s loud, peak 23696). The lock shows 0.6–0.9 s after logind's PrepareForSleep. Restart takes about 30 s to the lock screen. The check takes about 4 min; `disk` takes 5–6 min (push 80 s, builder up 18 s, install 4 min).
- **Worked as root, as designed:** PulseAudio's folder bound read-only in PID 1's mounts, which reaches the running container through its `rslave` screen folder (`native` labelled `wadspaces_sock_t`). The bridge runs as `wadspaces-clip`, connected to both sides. Rootful podman runs with wad-gen's id maps (no image copy: idmapped layers, about 1.5 s for the base) and SELinux enforcing, with categories of the wadspace's own. wadd.service runs under `ProtectSystem=strict` and `ProtectHome=read-only`.

## Building in CI (phase 4)

- **One source for packages and sets.** `wados-pkgs`, a bin of wad-gen's (`crates/wad-gen/src/bin/wados-pkgs`), prints wad-gen's answers for scripts. Those are the packages with their needs and waves, a package's Containerfile and labels, its refs, chain steps, a set's key and Containerfile, and CI's index. `images/build.sh` and CI ask it, so they build what wadd builds, under the same names and keys. Its tests compare each answer with wad-gen's, and the set key with `Set::of`'s.
- **The workflows** (`.github/workflows/`) use only GitHub's token, for ghcr and the `packages` release:
  - `images.yml` builds the builder and the base, then runs `images/check.sh --no-build --no-display`. From main it pushes `ghcr.io/hunggod/wados-{builder,base}:44`. It also runs weekly, for Fedora's updates.
  - `packages.yml` runs after it. It builds every package in waves (`package.yml`, one job per package), pushes each as `ghcr.io/hunggod/wados-pkg-<id>:<abi>` with v1's labels, then writes `index.json` on the `packages` release.
  - `host.yml` builds the host image and runs `bootc container lint` on it. It pushes nothing: `host/build.sh push` does that from the build computer, and the NVIDIA variant's key never comes to CI.
- **What isn't built again.** In CI, a package's Containerfile names the base and its needs by digest. The base carries the feature scripts and `wadspaces-pkg`, so the Containerfile's sha256 covers everything the package is built from. `inputs.json`, next to `index.json`, keeps that sha256 for each published package, and a package whose Containerfile is unchanged is skipped. A package that is built is pushed only when its files changed (its content label, as in v1), so machines keep the one they have.

## Known gaps (for later phases)

- **Projects** come in as a wadspace opens (phase 6, `apps/wadd/src/projects.rs`), each a stage of its open job. wadd clones a GitHub one into `<projects_dir>/<ws>/<id>` (as uid 1000, through the `github_token` secret) and the wadspace works in it where it is. A folder of this machine is mounted, when it's inside the person's home and isn't hidden. A file is copied into `<state>/copies/<ws>/<id>`. Another machine's come over Taildrive: a copy is fetched over WebDAV, a folder mounted with rclone. One that can't be here (its machine offline, a private repository without a sign-in) is left out, and the job says why. What's left:
  - rclone's mount, the 0711 folders and SELinux on a clone are checked in the VM, not by the tests.

  Inside, the image's `wadspaces-projects` takes any name wad-model allows ("My Notes", any script), but for `/`, a backslash, a leading dot and control characters. Its launchers quote the path for gio (`"`, `` ` ``, `$`, `\` escaped, `%` doubled). A copy of one file (its `done` names it) arrives as that file at `~/Desktop/<name>`. `images/base/try-projects.sh` checks this on a laptop, without a container, and runs each launcher's Exec through gio.
- **CI hasn't run yet.** The workflows are checked with actionlint and shellcheck only. They don't build the stream sidecar yet (phase 7, a TODO in `images.yml`), nor the prebuilt WadOS starter set.
- **Packages built by wadd lack the version and content labels**, because wad-podman can't copy a file out of an image yet.
- **Not tested yet:** the clipboard's contents through the bridge as `wadspaces-clip` (it starts and connects; neither image has a tool to copy with). The other root paths run in the VM ("Booting it").
- **Smaller ones:**
  - `lastLaunched` isn't set (wad-model has no edit for it);
  - `screens` is always 1;
  - web app icons wait for the browser (phase 5).
