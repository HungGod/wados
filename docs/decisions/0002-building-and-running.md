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
- **Timing on this laptop:** a set with one tool built cold in 29 s (the package, its chain and the set). Rebuilt from the cached package, it took 2.7 s.

## Running: what wadd does, and what the shell does

- **wadd starts the container** from `wad_gen::units`. It serves the container a security-context socket for it alone, with app id `io.wadspaces.<id>` (0001), at `<screen_dir>/<id>/wayland-1`, mounted at `/run/wadspaces-display`. It also writes the extras folder.
- **wadd does not place windows.** Each wadspace's run state carries its display mode (screen, window, background), and the shell places its window to match through foreign-toplevel.
- **Finding the compositor:** the shell says at start (`PUT /v1/screen/session`, an absolute socket owned by the caller's uid). Failing that, wadd uses `[display] socket`, then the newest `wayland-*` in `[display] runtime_dir`. A compositor that restarts (its socket's inode changes) gets every socket served again.
- **Ready** means the container's labwc is alive and answers on its own Wayland socket. It only makes that socket after connecting to the one wadd served it. wadd checks this from inside the container (`podman exec`), and the phases go starting → waiting → ready.
- **A race, fixed:** the Starting event woke reconcile on another thread before `open()` had registered the bring-up. Reconcile then saw no container yet, took the wadspace for stopped, and lost its display mode. Now `open()` marks it Starting and registers the task under one lock, and reconcile never takes a Starting wadspace for stopped.
- **The clipboard on the screen** goes through a bridge wadd runs on the host (labwc hides data-control from the wadspace). It reaches the container's labwc through `/proc/<pid>/root` with `RESOLVE_IN_ROOT`, so nothing planted in the container can point at the host.

## Known gaps (for later phases)

- **Projects** (GitHub clones, mounts, copies) are left out of a run with a logged note. The units expect `<state>/copies/<id>` folders (phase 6).
- **`images/build.sh` keeps its own copy** of wad-gen's package order, labels and set key. It was written while the workspace couldn't load. CI's packages job (phase 4) should replace it with a small tool that calls wad-gen.
- **Packages built by wadd lack the version and content labels**, because wad-podman can't copy a file out of an image yet.
- **Not tested yet:** the machine's root paths (PulseAudio bound into PID 1's mounts, the bridge as `wadspaces-clip`) wait for the host image (phase 4).
- **Smaller ones:**
  - `lastLaunched` isn't set (wad-model has no edit for it);
  - `screens` is always 1;
  - web app icons wait for the browser (phase 5).
