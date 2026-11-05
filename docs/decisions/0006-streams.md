# 0006: streams

Phase 7. A wadspace can be streamed to a browser through its own Selkies sidecar, in one of three modes:
- **Private:** over the tailnet, in WebSocket mode.
- **Public:** at the machine's host through its Cloudflare tunnel, with WebRTC and Cloudflare TURN for the media.
- **Secure (limited):** private, in a libkrun micro-VM.

A stream has a one-tap link with its key, and a public one also has a user and password. The person at the machine
always wins: opening a streamed wadspace on the screen ends its stream.

## The sidecar (`images/stream`, `localhost/wados-stream:44`)

- **What's in it:** upstream **Selkies 2.2.1** (with pixelflux 2.2.3 and pcmflux 2.2.1) from PyPI, all 31 wheels
  pinned by version and sha256 (`requirements.txt`, `lock.sh`), on the base's Fedora 44 (s6, user abc). It's 865 MB.
  LinuxServer's desktop image isn't used: it bundles a desktop, nginx and Docker, and it lags upstream.
- **How it attaches (v1's layout):**
  - Selkies' compositor owns `wayland-1` in the display volume, and the wadspace's labwc nests in it.
  - PulseAudio's socket is in the same volume.
  - The wadspace sees the volume at `/run/wadspaces-stream`; the sidecar sees it at `/run/wadspaces-display`.
- **Secure mode in every sidecar:**
  - `SELKIES_MASTER_TOKEN` comes from the podman secret `wad_stream_master_<id>`.
  - `SELKIES_TOKENS_FILE` (one controller row, the stream's key) and `SELKIES_RTC_CONFIG_JSON` come from
    `<state>/streams/<id>/`, which wadd writes: 0700, files 0600, owned by uid 1000 (the sidecar's abc), each file a
    temp file renamed in place in that folder.
  - `SELKIES_SUBFOLDER=/ws-<guid>`, and no basic auth.
  - Selkies' own Cloudflare TURN, file transfers, the apps panel and gamepads are off (gamepads are phase 10).
- **Transport:** private streams use `SELKIES_MODE=websockets`. Public ones use `webrtc` with dual mode and the mux
  ports.
- **Moving a running desktop (`wad_gen::units::moving_units`):** a wadspace whose image has
  `io.wadspaces.display=2` mounts both its screen folder and the display volume.
  - Taking it off the screen means: start the sidecar, write the `display` file, then `wadspaces-session switch`.
    Its apps and terminals keep running. Coming back is the reverse.

## The proxy (`apps/wadd/src/proxy.rs`)

- **One host per machine,** routed by path:
  - Public: `https://<host>/ws-<guid>/` on `127.0.0.1:8085`, cloudflared's target.
  - Private: one loopback listener per stream, `tailscale serve --https=<port>` → `127.0.0.1:<port>`, serving only
    that stream.
- **Routing:** the path and Host go through unchanged (Selkies checks Origin against Host). WebSockets are upgraded,
  with nothing buffered.
  - `/ws-<guid>` redirects to the trailing slash.
  - An unknown link gets **"This link has expired"**.
  - A sidecar that isn't up gets a retrying "isn't answering yet" page.
- **The key is checked by the proxy,** in constant time, before anything reaches Selkies: the `selkies_token` cookie,
  `?token=`, a Bearer header, or Selkies' `selkies.token.<base64url>` subprotocol. Only Selkies' `api/status` and
  `api/health` are open.
- **Signing in:**
  - **One-tap** (`/ws-<guid>/#token=<key>`): the fragment never reaches a server. The page takes it off the address
    bar, POSTs it for the HttpOnly cookie (`Path=/ws-<guid>/`, Secure, SameSite=Lax, 12 h), then reloads.
  - **User and password** (public): the sign-in card, rate-limited after 5 wrong tries.
  - The pages are self-contained, in the spike's look, with a strict CSP.
- **Selkies 2.2.1 never reads the cookie on its sockets.** Its data socket takes the key only from `?token=` or the
  subprotocol, and WebRTC signalling only from the first message's `client_token`. So for a viewer signed in by the
  cookie, the proxy adds `token=<key>` to the data socket's upstream request, and fills the key into an empty
  `client_token` in signalling's first `HELLO`. Found on the VM; the key still never reaches the address bar or a
  log.
- **The tunnel's status** (`tunnel.rs`): public is available with `[streaming] host` set, `/etc/cloudflared/token`
  present, the unit active, and cloudflared's `/ready` connected. Each failure has its own reason.
  `[streaming] tunnel = "external"` (tests only) counts a set host as available, for a quick tunnel.

## The lifecycle (`apps/wadd/src/streams.rs`, `turn.rs`)

- **Start** is a Stream job:
  - checks: available, built, image present;
  - takeover (a move, or for an unmovable or secure wadspace a restart);
  - the secrets (master, key, and the password for public);
  - the folder with `tokens.json` (and `rtc.json` for public);
  - the units, then Selkies' health, the route, and (private) `tailscale serve`;
  - then live, and `Event::Streams`.
- **Ends:** stopped, its wadspace stopped, opened on the screen, or idle `idle_minutes` with no viewers or activity.
  Teardown removes the route, the serve, the units, the volume (its unit stopped first), the secrets, the folder,
  and TURN credentials.
- **Adopted** after a wadd restart (`streams.json`, 0600, no secrets).
- **TURN:**
  - wadd mints credentials from `/etc/wados/turn.toml` (`[cloudflare.turn] token_id, api_token`) with Cloudflare's
    `generate-ice-servers`, and writes `{"iceServers": …}` without `:53` URLs to `rtc.json`.
  - **TTL:** 24 h, re-minted at three quarters of it, and revoked when the stream ends. Cloudflare disconnects a
    session shortly after its credentials expire, and Selkies' page reads its ICE servers only when it connects, so
    a short TTL would drop long sessions.
  - **Without it** (no file, or an error): `turnDown` and the warning "Cloudflare’s TURN didn’t answer: only viewers
    with a direct path can connect (no relay)"; the stream still starts.
  - The Cloudflare key never enters a container.
- **Another machine's streams:** `POST|DELETE /v1/peers/{id}/wadspaces/{ws}/stream` asks a peer's remote API for a
  private stream. It's kept here by (peer, ws), and opened in a WadBrowser `--app` window ("Stream from {machine}",
  Launch).
- **The API:**
  - socket: `POST|DELETE /v1/wadspaces/{id}/stream {mode, takeover}`, `GET /v1/streams` (with per-mode availability
    and its reasons), `GET /v1/streams/{id}/share` (the only place the key and password travel);
  - remote: the same, for my machines and trusted devices.

## Secure streams (limited): libkrun

- **Kata ruled out:** Kata Containers 2.x and later can't run under podman (its maintainers dropped the OCI CLI in
  September 2026).
- **libkrun instead:** a secure wadspace runs its own unchanged image as a libkrun micro-VM (`crun-krun`,
  `GlobalArgs=--runtime=krun`), from `wad_gen::units::secure_units`.
  - A Wayland socket can't cross into a VM, so Selkies and its compositor run **inside** it. Selkies comes from the
    stream image as an image volume, and runs from `/usr/lib/wados/secure` (`secure-init` and the guest service).
  - The VM gets 2 vCPUs and 2 GB by default; the VMM takes about 400 MB resident.
  - The VMM runs as `container_kvm_t`, with no SELinux denials.
- **Limited, for now:**
  - private (WebSocket) only, CPU encoding, and sound inside the VM;
  - offered only where the runtime and `/dev/kvm` exist;
  - refused for wadspaces with a folder mounted from the machine (`SecurityLabelDisable` would leave the VM
    unconfined);
  - public-secure says "Secure public streams come later".
- **Why it's limited:** inbound UDP for WebRTC needs passt mode, and the guest sees no inotify for `rtc.json`
  rewrites, so TURN rotation wouldn't reach it. GPU is untested.
- **Behaviour under krun:**
  - **No `podman exec`:** readiness is Selkies' health on the published port.
  - **Stopping** is SIGINT with 30 s, since SIGTERM never reaches the guest.
  - **No moving:** a secure stream always starts the wadspace again.

## The UI

- **The stream sheet:** Private | Public link, and Secure when it can start. Buttons are disabled with the reason as
  their tooltip. The confirm reads "{name} is on this screen. Streaming it takes it off the screen.", then the
  stages.
- **The share sheet:**
  - a QR of the one-tap link, the mode pill, Link / User / Password (masked, with copy);
  - warnings, "Streaming[ from {peer}] · {n} watching · since {ago}";
  - Open viewer, Stop streaming.

  The share is fetched when it opens, dropped when it closes, and never logged.
- **Around the shell:** card badges (`LIVE · n`), the Stream submenu, the tray's Streams tile and flyout, the reveal
  bar's Stream, Launch on another machine's card, and toasts with Show link and Retry.
- **The key on screen and on disk:** WadBrowser's error page and its history never show or keep a stream's key
  (`shown_uri`).

## Found on the VMs

- **Selkies ignores the cookie on its sockets:** fixed in the proxy (above).
- **A stream's display volume sometimes outlived it.** `podman volume rm` ran while podman was still removing the
  container, so the remove failed quietly. Every removal now goes through one helper (`backend::drop_volume`):
  1. wait until the containers are gone;
  2. remove the unit files and reload;
  3. stop and `reset-failed` the volume's oneshot;
  4. `volume rm`, retried while it's in use.

  Orphans are tidied when wadd starts. Checked clean over repeated DELETEs, idle ends, failed starts and peer stops.
- **wadd never stopped on SIGTERM while an event stream was open** (the shell's, Settings', a peer's), because axum's
  graceful shutdown waits for every connection. It now gives requests under way 3 s (`SHUTDOWN_GRACE`), then closes
  the rest: a restart takes about 3 s.
- **What the stream reports:**
  - **The transport** follows Selkies' own `/api/status` (`current_mode`), since a viewer can switch a dual-mode
    stream to WebSockets.
  - **Stopping a stream** that took a wadspace off the screen as it ran puts it back where it was (the screen),
    without restarting it. An idle end, or the wadspace stopping, still stops it, so nothing appears on the
    machine's screen unattended.

## Testing

- **`tools/vm.sh stream-check`:**
  - one VM with a Cloudflare **quick tunnel** (`trycloudflare.com`, no DNS change) to wadd's proxy;
  - a public stream opened from the laptop's Chromium: the one-tap and password flows, wrong tokens and passwords,
    the expired page through Cloudflare, takeover moving the wadspace, adoption, and no secret in any log;
  - `--no-secrets` checks every mode refused with its reason, with nothing crashing.
- **`tools/vm.sh tailnet-check`:** private streams on the local Headscale, reached through an SSH forward (Headscale
  has no HTTPS certificates for `tailscale serve`), streams from another machine, and `tools/krun/check-secure.sh`.
- **Unit and integration tests:** the proxy against a fake sidecar (routing, sign-in, the key on the sockets, and
  no key in any log); the lifecycle on the fake backend (start, move, idle, the person at the machine, adoption,
  teardown, TURN against a mock, peers, secure).
- **`images/stream/try-stream.sh`:** the sidecar and a wadspace on the laptop, including a move both ways.
- **Real TURN isn't tested yet.** The user chose to skip it for now; the checks run with `--no-turn`. The relay
  path, and the phone test on mobile data (`tools/vm.sh stream-phone`), wait for the user's go-ahead to move the TURN
  pair into a test VM.

## Not yet

- **Real TURN:** the relay path and the phone test.
- **Private streams over the real tailnet's HTTPS** (`tailscale serve --https` with Tailscale's certificates).
- **Secure streams:** public, GPU, and the machine's clipboard.
- **GPU encoding:** H.264 falls back to x264 in software. Fedora's Intel media driver lacks the encoder (RPM Fusion's
  has it), and NVENC needs the driver's libraries in the image.
- **Phase 10:** players and gamepads.
- **An intermittent slow teardown:** twice in four tailnet runs, a private stream's sidecar unit and display volume
  were still there 7 s and 36 s after it ended. Most likely its wadspace's container was slow to stop: wadd waits up
  to 120 s for it before removing the volume. The check now waits 150 s and logs wadd's lines and the containers if
  anything stays; the last run was clean within 2 s.
- **The key on WadBrowser's command line:** the one-tap link is passed to the viewer window there, so other
  processes of the same user can read it from `/proc`.
