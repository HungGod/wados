# 0005: accounts, the tailnet, peers and projects

Phase 6. A WadOS machine signs in to GitHub and Tailscale. It knows your other machines, and lets devices you
trust reach its wadspaces. It copies a wadspace from another machine, and keeps folders, files and repositories
in a wadspace. There's no cloud: v1's Firebase account is gone. Everything between machines goes over the
tailnet, so the other machine has to be online.

## Who's calling

wadd asks tailscaled who each remote connection is: LocalAPI WhoIs, cached for 30 s
(`apps/wadd/src/tailnet.rs`).

- **This machine.** Refused as a remote caller. Its own processes reach its tailnet port through `lo`.
- **One of my machines.** The same Tailscale user as this machine's login, untagged, not shared in, and its node
  answers `GET :8087/v1/remote/hello` as wadd.
  - `peers.rs` probes that in the background and keeps the answer by node id (`<state>/tailnet-peers.json`).
  - Tags are checked first. Headscale gives every tagged node the same pseudo-user ("tagged-devices"), so
    comparing users alone would call two tagged nodes "mine".
  - It's trusted with no ask.
- **Any other device** (the phone, a laptop without WadOS, another person's device, a shared-in or tagged node).
  - It's trusted only when the trust book says so. Otherwise it gets `hello` only, and an ask is raised.
  - A denied device stays quiet.
  - Asks are in memory (at most 8). Later puts one off for the session, everywhere.
- **Hello never waits on another machine.** An unknown caller from the same login is probed in the background,
  so two wadds saying hello to each other can't deadlock.

**The trust book** is `<state>/tailnet-devices.json` (0600). Each device has its answer: trusted, denied or
untrusted. It's wiped when the Tailscale login or tailnet changes. **Every machine you own follows an answer:**
- An answer made on one machine is posted to my online machines (`POST /v1/remote/trust`).
- When one of my machines comes online, the two swap books and merge them. Per device the latest `at` wins; a tie
  goes to the larger `by`. `untrusted` counts as an answer, so an untrust can't be undone by a merge.

## The remote API

`apps/wadd/src/remote.rs`, on port 8087 of the tailnet addresses (`SO_BINDTODEVICE tailscale0`, rebound when they
change; 64 KB bodies).

| Who | Routes |
|---|---|
| Anyone | `hello`: `{wadd, machine, trust: machine/trusted/asked/denied}` |
| My machines and trusted devices | `machine`; `wadspaces`; `wadspaces/{id}`; `wadspaces/{id}/{build,open,stop}` (streams are phase 7) |
| My machines only | `GET` and `POST trust` |

## On the machine's socket

- **The tailnet:**
  - `/v1/tailnet`: state, devices, asks, and Taildrive's state.
  - `login` is a Tailnet job: `tailscale up`, its AuthURL in a sign-in window, then Running.
  - `logout`.
  - `devices/{id}` (trust, untrust) and `asks/{id}/{deny,later}`.
- **Peers:**
  - `/v1/peers`: my machines.
  - `/v1/peers/{id}/wadspaces`: proxied, or 503 when it's offline.
  - `/v1/peers/{id}/wadspaces/{ws}/copy`: Copy here, a job that applies `space.import`.
    The job's `params.peer` names the machine. The copy gets this
    machine and a fresh build, and its projects keep pointing at the peer, so its folders stay there and come over
    Taildrive.
- **GitHub:**
  - `/v1/github` and `/v1/github/repos`.
  - `signin` is a Github job with stages `open`, `signin` (held) and `save`.
  - `DELETE /v1/github` signs out.
- **Keys:** `/v1/keys/web`, for the machine's own browser.
- **Events:** `tailnet`, `peers`, `github`, `signIn` and `signInDone`. A sample wadd, `wadd serve --sample`,
  answers all of it with fixed data, for building the UI.

## Signing in

- **wadd can't open the window itself.** It runs as root and can't open a window in the person's session, so it
  sends `signIn {provider, url, code}`. The shell (`signin.rs`) runs `wadbrowser --signin <provider> --url …
  [--code …]`, `--done` on `signInDone`, and `--show` for "Show the browser".
- **GitHub** uses the OAuth device flow with v1's OAuth app (client ID in `host/usr/lib/wados/github.toml`; no
  client secret).
  - **Scopes:** `repo workflow read:org` (phase 10 adds `write:packages` with a second sign-in).
  - **The token** becomes the podman secret `github_token`, which runs, clones and registry pulls use. The account
    is kept in `<state>/github.json` without the token.
  - **The browser:** GitHub's device page is behind its own sign-in and its form isn't documented, so the sign-in
    window shows the code with Copy, as well as filling it in.
  - **Errors** say what to do. One is "Enable Device Flow" not ticked on the app.
  - **Signing out** forgets the token here. GitHub still lists WadOS until it's revoked there, since revoking
    needs the client secret.
  - **Only a mock GitHub has been used** (`crates/wad-github` and `apps/wadd/tests/github.rs` with wiremock;
    `tools/vm/github-mock.py` in the VMs). The first real sign-in needs "Enable Device Flow" ticked on the OAuth
    app.
