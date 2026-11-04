//! WadBrowser's styles, on wad-gtk's tokens: the v2 widgets'
//! ([`WIDGETS_CSS`]: menus, the shield, the profile chip, the dialogs) and
//! the window's ([`WINDOW_CSS`]: the tab bar, the header bar).

use gtk::gdk;

/// The v2 widgets, as the spike draws them (`Browser.svelte`,
/// `ui/MenuList.svelte`, `PrivacyPopover.svelte`, `ui/Dialog.svelte`,
/// `ui/Button.svelte`, `ui/TextField.svelte`, `ui/Toggle.svelte`).
pub const WIDGETS_CSS: &str = r#"
/* ------------------------------------------------------------ header bar buttons */

button.wb-hb {
  min-height: 32px;
  min-width: 16px;
  padding: 0 8px;
  border: none;
  border-radius: 9px;
  background: none;
  box-shadow: none;
  color: var(--fg);
}
button.wb-hb:hover { background-color: var(--surface-3); }
button.wb-hb:disabled { opacity: 0.32; filter: none; }
/* Its own icon (its popover is a CSS child of it too). */
button.wb-shield > box > wad-icon { color: var(--accent); }
button.wb-shield.off > box > wad-icon { color: var(--warn); }
.wb-shield-count { font-size: 12px; font-weight: 400; color: var(--muted); font-feature-settings: "tnum"; }

