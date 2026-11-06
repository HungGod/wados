# 0008: the installer and first boot

Phase 9. A stick boots to WadSpaces Live with the installer open. It asks for the computer's name, the account, the
disk and its encryption, sets up the machine's public address, and installs. The machine's first sign-in opens Set up
WadSpaces.

## The stick

- **A stick is the host image installed on a stick,** as in v1, with no ISO. An install copies the stick's own running
  system to the disk (`bootc image copy-to-storage`), and the machine then follows `ghcr.io/hunggod/wados-host:latest`.
- **`host/stick-files`** is what makes an installed image a stick. `host/build.sh installer` and `tools/vm.sh stick` both
  use it:
  - `/var/lib/wados/installer/installer.toml` (root's) turns on `GET|POST /v1/install`;
  - `cloudflare.toml` beside it (0600) holds the tunnel and TURN sections, and nothing else from the secrets file;
  - a provision folder, so first boot makes the `live` person and `/etc/wados/live`.
- **WadSpaces Live** (`/etc/wados/live`, `WADOS_SESSION=live`): there's no lock, because `live` is a locked account
  with no password. The installer opens by itself, and the desktop has an Install tile.

## The install

- **The API:** `POST /v1/install` is an Install job. wadd has the last word on every rule (`wad_model::install_names`,
  the spike's), and checks the disk is the one chosen by its by-id link, size and serial.
- **`bootc install to-disk` isn't used.** Its only encryption is TPM, and the spike's disk password is typed at each
  start. So `install-to-disk` partitions the disk itself, as `to-disk` would (BIOS boot, EFI, a 1 GiB `/boot`, root),
  encrypts root with LUKS2, and calls `bootc install to-filesystem` with `rd.luks.name`. An unencrypted install takes
  the same path without the LUKS step, so there's one way to install.
- **The disk password** reaches the script as a 0600 file on tmpfs, and is deleted as soon as the disk has it. The
  account's password is hashed by wadd (`openssl passwd -6`, on stdin), and only the hash reaches the disk.
  `InstallRequest`'s `Debug` shows neither.
- **It runs as a unit of its own** (`wados-install`), since wadd's sandbox can't write disks. wadd follows its log:
  `STEP n/N` lines are the job's stages.
- **The boot's password prompt is the plain console's.** The image has no plymouth, and the kernel's messages are
  quietened (`quiet`, `loglevel=3`) so the prompt stands alone.
- **Retry runs the install again** rather than carrying on from the failed step, as the spike's text says. Its
  Cloudflare calls are safe to repeat.

## The public address

- **Cloudflare is called before the disk is touched,** so a name that's taken leaves the disk as it was.
- **`crates/wad-cloudflare`:** the zone, then the tunnel `wadspaces-<id>` (an idle one is reused, and a connected one
  refused), its ingress for the one host, and one proxied CNAME, `<id>.wadspaces.com`.
- **`<id>` is the computer's name.** `www`, `api`, `mail`, `docs`, `status`, `app` and `admin` are refused.
- **What reaches the machine:** its tunnel's token and the TURN key. The DNS token and the tunnel creator's token
  never leave the stick; v1 kept the DNS token on machines, for its per-stream records.
- **`wadd tunnel remove <id> --secrets FILE`** retires a machine, from the build computer.

## First boot

`wados-provision.service` runs `host/usr/libexec/wados/provision` before the network, wadd and greetd. It puts in place
the hostname, the account (uid 1000, in wheel and wados), `wadd.toml`, the tunnel's token, `turn.toml`, the stick's
Wi-Fi networks and `setup.json`. Each file goes only where none is yet and is deleted once placed; what fails stays,
and is tried again at the next start.

## Set up WadSpaces

- **`wados-settings --setup`,** as the spike's: Welcome, Wi-Fi, Accounts, WadOS, Done. Wi-Fi and Accounts are Settings'
  own panels.
- **`<state>/setup.json`** holds the step. The shell opens Set up at each sign-in until it's done. With no file there's
  nothing to set up, so a machine the installer didn't make never sees it.
- **WadOS is offered, not seeded.** A library normally starts with the WadOS wadspace; a machine the installer made
  starts empty, and Set up's Install adds it.
- **The base images start downloading** when Set up ends, either way. They aren't on ghcr until CI publishes them, so
  until then that job fails and says so.

## Tailscale

Tailscale's client logs are no longer uploaded (`TS_NO_LOGS_NO_SUPPORT=true`, a tailscaled drop-in), the user's choice.
The cost is Tailscale's support for these machines.

## Testing

- **`tools/vm.sh installer-check`** runs the stick against a blank NVMe disk, through wadd's API: encrypted, `--plain`,
  and `--no-secrets`. The disk then boots alone from its own firmware entry.
- **Cloudflare is always a mock** (`tools/vm/cloudflare-mock.py`, and wiremock in the unit tests), with fake keys.
  Nothing in the tests reaches Cloudflare or touches DNS. The first real run of provision is the user's first stick
  with the real secrets.

## Found on the VMs

- **`bootc install to-filesystem` works with LUKS on root** with `rd.luks.name`, a separate `/boot`, and no TPM. It
  leaves the filesystems read-only, so the script mounts root again to write the provision folder.
- **The first window lost the keyboard.** The installer opened before the desktop's surface mapped, which then took
  the keyboard, so Enter activated the desktop's tile. Live now opens it 2 s after the shell starts.
- **Enter on Welcome pressed Cancel,** the first focusable button. Each page now says where the keyboard starts.
- **Enter never reached a `gtk::Text`'s `activate` here,** by real keys or `wtype`. The installer catches it on the
  window instead. Settings' Wi-Fi and password fields use the same `activate` and may have the same problem.

## Not yet

- **Steam in Set up:** phase 10.
- **A designed disk password screen:** it's a plain console prompt for now.
- **Drops on Live's desktop** aren't blocked with the spike's "Install WadSpaces first".
- **The spike's boot menu** (Test this media, Boot first hard disk) isn't there; GRUB's is.
- **Real hardware:** the Surface Book 2's keyboard at the disk password prompt, firmware boot entries, and a real stick.
