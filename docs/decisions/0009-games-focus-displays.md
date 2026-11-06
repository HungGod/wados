# 0009: focus, displays, server mode, Play together and games

Phase 10, except Steam, which isn't done (see "Not yet").

## Focus

- **wadd holds the rule** (`apps/wadd/src/focus.rs`), so the phone and other machines can't get around it. The checks
  are on the paths people use, the API and the remote API. wadd's own stops aren't checked.
- **One at a time, at most 12 hours.** Other full-screen wadspaces go to the background. Refused while it's on: opening
  another wadspace on the screen or in a window, moving or stopping the focused one, streaming it, and server mode.
  Streams of other wadspaces go on, since they aren't on this screen.
- **It's kept in `/run`,** so it outlasts wadd restarting and not the machine: restarting is the only way out early.
- **The shell** has the setup and countdown dialogs, "Focus · MM:SS left" in place of the bar's buttons, a cover on
  every other display, and drops Super+Tab, Alt+Space and Ctrl+Alt+Down in one place (`main.rs`'s `leaves`).

## Displays

- **Open on N, Show on N, Full screen on N and To N** go through `Shell::open_on`. `Placer` remembers a wadspace's
  display when it isn't the primary.
- **Unplugged:** what was on that display goes to the background and keeps running, with the spike's toasts. A focus
  stays, on the primary now.
- **Moving a window between displays** by dragging is labwc's own. The spike's "Move to N" button isn't there.
- **A display with no model** goes by its connector: wlroots says "Unknown".

## Server mode

- **`wadd console` on tty1** (`wados-console.service`, under a logind inhibitor for sleep, idle, the lid and the power
  key) replaces v1's keyboard proxy. Once the session ends, the text console gets keys and the kernel's Ctrl+Alt+Delete
  itself, so no input code is needed. `ctrl-alt-del.target` only asks the console for its menu.
- **The console and greetd don't conflict in systemd.** With `Conflicts=`, a stray `start greetd`, refused by its
  `ExecCondition`, still stopped the console and left no way back. wadd stops one and starts the other.
- **Coming back is detached from its request.** The console that asks is the first thing stopped, and its connection
  going cancelled the rest of the handler, leaving no session and no console. Found on the VM.

## Play together

- **A public stream of a wadspace with Dolphin or Steam,** 2 to 4 players, each with a link and key of their own.
- **Pads:** the sidecar builds Selkies' Input Interposer and fake-udev from the wheel's own tag, pinned by sha256, and
  puts them in the display volume. The wadspace starts with them preloaded, so its apps see four Xbox 360 pads with no
  kernel device. It always restarts for Play together, since a running app can't gain a preload.
- **Selkies keeps each player to their slot** by the token table: player 1 is `controller`, the others `viewer` with a
  slot. The proxy accepts each player's key and hands that same key on, so a player signed in by the cookie stays that
  player on Selkies' sockets.
- **The phone's UI** is phase 13's.

## Dolphin

- **Fedora's `dolphin-emu`,** which forces X11: wadspaces have Xwayland.
- **A disc stays on this machine** at `<state>/discs/<id>/`, which a wadspace with Dolphin sees read-only at
  `/config/Games`. It's pushed to `ghcr.io/<login>/wadspaces-disc-<id>` only if "Also keep it in your GitHub packages"
  is chosen. This is the plan's opt-in; the spike always pushes.
- **wadd is root, so it only takes files from the person's home,** after resolving links.
- **The push's login** is a 0600 file in wadd's state folder, removed after. It's never in the disc's folder, which is
  the build's context and is mounted into wadspaces.

## Testing

- **`tools/vm.sh modes-check`:** focus, a second display through labwc's virtual output, and server mode on the
  machine.
- **`images/stream/try-pads.sh`:** the real sidecar, a process reading `/dev/input/js0-3` through the interposer, and
  four players on Selkies' socket. Each reaches their own pad, and player 2 can't press player 1's.
- **Unit and integration tests** for focus, server mode, player keys and discs.

## Not yet

- **Steam isn't done:** its feature script, gamescope, the sign-in container, the library, the push dialog and Set up's
  Steam card. They depend on the real client and a real account, which the tests may not use, so a mock wouldn't prove
  them.
- **`GCPadNew.ini` is unverified.** Dolphin opened the four pads in a container, but this release build doesn't log
  device names, so the names and axis numbers come from Selkies' pad name and Dolphin's documented numbering. It needs
  a real game.
- **Not run end to end:** a whole Play together stream through wadd in a browser, and a Dolphin game in a wadspace
  (the `dolphin` package isn't published yet).
- **Pulling a disc on another machine** and copying one over Taildrive.
- **`write:packages`** comes from a second GitHub sign-in that nothing asks for yet, so a real push may be refused.
- **Selkies' touch gamepad** hasn't been seen on a phone.