/* The profile chip (its colour: chip.rs adds a class per colour). */
button.wb-chip {
  min-height: 26px;
  padding: 0 11px 0 9px;
  border-radius: 999px;
  border: 1px solid alpha(#c6ff1f, 0.45);
  background-color: alpha(#c6ff1f, 0.14);
  box-shadow: none;
  color: var(--fg);
  font-size: 12px;
  font-weight: 400;
}
.wb-chip-dot { min-width: 9px; min-height: 9px; border-radius: 999px; background-color: #c6ff1f; }

/* ------------------------------------------------------------ menus */

popover.wb-menu,
popover.wb-pop { background: none; box-shadow: none; padding: 0; font: initial; }
popover.wb-menu > contents {
  min-width: 208px;
  padding: 6px;
  border-radius: 14px;
  border: 1px solid var(--line-strong);
  /* --surface-2 (opaque: there's no blur behind it here). */
  background-color: var(--surface-2);
  box-shadow: 0 18px 48px -12px var(--shadow);
  color: var(--fg);
  font-family: Inter, system-ui, sans-serif;
  font-size: 14px;
}
button.wb-menu-item {
  /* 34 px with its padding, as the spike's. */
  min-height: 22px;
  padding: 6px 10px;
  border: none;
  border-radius: 9px;
  background: none;
  box-shadow: none;
  outline: none;
  color: var(--fg);
  font-weight: 400;
}
button.wb-menu-item:hover,
button.wb-menu-item.open,
button.wb-menu-item:focus-visible { background-color: var(--surface-3); }
button.wb-menu-item.danger { color: var(--danger); }
button.wb-menu-item:disabled { color: var(--faint); background: none; filter: none; opacity: 1; }
button.wb-menu-item:disabled label { color: var(--faint); }
.wb-menu-ic { color: var(--muted); }
button.wb-menu-item.danger .wb-menu-ic { color: var(--danger); }
button.wb-menu-item:disabled .wb-menu-ic,
button.wb-menu-item:disabled .wb-menu-chev { color: var(--faint); }
.wb-menu-label { font-size: 14px; }
.wb-menu-hint { font-size: 11.5px; color: var(--faint); }
.wb-menu-kbd { font-size: 11.5px; color: var(--faint); margin-left: 12px; }
.wb-menu-sep { min-height: 1px; margin: 5px 8px; background-color: var(--line); }
.wb-menu-heading {
  padding: 8px 10px 4px;
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.66px;
  color: var(--faint);
}

/* ------------------------------------------------------------ the shield's popover */

popover.wb-pop > contents {
  padding: 12px;
  border-radius: 14px;
  border: 1px solid var(--line-strong);
  background-color: var(--surface-2);
  box-shadow: 0 18px 48px -12px var(--shadow);
  color: var(--fg);
  font-family: Inter, system-ui, sans-serif;
  font-size: 14px;
}
.wb-shield-badge { border-radius: 10px; background-color: var(--accent-soft); color: var(--accent); }
.wb-shield-badge.off { background-color: var(--warn-soft); color: var(--warn); }
.wb-pop-title { font-weight: 700; }
.wb-pop-small { font-size: 12px; color: var(--muted); }
.wb-shield-tally { padding: 10px 12px; border-radius: 10px; background-color: var(--surface); }
.wb-shield-n { font-family: 'Space Grotesk', Inter, sans-serif; font-weight: 600; font-size: 22px; }
.wb-pop-row { font-size: 13px; }
button.wb-pop-link {
  min-height: 34px;
  padding: 0 10px;
  border: none;
  border-radius: 9px;
  background: none;
  box-shadow: none;
  color: var(--muted);
  font-size: 13px;
  font-weight: 400;
}
button.wb-pop-link:hover { background-color: var(--surface-3); color: var(--fg); }

/* A switch, as ui/Toggle: 40 × 24, its knob 18. */
switch.wb-toggle {
  padding: 2px 1px;
  border: 1px solid var(--line-strong);
  border-radius: 999px;
  background-color: var(--surface-3);
  background-image: none;
  outline-offset: 2px;
}
switch.wb-toggle:checked { background-color: var(--accent); border-color: var(--accent); }
switch.wb-toggle > slider {
  min-width: 18px;
  min-height: 18px;
  margin: 0;
  border-radius: 999px;
  background-color: var(--fg);
  box-shadow: none;
}
switch.wb-toggle:checked > slider { background-color: var(--accent-fg); }
switch.wb-toggle image { color: transparent; }

/* ------------------------------------------------------------ dialogs */

dialog.wb-dialog floating-sheet > dimming { background-color: var(--dim); }
dialog.wb-dialog sheet {
  border-radius: 24px;
  border: 1px solid var(--line-strong);
  outline: none;
  background-color: alpha(#140d22, 0.97);
  box-shadow: 0 30px 80px -20px var(--shadow);
  color: var(--fg);
  font-family: Inter, system-ui, sans-serif;
  font-size: 14px;
}
.wb-dialog-header { padding: 20px 20px 0; }
.wb-dialog-badge { border-radius: 12px; }
.wb-dialog-badge.accent { background-color: var(--accent-soft); color: var(--accent); }
.wb-dialog-badge.magenta { background-color: var(--accent-2-soft); color: var(--accent-2); }
.wb-dialog-badge.warn { background-color: var(--warn-soft); color: var(--warn); }
.wb-dialog-badge.danger { background-color: var(--danger-soft); color: var(--danger); }
.wb-dialog-title {
  font-family: 'Space Grotesk', Inter, sans-serif;
  font-weight: 600;
  font-size: 18px;
  letter-spacing: -0.18px;
}
button.wb-dialog-x {
  min-width: 30px;
  min-height: 30px;
  padding: 0;
  border: none;
  border-radius: 8px;
  background: none;
  box-shadow: none;
  color: var(--muted);
}
button.wb-dialog-x:hover { background-color: var(--surface-2); color: var(--fg); }
.wb-dialog-body { padding: 16px 20px 20px; color: var(--muted); }
.wb-dialog-text { color: var(--muted); line-height: 1.45; }
.wb-dialog-footer { padding: 0 20px 20px; }

/* Buttons (ui/Button: md). */
button.wb-btn {
  min-height: 38px;
  padding: 0 16px;
  border-radius: 12px;
  border: 1px solid transparent;
  box-shadow: none;
  font-weight: 500;
  background-image: none;
}
button.wb-btn.primary {
  background-color: var(--accent);
  color: var(--accent-fg);
  font-weight: 600;
  box-shadow: 0 8px 24px -10px var(--accent);
}
button.wb-btn.primary:hover { box-shadow: 0 10px 28px -8px var(--accent); }
button.wb-btn.secondary { background-color: var(--surface-2); border-color: var(--line-strong); color: var(--fg); }
button.wb-btn.secondary:hover { background-color: var(--surface-3); }
button.wb-btn.ghost { background: none; color: var(--muted); }
button.wb-btn.ghost:hover { background-color: var(--surface-2); color: var(--fg); }
button.wb-btn.quiet { background: none; border-color: var(--line); color: var(--muted); }
button.wb-btn.quiet:hover { border-color: var(--line-strong); color: var(--fg); }
button.wb-btn.danger { background-color: var(--danger-soft); color: var(--danger); border-color: rgb(255 93 122 / 0.3); }
button.wb-btn:disabled { opacity: 0.4; box-shadow: none; filter: none; }
button.wb-btn:disabled label { color: inherit; }

/* Text fields (ui/TextField). */
.wb-field-label { font-size: 12px; font-weight: 500; color: var(--muted); }
entry.wb-field {
  min-height: 40px;
  padding: 0 14px;
  border-radius: 12px;
  border: 1px solid var(--line-strong);
  background-color: var(--bg-2);
  box-shadow: none;
  outline: none;
  color: var(--fg);
}
entry.wb-field:focus-within { border-color: var(--accent); box-shadow: 0 0 0 3px var(--accent-soft); }
entry.wb-field.mono { font-family: monospace; font-size: 13px; }
entry.wb-field > text > placeholder { color: var(--faint); }
entry.wb-field > text > selection { background-color: var(--accent-soft); color: var(--fg); }
.has-error entry.wb-field { border-color: var(--danger); }
.wb-field-error { font-size: 12px; color: var(--danger); }

/* A choice (Create app's profile), as the spike's select. */
dropdown.wb-select > button {
  min-height: 40px;
  padding: 0 12px;
  border-radius: 12px;
  border: 1px solid var(--line-strong);
  background-color: var(--bg-2);
  background-image: none;
  box-shadow: none;
  color: var(--fg);
  font-weight: 400;
}
dropdown.wb-select > button:focus-visible { border-color: var(--accent); }
dropdown.wb-select popover > contents {
  padding: 6px;
  border-radius: 14px;
  border: 1px solid var(--line-strong);
  background-color: var(--surface-2);
  color: var(--fg);
}

/* New profile's colours. */
button.wb-swatch {
  min-width: 24px;
  min-height: 24px;
  padding: 0;
  border-radius: 999px;
  border: 2px solid transparent;
  box-shadow: inset 0 0 0 2px var(--surface);
  background-image: none;
}
button.wb-swatch.used { opacity: 0.55; }
button.wb-swatch.on { border-color: var(--fg); opacity: 1; }

/* Saved sign-ins: a key and its sites. */
.wb-key {
  padding: 10px 12px;
  border-radius: 12px;
  border: 1px solid var(--line);
  background-color: var(--surface-2);
}
.wb-key-icon { color: var(--c-key); }
.wb-key-name { font-weight: 700; color: var(--fg); }
.wb-small-faint { font-size: 12px; color: var(--faint); }
.wb-site {
  padding: 3px 4px 3px 10px;
  border-radius: 999px;
  background-color: var(--surface-3);
  color: var(--fg);
  font-size: 12px;
}
.wb-site.here { box-shadow: inset 0 0 0 1px var(--c-key); }
button.wb-site-x {
  min-width: 18px;
  min-height: 18px;
  padding: 0;
  border: none;
  border-radius: 999px;
  background: none;
  box-shadow: none;
  color: var(--muted);
}
button.wb-site-x:hover { background-color: var(--danger-soft); color: var(--danger); }

/* The permission prompt's "Remember". */
checkbutton.wb-check { color: var(--fg); padding: 0; }
checkbutton.wb-check check { border-radius: 6px; }
checkbutton.wb-check check:checked { background-color: var(--accent); color: var(--accent-fg); }
"#;

/// The window, as the spike's `Browser.svelte` draws it: the tab bar, the
/// header bar (its address, an app's offsite pill), the loading line, the
/// window's own buttons, the find bar.
pub const WINDOW_CSS: &str = r#"
window.wb-window, .wb-page { background-color: var(--bg-2); }

/* ------------------------------------------------------------ the tab bar */

.wb-tabbar { min-height: 34px; padding: 6px 6px 0; background-color: var(--bg); }
.wb-tab {
  min-height: 32px;
  padding: 0 6px 0 10px;
  border-radius: 9px;
  color: var(--muted);
  font-size: 12.5px;
}
.wb-tab:hover { background-color: var(--surface); color: var(--fg); }
.wb-tab.active { background-color: var(--surface-2); color: var(--fg); box-shadow: inset 0 0 0 1px var(--line); }
button.wb-tab-x {
  min-width: 22px;
  min-height: 22px;
  padding: 0;
  border: none;
  border-radius: 6px;
  background: none;
  box-shadow: none;
  color: var(--faint);
  opacity: 0;
}
.wb-tab:hover button.wb-tab-x, .wb-tab.active button.wb-tab-x { opacity: 1; }
button.wb-tab-x:hover { background-color: var(--surface-3); color: var(--fg); }
.wb-fav-ic { color: var(--accent); }
button.wb-hb.wb-new-tab { margin-left: 2px; color: var(--muted); }

/* ------------------------------------------------------------ the header bar */

.wb-header { min-height: 45px; padding: 0 8px; background-color: var(--surface-2); border-bottom: 1px solid var(--line); }
window.wb-mode-app .wb-header, window.wb-mode-focus .wb-header { background-color: var(--surface); }
window.wb-private .wb-header, window.wb-private .wb-tabbar {
  background-color: color-mix(in srgb, var(--accent-2) 9%, var(--surface));
}
.wb-identity { padding: 0 6px; font-size: 13px; }
.wb-identity-name { font-weight: 700; }
.wb-page-title { font-weight: 600; font-size: 13px; }
.wb-offsite {
  min-height: 26px;
  padding: 0 10px;
  border-radius: 999px;
  background-color: var(--warn-soft);
  color: var(--warn);
  font-size: 12px;
}
button.wb-back-to {
  min-height: 26px;
  padding: 0 12px;
  border-radius: 8px;
  border: 1px solid var(--line-strong);
  background-color: var(--surface-2);
  background-image: none;
  box-shadow: none;
  color: var(--fg);
  font-size: 12px;
  font-weight: 400;
}
button.wb-back-to:hover { background-color: var(--surface-3); }

/* A GitHub sign-in's code, under the header bar. */
.wb-code { min-height: 44px; padding: 0 12px; background-color: var(--surface); border-bottom: 1px solid var(--line); }
.wb-code-label { color: var(--muted); font-size: 13px; }
.wb-code-value {
  padding: 2px 10px;
  border-radius: 8px;
  background-color: var(--accent-soft);
  color: var(--fg);
  font-family: monospace;
  font-size: 18px;
  font-weight: 700;
  letter-spacing: 2px;
}
button.wb-code-copy {
  min-height: 28px;
  padding: 0 12px;
  border-radius: 8px;
  border: 1px solid var(--line-strong);
  background-color: var(--surface-2);
  background-image: none;
  box-shadow: none;
  color: var(--fg);
  font-size: 12.5px;
}
button.wb-code-copy:hover { background-color: var(--surface-3); }
.wb-code-hint { color: var(--faint); font-size: 12px; }

/* The address: shown, then typed in. */
.wb-addr { border-radius: 10px; border: 1px solid var(--line); background-color: var(--bg-2); }
.wb-addr.editing { border-color: var(--accent); box-shadow: 0 0 0 3px var(--accent-soft); }
button.wb-addr-shown {
  min-height: 32px;
  padding: 0 12px;
  border: none;
  border-radius: 10px;
  background: none;
  box-shadow: none;
  color: var(--muted);
  font-size: 13px;
  font-weight: 400;
}
.wb-addr-host { color: var(--fg); }
.wb-addr-rest, .wb-addr-placeholder { color: var(--faint); }
entry.wb-addr-entry {
  min-height: 32px;
  padding: 0 12px;
  border: none;
  border-radius: 10px;
  background: none;
  box-shadow: none;
  outline: none;
  color: var(--fg);
  font-size: 13px;
}
entry.wb-addr-entry > text > selection { background-color: var(--accent-soft); color: var(--fg); }

/* The loading line, along the header bar's foot. */
progressbar.wb-loading > trough { min-height: 2px; border: none; border-radius: 0; background: none; box-shadow: none; }
progressbar.wb-loading > trough > progress {
  min-height: 2px;
  border: none;
  border-radius: 0;
  background-color: var(--accent);
  box-shadow: 0 0 8px var(--accent);
}

/* The window's own buttons (minimize, maximize, close). */
windowcontrols { margin-left: 6px; }
windowcontrols > button {
  min-width: 26px;
  min-height: 26px;
  padding: 0;
  margin: 0 1px;
  border: none;
  border-radius: 7px;
  background: none;
  box-shadow: none;
  color: var(--muted);
}
windowcontrols > button > image { background: none; box-shadow: none; padding: 0; }
windowcontrols > button:hover { background-color: var(--surface-3); color: var(--fg); }

/* Find in page. */
.find-bar { padding: 6px 10px; background-color: var(--surface); border-top: 1px solid var(--line); }
"#;

/// WadSpaces' look (wad-gtk's tokens, fonts and base) and the v2 widgets'
/// styles, on `display`.
pub fn install(display: &gdk::Display) {
    wad_gtk::theme::install(display);
    wad_gtk::theme::add_css(display, WIDGETS_CSS);
    wad_gtk::theme::add_css(display, &crate::dialogs::new_profile::css());
}

thread_local! {
    static INSTALLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// [`install`] and the window's styles, the first time a window's made.
pub(crate) fn install_once(display: &gdk::Display) {
    if INSTALLED.replace(true) {
        return;
    }
    install(display);
    wad_gtk::theme::add_css(display, WINDOW_CSS);
}
