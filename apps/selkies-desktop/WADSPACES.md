# selkies-desktop, as WadSpaces builds it

The desktop icons, Start menu and taskbar inside every wadspace (the base
image runs it from labwc's autostart). Taken from
https://github.com/selkies-project/selkies-desktop at
0c4ed880c96ff4a9a3797d2f959c797b31b6ff06 (2026-10-13), under its licence,
the Mozilla Public License 2.0 (LICENSE): these files stay under it, changed
as below. `images/build.sh` builds it in the trixie builder with WadBrowser.

WadSpaces' changes (marked "WadSpaces" in the source):

- desktop.h: the panel is 56 px tall (was 30; the HUD's 28 px bar sits in
  its middle); the Start button 60 px wide (was 80) with a 42 px logo (was
  16); the taskbar's window icons 24 px (TASKBAR_ICON_SIZE; the menus keep
  ICON_SIZE, 16).
- ui.c: no logo in the middle of the desktop; the taskbar's icons and text
  centred in the taller panel, the text 13 px.
- backend-x11.c: X11 windows' taskbar icons at the new size.
