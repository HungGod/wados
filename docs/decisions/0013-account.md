# 0013: the WadSpaces account

After phase 13. Linking machines by hand over the tailnet worked, but it was tedious: the phone had to be told a
machine's name, and every device had to be trusted on a machine's screen. Projects' secret files (`.env`, keys) had
no place to live either. v1's Firebase account comes back for both. The tailnet is still the only way between
machines and the phone: the account is a directory and a vault, never a relay.

## What the account is

v1's Firebase project (`wadspaces-client/`: its rules, the `enrollMachine` function, Hosting's sign-in page),
unchanged. A person is a Firebase user signed in with GitHub. Their account has:

- `users/<uid>/machines/<id>`: one per linked machine. The machine writes its own `hostname`, `daemonVersion`,
  `lastSeen` and `tailnet` (`{wadd, name, dnsName, ip, kind, port}`). The rules let it change nothing else.
- `users/<uid>/secrets/f-<12 hex>`: the secret files (`value`, `encoding`, `file: {repo, path}`, `updatedAt`).

## Linking a machine

The machine links itself once it's signed in to GitHub (`apps/wadd/src/account.rs`). Nothing is typed:

1. It signs in to Firebase as the person, with its GitHub token (`signInWithIdp`, provider `github.com`).
2. As the person, it creates `enrollCodes/<code>` (10 characters, A–Z 0–9: what the rules take).
3. It calls `enrollMachine` with the code. That makes `users/<uid>/machines/<id>` and returns a custom token for
   the machine.
4. It signs in with that token. From then on it's the machine, never the person: the rules give it its own
   document and the account's secrets.

What's kept:
- The machine's refresh token, as the podman secret `wados_account`.
- `<state>/account.json` (0600): the uid, machine id, GitHub login, when it was linked. No token, ever.
- The person's Firebase tokens are dropped once the machine has its own.

**One sign-in.** WadSpaces' only way in is GitHub, so WadOS has no GitHub sign-in of its own any more: Settings ▸
Accounts (and Set up WadSpaces' step) has one **WadSpaces** group, with **Sign in to WadSpaces**. It's GitHub's
device flow (the browser's window says "Sign in to WadSpaces"), and its GitHub credentials do both jobs: clones and
pushes, and the link above. Signed in, the row says who and how the link is ("Your account heard from it…", "Not
linked to your account" with **Link**). **Sign out** (`DELETE /v1/github`) forgets the GitHub sign-in and unlinks
the machine with it. A WadOS with no account (no `firebase.toml`) keeps the same group, for the GitHub credentials
alone. Messages that asked for GitHub ("Sign in to GitHub first") ask for WadSpaces now.

The link also happens with `POST /v1/account/link` (the group's Link), and stops with `DELETE /v1/account`. A machine taken out of the account (its document deleted) notices at its next heartbeat. Its update is
`exists`-preconditioned. It then forgets the link and says so.

**The heartbeat** is every 60 s (`[account] heartbeat_s`): an update of the machine's document. Its mask is only the fields sent; `lastSeen` is
a server timestamp. The phone counts a machine as online when it was heard from in the last 3 minutes.

**The web app's config** is `/etc/wados/firebase.toml` (`api_key`, `project_id`, `auth_domain`,
`functions_region`; wad-config's `[account] firebase`). **It's never in the image**, which is public: the image is
the same for everyone and has no Firebase project in it. It goes the way the Cloudflare keys do:
- `host/build.sh installer` reads the secrets folder's `firebase.toml`, and `host/stick-files` writes only those
  four keys to the stick's `/var/lib/wados/installer/firebase.toml` (0600).
- Each install puts it in the machine's `provision/` (`write_provision`), and first boot
  (`host/usr/libexec/wados/provision`) installs it at `/etc/wados/firebase.toml`, which bootc's updates keep.
- A machine installed before, or by hand: copy the file there (`sudo install -m 0644 firebase.toml
  /etc/wados/firebase.toml`) and restart wadd.

With no file, or an empty one, the account is `off`: Settings hides it, and linking says why.

## The phone

`../wadspaces-android` signs in to the account with GitHub (Hosting's sign-in page in a WebView, then Firebase Auth).
It reads `users/<uid>/machines` and calls each machine at its tailnet address. Every call carries the person's ID
token (`Authorization: Bearer`).

wadd believes a token only when Firebase says it's the owner's (`accounts:lookup`, its uid that of the link; the
answer is cached by the token's SHA-256 for 5 minutes). It then treats the caller as trusted, with no ask
(`TailnetPeers::identify_owner`). Even so:
- the call has to come over the tailnet, from a device that isn't this machine;
- a device denied here stays denied;
- routes for my machines only (`Need::Machine`) never take a token;
- with no token, or someone else's, the phone is any other device: it's asked about, as in 0005.

A machine outside the account is no longer added by name: the account lists them all, and a machine that trusts the phone tells it the others (`/v1/remote/peers`).

## Secret files

v1's project files, ported to WadOS's clones (`apps/wadd/src/secret_files.rs`). A file is kept by its GitHub repo
and its path in the project. Its name is `f-` and the first 12 hex digits of SHA-256(`lower(repo)` `\n` `path`), so
every machine and the phone agree on it. A file is at most 64 KiB.

- Kept files are podman secrets on each machine. Linked, they're the account's: every machine fetches them (every
  10 minutes, `[account] sync_s`, and at once after a change). Before linking they're this machine's (`origin: local`) and go up when it
  links.
- They're placed into every clone of their repo: after a project is cloned or pulled (`projects.rs`), and every
  60 s. The writes run as the projects' user (uid 1000, an empty environment), never as root into a clone. They go
  through a temporary file and a rename, and a symbolic link at the path is never written through.
- **A file changed in a clone is never written over.** Each clone has a book (`<state>/secret-files.json`) of what
  was put there. A clone whose file isn't what was put there gets an ask: keep this version for every machine, or
  for this clone only.
- A file that git ignores and that's named like a secret (`.env`, `.env.*` but not `.env.example` and the like, `*.pem`,
  `*.key`, `.npmrc`, `*.local.toml`, `credentials*.json`, anything with "secret" in its name…, at most three
  folders deep) and isn't kept
  also gets an ask: keep it, or leave it be (not asked again).

Routes: `GET/POST /v1/secret-files`, `DELETE /v1/secret-files/{name}`, `POST /v1/secret-files/asks/{id}` on the
socket, and the same under `/v1/remote/secret-files` for trusted callers (the owner's phone). No answer ever has a
file's contents. Settings ▸ Accounts lists them, with Remove, and the asks with their answers. The phone keeps,
removes, and answers.

## Testing

None of these use the real Firebase, GitHub or tailnet.
- `tests/account.rs`: a whole wadd links against a mock Firebase. It checks that the enroll code is the one
  `enrollMachine` gets, that only the refresh token is kept, the heartbeat's mask and precondition, the secret
  files fetched, whose ID tokens are believed, and unlinking.
- `the_owners_phone_signed_in_is_trusted_without_asking` (tests/tailnet.rs): on the fake tailnet. The owner's
  token is trusted with no ask; someone else's, a forged one, or none isn't; a denied device stays denied; secret
  files are kept and removed over the remote API. With `WADOS_PHONE_FIXTURES` it records `secret-file-kept.json`
  and `secret-files-kept.json` for the app.
- `secret_files.rs`' tests: a real git clone and the real `sh`. A kept file goes in 0600; a change in the clone is
  asked about and never overwritten; a link isn't written through.
