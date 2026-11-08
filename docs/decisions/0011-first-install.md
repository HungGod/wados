# 0011: what the first real install changed

The first install from a real stick, on two monitors, found what the VMs couldn't.

## Every display has a desktop

A second display had a wallpaper that took no input and a bar of its own. The bar is gone, and each display now has:

- **A desktop** (`desktop::Desktop`, one per display). The primary's shows every icon that isn't on another. Another
  display's starts empty and keeps its own file, `~/.config/wados/desk.<connector>.json`. Unplugged, its icons are on
  the primary until it's back. Right-click, the box, selecting and drags work as on the primary.
- **A sheet** (TOP layer) with the launcher's picture, so the launcher page covers it too.

An empty desktop draws nothing, and GTK makes no frame for that, so its layer surface was never mapped and clicks went
to labwc's root menu. `.desk-other` is 1/255 opaque for that reason, as the bar's reserve is.

## The launcher and the desktop fade into each other

One spring fades them on every display, where they are: the launcher (and its dock) fades out, and the desktop, its
bar and the other displays' desktops fade in. The first version slid them a page's height, which didn't look right on
the machine (the user, after trying it); a fade has nothing to keep in step between surfaces.

Windows stay where they are: the launcher fades off them.

## Steam

The client never opened. `steamwebhelper` runs in Steam's runtime, a `bwrap` container, and `bwrap` needs to mount.
In a wadspace, `container_t` refused the mount of devpts, and with that allowed the kernel refused a new `/proc` while
parts of the wadspace's own were masked. It restarted every 10 seconds, with no window and no useful message.

A wadspace with Steam now has `SecurityLabelType=container_engine_t` and `Unmask=/proc/*`, and keeps its MCS categories.
No other wadspace changes. Seccomp didn't need changing.

Proton isn't a package: Steam downloads it. `wadspaces-steam` writes a first `config.vdf` that makes Proton Experimental
the default for every game, so Steam fetches it for the first game that needs it.

## NVIDIA

Set up's WadOS step offers the host image with NVIDIA's driver, on by default when wadd finds an NVIDIA card:
`bootc switch` to the `-nvidia` image beside the booted one, staged for the next start. Failing leaves the machine as it
was; Set up carries on.

## Smaller ones

- **The running strip's display number:** only for a wadspace full screen on another display. A window touching display
  2 showed a 2, which reads as a count.
- **The Builder:** opens in the middle of the display (labwc's `AutoPlace`), and its stacks stop at 420 px wide: a bigger
  window shows more of them.
- **Sleep** is a power item (logind's Suspend: the screen locks first, as for the lid), in the bar's power menu, the
  power key's and the desktop's right-click menu.
- **Lock** is Super+L (was Ctrl+Alt+L).
- **WadOS's RUN_4** read `--component rustfmt clippy`, which rustup takes as a toolchain named `clippy`: it never
  built. It's `--component rustfmt,clippy` now, and wadd puts the old line right in a saved WadOS as it loads.
- **WadBrowser:** its pages are at `wadbrowser://` (a saved `cactus://` home page moves over), it shows its own logo and
  name, and the mouse's side buttons go back and forward. The Cactus credit stays on About and Settings.
- **Set up** has a solid background (Settings' own CSS never reached it) and no Steam.
- **CI:** `images` follows WadBrowser's code, which is in the base, and `host` follows all of `apps/`.

## Not yet

- **NVIDIA and Secure Boot:** nothing enrols the image's MOK certificate. With Secure Boot on, its modules won't load.
- **NVIDIA's open modules** need Turing or newer: Pascal cards (GTX 10 series) need the proprietary ones.
- **The icons of a new wadspace dropped on another display** go to the primary.
- **A real Steam sign-in, a Proton game, and a real GPU in a Steam wadspace.**
