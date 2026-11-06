# 0007: system apps

Phase 8. Settings gets every panel, Files becomes a real file manager, and wadd takes on the machine's network,
Bluetooth, updates and storage. The rule that divides the work: what needs root or belongs to the machine is wadd's,
and what belongs to the person's session stays in the session.

## Who owns what

- **wadd (root, the machine's):** Wi-Fi, Ethernet, airplane mode, the exit node, Bluetooth, updates, storage.
- **The session (no root):** displays, night light, sound, mouse, touchpad, keyboard, idle times, the lid, the lock
  screen and passwords (`crates/wad-desk`, shared by Settings, which writes them, and the shell, which follows them).

## Settings

- **A panel registry.** A panel is one module in `apps/wados-settings/src/panels` named by `panels::REGISTRY`. It's
  made when Settings starts, so it hears wadd's events from the first, and its page is built the first time it opens.
  `nav.rs` holds the sidebar, the search words, and the other names the shell or a notification may open a panel by.
- **Shared widgets:** `crates/wad-gtk/src/form.rs`, including `display_trial`.
- **Displays:** labwc's output management, on a Wayland connection of Settings' own.
  - **Every change is tried for 15 s.** Keep Changes saves the layout for this set of displays, and anything else puts
    them back. The shell restores a saved layout each time that set is plugged in.
  - **A move goes in two steps.** labwc fixes a display only when it's put somewhere other than where it is; one left
    in place stays in wlroots' automatic layout and gets moved. So a configuration that moves anything first puts every
    display a pixel to the right of its target, then on it.
  - **Night light** is held by the shell, because labwc gives gamma back when the connection that set it goes.
- **Power:** UPower, power-profiles-daemon, and logind's lid switch. The shell holds the lid switch for Lock the screen
  and Do nothing, so no root is needed. Server Mode is shown off until phase 10.
- **Mouse and keyboard:** the person's `~/.config/labwc/rc.xml` and `environment`, then `labwc --reconfigure`. The
  session now starts `labwc -m`, so the person's file merges over the machine's and the machine's keybinds stay.
- **Passwords:** PAM checks the current one. The change goes through `passwd` on a pseudo-terminal of our own, as
  GNOME Settings does, because only root writes `/etc/shadow`.
- **The session's settings** live in `$XDG_CONFIG_HOME/wados/settings.toml` (written whole and atomically; the shell
  watches its folder) and `displays.toml`.

## wadd

- **The API** is in `apps/wadd/src/api_phase8.rs`: `/v1/network`, `/v1/bluetooth`, `/v1/update`, `/v1/storage`.
  `fake_phase8.rs` backs tests and `wadd serve --sample`.
- **Network** (`crates/wad-net`, NetworkManager over D-Bus): polled every few seconds, told as the `network` event.
- **Airplane mode** is Wi-Fi and Bluetooth off, kept in `airplane.json` across restarts. Turning it off brings Wi-Fi
  back, and Bluetooth only if it was on before.
- **Bluetooth** (`crates/wad-bt`, BlueZ over D-Bus): pairing answers at once, and what BlueZ asks goes out as the
  `btPrompt` event. Looking for devices stops by itself unless it's asked again. Turning it on with no controller is
  refused with "This machine has no Bluetooth".
- **Updates** (`updates.rs`):
  - **bootc's timer stays off,** because it would reboot. wadd checks every 12 hours, downloads what it finds and
    stages it. **Restarting is always the person's.**
  - **Cancelling** undeploys a staged update and remembers its digest as turned down, so the next check doesn't fetch
    it again. Download on the page still does.
  - **Rollback** queues the deployment before.
  - **How it runs:** wadd.service is sandboxed and bootc writes `/sysroot`, so wadd runs it as a unit of its own with
    `systemd-run --wait`.
  - **A private image:** a refused check is retried once with the GitHub token written to `/run/ostree/auth.json`
    (0600, ghcr.io only, in `/run` only).
  - **Base images** are refreshed from ghcr. A wadspace built on an older base shows an update and rebuilds when it
    next opens.
- **Storage** (`storage.rs`): what the disk holds by base images, wadspaces and other, measured at most every 30 s.
  Clean Up removes only what belongs to nothing; set images of wadspaces in the Trash stay, since they can come back.
- **Notifications:** `Event::Notice` gained a body, icon, action and toast. A notice's button is a verb
  (`apps/wados-shell/src/verbs.rs`), and the lock screen shows the unread ones unless Settings turns that off.
- **Deleting for good:** `DELETE /v1/wadspaces/{id}` purges a wadspace, and `drop_volumes` removes its volumes
  (`wad-<id>-config`, `wadstream-<id>`, `wadsecure-<id>`).

## Files

- **`apps/wados-files`** has this machine's places, the Trash (files and wadspaces), wadspaces read-only, Machines, and
  text and image viewers.
- **Open with** (`wad_model::open_with`, `wad_gtk::opened`) opens a file in a wadspace with an app for it, and
  remembers the choice in the session. With nothing remembered, a double-click uses the viewers.

## Found on the VMs

- **"This machine's person has no home":** wadd looked the home up once, at its start. It looks it up again when
  it's missing.
- **A purge left a volume behind:** fixed with `drop_volumes`.
- **Power mode was refused over SSH by polkit.** That's right, since only the person at the machine may change it. The
  check accepts a clear refusal, and the panel shows the error instead of hiding it.
- **`bluetoothctl` hangs with no controller.** It stalled `radios-check` for 45 minutes; the check now has time limits.

## Testing

- **`tools/vm.sh check`,** extended for the new API.
- **`tools/vm.sh update-check`:** a newer image in a local registry is found and staged, turned down, staged again,
  restarted into, and rolled back.
- **`tools/vm.sh radios-check`:** Ethernet and Bluetooth on a VM's own wadd. A VM has no radios, so Bluetooth checks
  the refusal, not pairing. Its Wi-Fi step needs `mac80211_hwsim` and is skipped on kernels without it, as on this
  laptop's test VM.
- **`wadd` tests on the fakes** (`wad-net`, `wad-bt`, `fake_phase8.rs`).

## Not yet

- **Real hardware.** The plan's check for this phase is each panel on the Surface Book 2 at 200% and the desk's two
  monitors, plus a real Wi-Fi join, a wrong password, rfkill and Bluetooth pairing. It's the user's to run.
- **Server Mode:** phase 10.
