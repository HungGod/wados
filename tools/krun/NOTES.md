# Secure mode on libkrun: the prototype's notes (phase 7)

What was tried, on the host image (`localhost/wados-host:latest`, Fedora 44
bootc, rootful podman 5.8.9, SELinux enforcing) in a test VM with nested KVM
(`tools/vm.sh --name krun`), November 2026, and what phase 7 ships of it
("Shipped", below). `prototype.sh` repeats the prototype with the shipped
pieces.

## Answer: partly. A secure stream works, limited to CPU and WebSockets

A wadspace's own image (`localhost/wados-base:44`, unchanged) ran as a libkrun
micro-VM through a quadlet. Inside it ran its desktop (s6, its labwc nested in
Selkies' own Wayland compositor, software rendering), PulseAudio and Selkies
2.2.1. A browser on the laptop, reaching the VM's port through SSH, drew the
desktop, clicked its Start button open, and played a tone an app made in the VM.

## Packages (Fedora 44 `updates`)

| package | version | |
|---|---|---|
| crun-krun | 1.30-1.fc44 | `/usr/bin/krun` → crun; needs crun = 1.30 exactly (the image has 1.28: an upgrade) |
| libkrun | 1.19.6-1.fc44 | the VMM; links virglrenderer and libpipewire |
| libkrunfw | 5.6.2-1.fc44 | the guest kernel, Linux 6.12.109 |
| virglrenderer | 1.3.0-1.fc44 | pulled in by libkrun |

That's 4 packages, about 33 MB installed. `passt` (0^20261002) is in the image already.

**What it needs:**
- `/dev/kvm`, which crun-krun adds to the container itself.
- No containers.conf change: podman finds `krun` on its own.
- No SELinux change. container-selinux 2.251 runs the VMM as `container_kvm_t` with the container's MCS level, and no AVC denials appeared in any test.

## What works

| | result |
|---|---|
| `podman run --runtime=krun --rm fedora:44 uname -a` | Linux 6.12.109 (libkrunfw's kernel); 1.7 s for `true`, against 0.9 s for crun (nested KVM) |
| `wados-base:44` with a command | 2.3–3.0 s |
| VM size | default: vCPUs = the CPU affinity (2), 1024 MiB. `krun.cpus` and `krun.ram_mib` annotations set them; `--memory` sets RAM; `--cpus` doesn't set vCPUs |
| rootful uid/gid maps + `idmap` home volume | work as with crun |
| s6 (`/init`) | only through `secure-init` (host/usr/lib/wados/secure): `init.krun` is PID 1 and s6-overlay refuses otherwise ("can only run as pid 1"). `unshare --pid --fork --mount-proc /init` makes it PID 1 of its own namespace |
| headless labwc (pixman) in the VM | yes (grim shot) |
| Selkies 2.2.1, Wayland, its own compositor | software (Pixman), no render node; svc-de nests the wadspace's labwc into it unchanged |
| Selkies from the stream image | an image-driver volume (`Driver=image`) at `/opt/wados-stream:ro`, run in a chroot of it (`svc-secure-stream/run`). `--mount type=image` fails under a user namespace (`Permission denied`, with crun too) |
| `PublishPort=127.0.0.1:3000:3000` | through TSI (no network interface in the VM) |
| browser | first frame 0.9 s after load, about 60 video frames/s over the WebSocket; the Start button opens the menu |
| sound | PulseAudio inside the VM (null sink `output`), Selkies' pcmflux records its monitor. A 440 Hz tone gave about 100 Opus frames/s (about 50 KB/s) in the browser; silence gave none (Selkies' silence gate) |
| project folder | `Volume=<host dir>:/config/Desktop/Project:rw` over virtio-fs, both ways: files land as uid 1000, `container_file_t:s0` |
| outbound | HTTPS and UDP (a STUN round trip to stun.cloudflare.com) through TSI |
| stop | `StopSignal=SIGINT` + `StopTimeout=30`: about 10 s, s6 clean, exit 0 |
| quadlet | `GlobalArgs=--runtime=krun` (→ `podman --runtime=krun run`) and `PodmanArgs=--runtime=krun` both work; `Annotation=`, `Entrypoint=`, `StopSignal=`, `.volume` with `Driver=image` too |

**Cost, measured in the nested VM:**
- **VMM memory:** 364–445 MB resident at `krun.ram_mib=1536` (guest RAM is reserved lazily).
- **CPU:** idle with no viewer, 11.5% of one CPU; one viewer on a still desktop, 64% (x264 in software).
- **Start-up:** Selkies answered 30–60 s after start, and once 157 s, with the laptop deep in swap. The VM and s6 take about 16 s; Selkies' own start takes the rest. That is slow here because of nested KVM and the swap; bare metal should do better.
- **Stop:** 10–13 s.

## What doesn't, or needs work

- **`podman exec`:** not supported ("the handler does not support exec"). wadd's `desktop_up` (apps/wadd/src/backend.rs) execs into the container. For a secure wadspace it needs another probe: Selkies' port answering, or a file the guest writes on a shared folder.
- **SIGTERM never reaches the guest.**
  - Without `StopSignal=SIGINT` and `secure-init`, every stop is a SIGKILL after the timeout.
  - SIGINT is forwarded (other signals aren't), and `secure-init` hands it to s6 as SIGTERM.
- **Inbound UDP under TSI:** doesn't work. A public stream's WebRTC mux port (`PublishPort=<mux>:<mux>/udp`) needs passt mode instead:
  - `Annotation=krun.use_passt=1` plus `Volume=/usr/bin/passt:/usr/bin/passt:ro`;
  - crun runs passt *inside* the container's root, after the mounts, and the base has no passt;
  - crun runs it as `passt -t all -u all`;
  - tested: inbound UDP and TCP through published ports, and outbound both, with `eth0` = the container's 10.88.x address.

  Private streams (WebSockets) are fine on TSI.
- **inotify:** host-side writes into a virtio-fs folder raise no inotify event in the guest; polling sees them at once.
  - Selkies polls `SELKIES_TOKENS_FILE`, which is fine.
  - It watches `SELKIES_RTC_CONFIG_JSON` with watchdog (inotify), so wadd's TURN rotation wouldn't be seen. Fix: a guest-side relay that copies the file into a guest tmpfs when its mtime changes (or Selkies polling it).
- **GPU:** not testable here.
  - With `krun.gpu_flags` on the test VM's render node (virtio-gpu, no 3D), libkrun's "gpu worker" segfaults (exit 139), on both Venus and the DRM native context flags.
  - Real Intel/AMD hardware with virglrenderer may work, but that needs a bare-metal test. wadd must offer it only where it's known to work: a crash, not an error.
  - Until then: CPU encoding, `WLR_RENDERER=pixman`.
- **Sound from the machine:** a host PulseAudio socket can't cross. crun never calls `krun_set_snd_device` (no virtio-snd through podman); sound stays inside the VM, which is what a stream needs.
- **The display volume:** not needed, and can't be shared (a Wayland socket can't cross). No sidecar.
- **Screen and stream moves:** a secure wadspace can't move onto the machine's screen while it runs. "The person at the machine wins" means stopping the VM and starting it again as a normal container.
- **The machine's clipboard:** no bridge.
- **SELinux:**
  - With `SecurityLabelDisable=true` (wad-gen's choice for a wadspace with a Mount project), the VMM runs unconfined as `container_runtime_t`. Secure mode should refuse Mount projects or keep a level (relabelled folders).
  - Files the host image mounts in (`/usr/lib/wados/secure`, `lib_t`) are read without a relabel. Files under `/root` (`admin_home_t`) are denied.
- **The stream image's volume:** created once from the image by its `.volume` unit, a oneshot that stays active.
  - Removing the volume alone leaves podman to make an empty one at the next start.
  - wadd must stop `wadstream-<id>-volume.service` and remove the volume when a stream ends or the image changes.
- **Selkies' needs in the stream image:** pixelflux's compositor dlopens `libwayland-server`, and pcmflux needs `libICE`/`libSM`. The real `images/stream` already installs them.

## Shipped in phase 7: Secure (limited)

Private WebSocket streams only, CPU encoding, sound inside the VM. Public secure streams are refused with "Secure public streams come later: secure streams are private for now".

- **wad-gen** (`crates/wad-gen/src/units.rs`): `secure_units(ws, run, machine, sidecar, vm)`.
  - It writes `wad-<id>.container` (the wadspace's own image in krun, with Selkies inside) and `wadsecure-<id>.volume` (`Driver=image`, the stream image).
  - It refuses a Mount project here, and a public stream (`SecureRefused`).
  - The stream image's `SELKIES_*` ENV is repeated as `STREAM_IMAGE_SELKIES_ENV`, because image ENV doesn't reach a chroot. A test keeps it equal to `images/stream/Containerfile`.
  - The example `secure-units` prints them, for `prototype.sh`.
- **The host image:**
  - `host/Containerfile` installs `crun-krun` and checks it.
  - The guest side is in `host/usr/lib/wados/secure/`:
    - `secure-init`;
    - `svc-secure-stream/`: PulseAudio from the image's `stream.pa`, then Selkies, chrooted into `/opt/wados-stream`. It halts the VM without a master token, a token table and a `/ws-` path.
    - `empty`.
- **wadd:**
  - `runs.rs` writes `secure_units()` for a stream whose `StreamUnit::vm` is set. It waits for Selkies' health on the published port, never `desktop_up`: the fake backend, as crun-krun, refuses `exec` into a VM.
  - `streams.rs`:
    - it refuses Mount projects (409);
    - public-secure is never available;
    - its teardown stops `wadsecure-<id>-volume.service` before `volume rm` (and does the same for the display volume);
    - adopting after a restart checks the VM's health, not a sidecar.
  - `[streaming] secure_cpus` (2) and `secure_ram_mib` (2048) set the VM's size. `Settings::with_kvm` takes the KVM path, for tests.
- **Checks:**
  - `prototype.sh` runs the real pieces in a test VM, without wadd.
  - `check-secure.sh` runs a private secure stream through wadd end to end, on a disk built from this tree and on a tailnet.

## The unit, as wad-gen writes it

One container, the wadspace's own image; no sidecar and no display volume. Compared with the screen's unit, it adds or changes:

| key | value |
|---|---|
| `GlobalArgs` | `--runtime=krun` |
| `Annotation` | `krun.cpus=<secure_cpus>`, `krun.ram_mib=<secure_ram_mib>` |
| `Entrypoint` | `/usr/libexec/wados-secure-init` |
| `StopSignal`, `StopTimeout` | `SIGINT`, `30`; `[Service] TimeoutStopSec=60` |
| `Volume` (stream image) | `wadsecure-<id>.volume:/opt/wados-stream:ro` |
| `Volume` (guest side) | `/usr/lib/wados/secure/…`: secure-init, the s6 service and its `user/contents.d` entry |
| `Volume` (the stream's folder) | `<state>/streams/<id>:/run/wadspaces-selkies:ro,Z` (its token table: Selkies polls it) |
| `Environment` | `WLR_RENDERER=pixman`; `PULSE_SERVER=unix:/run/wadspaces-current/pulse/native` (the VM's own PulseAudio); the image's and the stream's `SELKIES_*` (websockets, `/ws-<guid>`, the tokens file, CPU encoding) |
| `Secret` | `wad_stream_master_<id>,type=env,target=SELKIES_MASTER_TOKEN` |
| `PublishPort` | `127.0.0.1:<web_port>:3000` |
| `SecurityLabelLevel`, `UIDMap` | as the screen's (never `SecurityLabelDisable`) |

It drops:
- the screen's folder and Pulse bind;
- `AddDevice=/dev/dri`;
- device cgroup rules.

Public secure would add `Annotation=krun.use_passt=1`, `Volume=/usr/bin/passt:/usr/bin/passt:ro`, and a relay for the RTC file, which Selkies watches with inotify.

## Files

| file | what it is |
|---|---|
| `prototype.sh` | wad-gen's units, the host's guest side and the real stream image, in a test VM without wadd: `setup`, `plain`, `up [--check]`, `view`, `check`, `down`, `vm-stop` |
| `check-secure.sh` | a private secure stream through wadd, start to nothing left, for `tools/vm/`'s checks |
| `guest/check/90-krun-check.sh` | `up --check` only: the tone, and the folder both ways |
| (`view.mjs`) | Playwright on the stream: screenshots, the Start click, audio and video frames per second. Gone with the v2 spike's Playwright (and Node.js); `WADOS_KRUN_VIEWER` takes its place |
| `wavstat.py` | how loud the guest's recording was |
