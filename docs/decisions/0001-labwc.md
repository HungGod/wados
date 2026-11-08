# 0001: labwc as the compositor, and what it needs from us

Phase 0's probes, on Fedora 44's labwc 0.9.6 (built on wlroots 0.19).
`tools/headless.sh` runs them, with `crates/wad-wayland/examples/{listen,toplevels,pointer}` and the shell
(`wadsctl wm …`). `tools/build-labwc.sh` builds the patched labwc they need.

## A wadspace's socket hides everything privileged (no patch)

A client of a security-context socket (`wad_wayland::security_context::listen`,
engine `io.wadspaces`) sees 31 of labwc's 51 globals.

- **Hidden from a wadspace:** `ext_data_control_manager_v1`, `zwlr_data_control_manager_v1`,
  `ext_foreign_toplevel_list_v1`, `zwlr_foreign_toplevel_manager_v1`, `ext_image_copy_capture_manager_v1`,
  `ext_output_image_capture_source_manager_v1`, `zwlr_screencopy_manager_v1`, `zwlr_export_dmabuf_manager_v1`,
  `zwlr_layer_shell_v1`, `ext_session_lock_manager_v1`, `ext_idle_notifier_v1`, `zwlr_output_manager_v1`,
  `zwlr_output_power_manager_v1`, `zwlr_gamma_control_manager_v1`, `zwp_virtual_keyboard_manager_v1`,
  `zwlr_virtual_pointer_manager_v1`, `zwp_input_method_manager_v2`, `ext_workspace_manager_v1`,
  `zcosmic_workspace_manager_v1`, `wp_security_context_manager_v1`.
- **What a wadspace still has:** what a desktop needs (`wl_compositor`, `xdg_wm_base`, `wl_seat`, `wl_shm`,
  `wl_data_device_manager`, `wp_viewporter`, `wp_fractional_scale`, `zwp_pointer_constraints_v1`,
  `zwp_relative_pointer_manager_v1`, `xdg_activation_v1`, `zwp_text_input_manager_v3`, …).

## Super+Tab can't be stolen (no patch)

labwc doesn't offer `zwp_keyboard_shortcuts_inhibit_manager_v1` to anyone, so a wadspace's nested labwc can't take
the host's keybinds. Super+Tab, Alt+Space and the rest always reach the host labwc, which runs `wadsctl`. v1's
evdev keyboard proxy (`wad-input`) isn't needed.

## Which window is which wadspace: **patch** (`host/labwc/0001-…`)

Stock labwc reports every wadspace's nested labwc window the same way: `app_id="labwc"`, `title="labwc - WL-1"`.
A wadspace can open more windows with any app_id or title, so the shell has no sound way to tell them apart.

- A window rule *can* match the security context (`<windowRule sandboxEngine="io.wadspaces" sandboxAppId="…">`),
  and its `Execute` action does fire. But it passes nothing about the window, so pairing the signal with a
  foreign-toplevel handle is a race that a misbehaving wadspace can win.
- **Decision:** carry `host/labwc/0001-foreign-toplevel-report-sandbox-app-id.patch` (22 lines). For a client with
  a security context, foreign-toplevel reports the context's app_id instead of the client's own. wadd sets it to
  `io.wadspaces.<wadspace id>`.
  - The shell treats a window as wadspace X only if its app_id is exactly `io.wadspaces.X`.
  - Host apps (trusted, unsandboxed) keep their own app ids (`io.wados.*`).
- **Verified:** the patched labwc reports a wadspace's nested labwc as `app_id="io.wadspaces.probe"`, and foot on
  the normal socket as `io.wados.Terminal`.
- It's generic (a Flatpak window would report its Flatpak id), so it can go upstream.
- `host/Containerfile` builds labwc 0.9.6 with it from Fedora's sources. For development, build it in a
  `fedora:44` container (`tools/build-labwc.sh`); `tools/headless.sh` uses it.

## Full screen on a chosen display: **patch** (`host/labwc/0002-…`)

foreign-toplevel's `set_fullscreen(output)` went full screen wherever the window was (labwc's code had a TODO).
The patch moves the window to that output first.

- **Verified:** a wadspace opened on HEADLESS-2 goes full screen on HEADLESS-1, and then back to HEADLESS-2
  ("Open on 2", "To 2").

## The desktop takes the keys when it comes into view: **patch** (`host/labwc/0003-…`, phase 2)