- **Tailscale:**
  - **Logging in:** `tailscale up --json` gives the AuthURL. wadd follows tailscaled's IPN bus to Running.
    `[tailnet] login_server` points at another control server (the test's Headscale).
  - **MagicDNS** is left on (v1 turned it off), so your machines reach each other by name.

## Taildrive

- **Sharing:** once Running, wadd shares the person's home **as them**: `runuser -u <person> -- tailscale drive
  share home <home>`. tailscaled serves a share as the user who asked for it, and asking as root would serve it as
  root. tailscaled runs `sudo -n -u <person> tailscaled serve-taildrive`; that works with the image's stock sudoers
  and SELinux.
- **The address:** another machine's home is `dav://100.100.100.100:8080/<tailnet>/<hostname>/home`.
  - `<tailnet>` is Tailscale's `CurrentTailnet.Name`, percent-encoded. On Headscale that's the server URL's host,
    not the MagicDNS suffix.
  - `<hostname>` is the first label of the peer's MagicDNS name.
- **Browsing:** GIO needs `mount_enclosing_volume` before it can list one.
- **The policy:** a tailnet needs `drive:share` and `drive:access` for the machines, plus a
  `tailscale.com/cap/drive` grant. Without them sharing is refused (`drive: notAllowed`), and Settings ▸ Accounts
  explains the policy.

## Projects

`apps/wadd/src/projects.rs`, `copies.rs` and `taildrive.rs`.

- **Opening:** a wadspace with projects opens through a Launch job, one stage per project: "Cloning {repo}",
  "Mounting {as}", "Mounting {as} from {peer} over Taildrive", "Copying {as}".
- **When one can't be made** (a private repo without a sign-in, a peer that's offline), it's left out and said:
  "{as} is on {name}, which is offline. Folders come over Taildrive, so both machines need to be online."
- **GitHub:** wadd clones into `<projects_dir>/<ws>/<id>` as uid 1000, with the token. The clone outlives the
  wadspace's home, shows its progress, and is mounted in. The image's `wadspaces-projects --sync` fetches and
  fast-forwards it.
- **This machine's folders** are mounted. They have to be strictly inside the person's home, or `/run/media/<them>`;
  symlinks are resolved and hidden folders refused.
- **A file** is copied into `copies/<ws>/<id>` and arrives on the wadspace's desktop as that file.
- **Another machine's files.** A file is fetched over WebDAV. A folder is mounted with rclone (`rclone mount
  :webdav: --allow-other`) as a transient unit in the machine's own mount namespace, and released when the wadspace
  stops.
- **Cleanup:** wadd removes only what it made. A clone with unpushed work is set aside (`.kept-…`), never deleted.
- **Permissions:** `/var/lib/wados`, `keys/` and `projects/` are 0711, so uid 1000 reaches its own folders (git,
  and the machine's browser using a web key's cookies) without listing them.
  - Set in `tmpfiles.d`, `StateDirectoryMode`, and wadd's store, since any one of them alone would set it back.
- **Names** are any name the model allows ("My Notes"). Launchers escape them for `Exec=`
  (`images/base/try-projects.sh`).

## The UI

- **The shell:**
  - the machine picker (this machine and my online ones);
  - another machine's cards: Copy here, Focus asking to copy first, Launch dimmed until phase 7;
  - the trust dialog (Later, Deny, Trust it);
  - the Tailscale tray tile, with a pip for asks;
  - "Show the browser" on a held sign-in;
  - the Builder's "It lives on {peer}: copy it here to change it";
  - the projects menu;
  - Keep from the desktop.
- **`apps/wados-settings`** (`--page accounts`): GitHub, Tailscale, Taildrive, Asking to Be Trusted, Devices. Its
  other panels are placeholders until phase 8.
- **`apps/wados-files`** (`--machines`, `--machine <id>`): my machines, a machine's home over Taildrive, and its
  wadspaces with Copy here.
- **Shared:** both apps share `wad-gtk`'s `kit.rs`.
- **Ids are per machine** (every machine has the starter `wadspaces`, and a copy may keep its source's id). So
  the shell keeps another machine's wadspaces under `<peer>:<id>`, and never mixes them with its own.

## Testing

- **`tools/vm.sh tailnet-check`:** two WadOS VMs on a local **Headscale 0.29.7** (pinned by digest).
  - **The test tailnet:** TLS from a test CA, its embedded DERP, the Taildrive policy, and user-owned nodes for sam.
  - **The untrusted device:** a third node from another user: a userspace tailscaled on the laptop, with its own
    socket and state.
  - **Isolation:** nftables keeps each VM to QEMU's network and the tailnet, so traffic is relayed over DERP and
    nothing reaches the internet or Tailscale's servers.
  - **Never the real tailnet.**
  - **`--plain`** checks plain Tailscale and Taildrive alone.
- **Other checks:**
  - `apps/wadd/tests/{tailnet,github,projects,api_phase6}.rs`: two wadds with fake tailscaleds, a mock GitHub, and
    a local WebDAV server.
  - `crates/wad-tailscale`'s `fake` feature.
  - `images/base/try-projects.sh`.

## Not yet

- Launching another machine's wadspace (it streams from there: phase 7).
- A peer's folders aren't listed in the projects menu (its usual folders are offered).
- The machine's browser picks up keys at its first window, not when they change while it runs.
- A wadspace gets `github_token` when it opens, so open ones need reopening after the first sign-in.
- The NeedsMachineAuth state.
- IPv6 between peers.
- A real GitHub sign-in.
- **Tailscale uploads its logs by default.** The test VMs turn that off; whether WadOS machines should is open.