The desktop is a BOTTOM-layer surface with on-demand keys, and labwc gives those a click only (top and overlay
ones also get the keys when they map). So after coming down to the desktop by the keys, Delete, F2 or Ctrl+A did
nothing until a click, and the bar (TOP, on-demand) took the keys as it mapped. An exclusive desktop isn't an
answer: labwc won't let a window have the keys while an exclusive surface does, whatever its layer.

The patch: a mapped layer surface that turns on-demand on (from none) asks for the keys, and gets them unless
an exclusive surface has them. Only the host's clients have layer-shell, so a wadspace can't use it.

- **The shell:** the desktop turns its keys off, then on-demand again once that's committed (the frame after),
  when it lands, and when a bar flyout closes with no window in front. The bar and the extra displays' bars take
  no keys of their own: their flyouts are popovers that grab, which labwc gives the keys to.
- **Verified:** Page Down from the launcher, then Ctrl+A, selects every desktop icon; the start menu's search
  takes typing; Esc closes it and the desktop has the keys again.

## What else the probes showed (no patch)

- **Layering:** a full-screen window covers the TOP layer (the bar), and OVERLAY shows above it. So the
  reveal bar, Super+Tab, Alt+Space, toasts and focus covers are OVERLAY surfaces; the bar and the launcher's
  chrome are TOP.
- **The bar's room (found in phase 3):** the reserve keeps 78 px from windows and the desktop, but only once
  it's mapped, and a layer surface is mapped by its first frame. GTK draws no frame for a window with nothing
  to paint, so an empty, transparent reserve could stay unmapped; then the desktop and the Builder started
  under the bar. It now holds a strip the bar's height at 1/255 opacity: never seen, drawn on the first frame.
- **The Builder (phase 3):** an ordinary window of the shell's with its own app id (`io.wados.Builder`, set
  as it's realized and mapped, after GtkApplication's). `rc.xml` has a `builder` region (the right 74% of
  the usable area) and a window rule that snaps it there with no server-side title bar (GTK draws its own),
  and `<gap>10</gap>` so it floats as the spike's does.
- **foreign-toplevel** `activate`, `set_minimized`/`unset_minimized`, `unset_fullscreen` and `close` all work
  on wadspace windows and host windows alike. `output_enter`/`output_leave` say which display a window is on.
- **Keybinds:** labwc's `rc.xml` (`host/etc/xdg/labwc/rc.xml`) runs `wadsctl` for Super+Tab, Super+Shift+Tab,
  Alt+Space, Ctrl+Alt+↓ (a full-screen wadspace into a window: `wadsctl windowed`) and Super+L. A virtual keyboard (`wtype`, the same protocol the shell uses) triggers
  them too.
- **Moving a window between displays:** there's no foreign-toplevel request for it. The shell activates the
  window and types `W-C-A-S-F13` (or `F14`), which `rc.xml` binds to `MoveToOutput right` (`left`). Verified:
  foot moved from HEADLESS-2 to HEADLESS-1.
- **Drag and drop:** a drag from an ordinary window lands on the desktop's BOTTOM layer surface, in its
  coordinates (desktop y = screen y − the bar's 78 px). Driven by a virtual pointer. Phase 0 dragged a piece
  from a Builder stub; since phase 2 the probe drags a file from another app (`tools/drag-source.py`) and checks
  it's copied into the desktop's folder. The Builder's pieces come back in phase 3.
- **The lock screen:** gtk4-layer-shell 1.3's session lock (`wad_gtk::session_lock`, our FFI; its Rust crate
  has none) holds every monitor. PAM, called directly (`apps/wados-shell/src/pam.rs`, no bindgen), checks the
  user's own password without root and refuses a wrong one. The right password is checked in the VM (phase 4),
  with a test user.
- **The launcher ↔ desktop transition:** one spring drives the chrome (TOP) and the desktop (BOTTOM). The bar
  (TOP) takes over at the end with the same pixels. In a headless session, use `GSK_RENDERER=cairo` (there's no
  GPU; GTK's Vulkan renderer only warns and drops frames).

## Later

- **The nested window's title in window mode** ("labwc - WL-1", drawn by the host labwc's decorations):
  cosmetic, decided with window mode in phase 2. Either the shell draws a wadspace window's title bar
  (client-side), or the image's labwc names its window.
- **The title bar's theme:** labwc's default is light. `host/` ships a dark labwc theme in the spike's colours
  (phase 4).
