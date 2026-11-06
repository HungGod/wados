//! The bar's look (`shell/bar/*.svelte`'s styles): the halves' glass, the
//! strips between them, a bar's items, the tray, its flyouts and menus.
//! GTK has no backdrop blur, so the glass is a little less see-through.

use std::cell::Cell;

/// The glass the halves, the strips and the extra displays' bars share.
pub const CSS: &str = r#"
window.wados-display-bar { background: transparent; }

/* ---- the halves, the strips between them */
.wb-half, .wb-strip, .wb-glass {
  background-color: rgb(14 9 25 / 0.84);
  border: 1px solid var(--line-strong);
  border-radius: 16px;
  box-shadow: 0 16px 40px -14px rgb(0 0 0 / 0.75);
}
.wb-half { padding: 0 8px; min-height: 60px; }
.wb-strip { padding: 0 6px; min-height: 60px; }
/* Together on the launcher, one dock: the halves meet in the middle. */
.wb-half.wb-dock { border-radius: 20px; }
.wb-left.wb-dock { border-radius: 20px 0 0 20px; border-right-width: 0; padding-right: 6px; }
.wb-right.wb-dock { border-radius: 0 20px 20px 0; border-left-color: var(--line); padding-left: 6px; }
.wb-sep { min-width: 1px; min-height: 28px; margin: 0 4px; background-color: var(--line-strong); }
.wb-rule { min-width: 1px; min-height: 30px; margin: 0 4px; background-color: var(--line-strong); }
.wb-none { padding: 0 12px; font-size: 12px; color: var(--faint); }

/* Every button of ours starts bare. */
button.wb-tb, button.wb-start, button.wb-item, button.wb-task, button.wb-row, button.wb-menu-item,
button.wb-x, button.wb-tile, button.wb-act, button.wb-quiet, button.wb-pick, button.wb-res,
button.wb-fs, button.wb-ghost, button.wb-day {
  background: none;
  border: none;
  box-shadow: none;
  outline: none;
  text-shadow: none;
  color: var(--fg);
  min-height: 0;
  min-width: 0;
  padding: 0;
  font-weight: inherit;
}
button.wb-start { min-height: 50px; padding: 0 10px; border-radius: 13px; }
button.wb-start:hover, button.wb-start.wb-on { background-color: var(--surface-3); }
button.wb-tb { min-height: 40px; min-width: 20px; padding: 0 8px; border-radius: 11px; }
.wb-right button.wb-tb { min-height: 36px; min-width: 18px; border-radius: 10px; }
button.wb-tb:hover, button.wb-tb.wb-on { background-color: var(--surface-3); }
button.wb-tb:disabled { color: var(--faint); }
.wb-badge { min-width: 7px; min-height: 7px; border-radius: 99px; background-color: var(--accent); margin: 5px 3px 0 0; }

/* The dock's tasks (a button per window). */
button.wb-task { min-width: 42px; min-height: 42px; border-radius: 12px; }
button.wb-task:hover { background-color: var(--surface-2); }
.wb-task-line { min-width: 4px; min-height: 4px; border-radius: 2px; background-color: var(--muted); margin-bottom: 1px; }
.wb-task.wb-active .wb-task-line { min-width: 14px; background-color: var(--accent); }
.wb-task.wb-dim { opacity: 0.6; }

/* ---- one thing in a bar: its icon, its name under it */
button.wb-item { min-width: 68px; min-height: 54px; padding: 2px 3px 0; border-radius: 12px; color: var(--muted); }
button.wb-item:hover { background-color: rgb(255 255 255 / 0.06); color: var(--fg); }
button.wb-item.wb-active { background-color: rgb(255 255 255 / 0.1); box-shadow: inset 0 0 0 1px rgb(255 255 255 / 0.1); color: var(--fg); }
button.wb-item.wb-dim { opacity: 0.55; }
/* Something held over it that it would take. */
button.wb-item.wb-hot {
  background-color: color-mix(in srgb, var(--accent) 16%, transparent);
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent) 50%, transparent);
}
button.wb-item.wb-compact { min-width: 40px; min-height: 38px; padding: 0; border-radius: 10px; }
.wb-item-name { font-size: 11px; font-weight: 500; }
.wb-glyph { min-width: 32px; min-height: 32px; border-radius: 9px; background-color: var(--surface-3); color: var(--fg); }
.wb-glyph.wb-small { min-width: 26px; min-height: 26px; border-radius: 8px; }
.wb-glyph.wb-own { background: none; }
/* Open: a line under it (a dot per window, with more than one); lit in front. */
.wb-line { min-width: 5px; min-height: 3px; border-radius: 2px; background-color: var(--muted); }
.wb-line.wb-only { min-width: 16px; }
.wb-lines.wb-lit .wb-line { background-color: var(--accent); }
.wb-compact-line { min-width: 14px; min-height: 3px; border-radius: 2px; background-color: var(--accent); }
/* On another display: its number. */
.wb-on-n { min-width: 16px; min-height: 16px; border-radius: 5px; background-color: var(--surface-3); border: 1px solid var(--line-strong); font-size: 10px; font-weight: 700; color: var(--muted); }

/* ---- the tray */
.wb-clock { padding: 0 10px; border-radius: 10px; min-height: 38px; }
.wb-clock:hover, .wb-clock.wb-on { background-color: var(--surface-3); }
.wb-clock-t { font-size: 13px; font-weight: 600; }
.wb-clock-d { font-size: 11px; color: var(--muted); }
.wb-count { min-width: 16px; min-height: 16px; padding: 0 4px; border-radius: 8px; background-color: var(--accent-2); color: #fff; font-size: 10.5px; font-weight: 700; margin: 1px -1px 0 0; }

/* ---- flyouts, menus: popovers of our own glass */
popover.wb-pop { background: none; padding: 0; box-shadow: none; }
popover.wb-pop > contents {
  background-color: rgb(20 13 34); /* opaque: no blur behind it here */
  border: 1px solid var(--line-strong);
  border-radius: 18px;
  padding: 14px;
  box-shadow: 0 24px 60px -16px var(--shadow);
  color: var(--fg);
}
popover.wb-menu > contents { border-radius: 14px; padding: 6px; background-color: rgb(27 19 45); }
popover.wb-peek > contents { border-radius: 14px; padding: 8px; background-color: rgb(14 9 25); }
popover.wb-start-pop > contents { border-radius: 20px; padding: 12px; background-color: rgb(14 9 25); }

.wb-fly-head { padding: 2px 4px 8px; }
.wb-h3 { font-family: 'Space Grotesk', Inter, sans-serif; font-size: 16px; font-weight: 600; }
.wb-h4 { margin: 10px 4px 4px; font-size: 11.5px; letter-spacing: 0.06em; font-weight: 600; color: var(--faint); }
.wb-foot { padding-top: 8px; margin-top: 4px; border-top: 1px solid var(--line); }
.wb-p { margin: 4px 6px 8px; font-size: 13px; color: var(--muted); }
.wb-small-t { font-size: 11.5px; color: var(--muted); }
.wb-row { min-height: 32px; padding: 6px 10px; border-radius: 11px; }
button.wb-row:hover { background-color: var(--surface-3); }
.wb-row.wb-on-row { background-color: var(--accent-soft); }
.wb-row.wb-on-row wad-icon { color: var(--accent); }
.wb-later { padding: 10px 12px; border-radius: 12px; background-color: var(--surface-2); border: 1px dashed var(--line-strong); font-size: 12.5px; color: var(--muted); }
button.wb-quiet { min-height: 30px; padding: 0 10px; border-radius: 9px; font-size: 12.5px; color: var(--muted); }
button.wb-quiet:hover { background-color: var(--surface-3); color: var(--fg); }
button.wb-quiet:disabled { color: var(--faint); }
button.wb-ghost { min-height: 26px; padding: 0 10px; border-radius: 8px; font-size: 12.5px; font-weight: 600; border: 1px solid var(--line-strong); }
button.wb-ghost:hover { background-color: var(--surface-3); }
button.wb-ghost.wb-danger { color: var(--danger); }
button.wb-x { min-width: 24px; min-height: 24px; border-radius: 6px; color: var(--faint); }
button.wb-x:hover { color: var(--fg); background-color: var(--surface-3); }

/* The jobs. */
.wb-job { padding: 10px; border-radius: 12px; background-color: var(--surface-2); }
.wb-job-t { font-weight: 600; font-size: 13px; }
.wb-err { color: var(--danger); }
progressbar.wb-progress > trough { min-height: 4px; border-radius: 2px; background-color: var(--surface-3); border: none; }
progressbar.wb-progress > trough > progress { min-height: 4px; min-width: 0; margin: 0; border-radius: 2px; background-image: none; background-color: var(--accent); border: none; box-shadow: none; }
/* The strips scroll sideways with no bar showing. */
scrolledwindow.wb-scroll > scrollbar, scrolledwindow.wb-scroll > scrollbar slider { min-width: 0; min-height: 0; margin: 0; padding: 0; border: none; }

/* The hidden icons. */
button.wb-tile { padding: 10px; border-radius: 12px; border: 1px solid var(--line); background-color: var(--surface-2); font-size: 13px; font-weight: 500; }
button.wb-tile:hover { border-color: var(--line-strong); background-color: var(--surface-3); }
button.wb-tile wad-icon { color: var(--accent); }
.wb-tile small, .wb-tile .wb-small-t { font-size: 11.5px; font-weight: 400; color: var(--muted); }

/* The notifications. */
.wb-ev { padding: 8px; border-radius: 12px; }
.wb-ev:hover { background-color: var(--surface-2); }
.wb-eic { min-width: 28px; min-height: 28px; border-radius: 9px; background-color: var(--surface-3); color: var(--muted); }
.wb-ok .wb-eic { color: var(--accent); }
.wb-warn .wb-eic { color: var(--warn); }
.wb-error .wb-eic { color: var(--danger); }
.wb-ev-title { font-size: 13px; font-weight: 600; }
.wb-ev-body { font-size: 12.5px; color: var(--muted); }
.wb-ev-time { font-size: 11px; color: var(--faint); }

/* The calendar. */
.wb-cal { padding: 6px; border-radius: 14px; background-color: var(--surface-2); }
.wb-cal-name { font-weight: 600; padding: 4px 0 8px; }
.wb-dow { font-size: 11px; color: var(--faint); }
.wb-day { min-height: 30px; min-width: 40px; border-radius: 8px; font-size: 13px; }
.wb-today { background-color: var(--accent); color: var(--accent-fg); font-weight: 700; }

/* A menu (right-click). */
button.wb-menu-item { min-height: 22px; min-width: 200px; padding: 6px 10px; border-radius: 9px; }
button.wb-menu-item:hover { background-color: var(--surface-3); }
button.wb-menu-item:disabled { color: var(--faint); }
button.wb-menu-item wad-icon { color: var(--muted); }
button.wb-menu-item.wb-danger, button.wb-menu-item.wb-danger wad-icon { color: var(--danger); }
.wb-menu-sep { min-height: 1px; margin: 5px 8px; background-color: var(--line); }

/* Hovering a machine app: its windows, like KDE's task tooltip. */
button.wb-pick { padding: 6px; border-radius: 10px; min-width: 156px; }
button.wb-pick:hover, button.wb-pick.wb-front { background-color: rgb(255 255 255 / 0.07); }
.wb-shot { min-height: 92px; border-radius: 8px; background-image: linear-gradient(160deg, var(--surface-3), var(--bg-2)); box-shadow: inset 0 0 0 1px var(--line); color: var(--muted); }
.wb-pick-title { font-size: 12px; font-weight: 600; }
.wb-min-tag { padding: 1px 6px; border-radius: 99px; background-color: rgb(10 6 20 / 0.7); font-size: 10px; color: var(--muted); margin: 6px; }
button.wb-close { min-width: 20px; min-height: 20px; border-radius: 6px; color: var(--muted); background: none; border: none; box-shadow: none; padding: 0; margin: 0 2px 2px 0; }
button.wb-close:hover { background-color: var(--danger); color: #fff; }

/* The logo's menu. */
entry.wb-search, .wb-search {
  min-height: 36px;
  padding: 0 12px;
  border-radius: 11px;
  background-color: var(--bg-2);
  border: 1px solid var(--line-strong);
  box-shadow: none;
  outline: none;
  color: var(--fg);
}
button.wb-res { padding: 8px 2px 6px; border-radius: 12px; min-width: 0; }
button.wb-res:hover { background-color: var(--surface-3); }
.wb-res-name { font-size: 12px; color: var(--fg); }
.wb-ai { min-width: 40px; min-height: 40px; border-radius: 12px; background-color: var(--surface-3); }
.wb-avatar { min-width: 28px; min-height: 28px; border-radius: 99px; background-color: var(--accent-2-soft); color: var(--accent-2); font-weight: 700; font-size: 13px; }
.wb-me { font-size: 13px; font-weight: 500; }
button.wb-tb.wb-pw { min-height: 32px; min-width: 32px; padding: 0; border-radius: 9px; }

/* ---- an extra display's bar */
.wb-glass { min-height: 60px; padding: 0 14px; border-radius: 14px; }
.wb-who { font-size: 13px; }
.wb-n { min-width: 22px; min-height: 22px; border-radius: 7px; background-color: var(--surface-3); font-size: 12px; font-weight: 700; color: var(--muted); }
.wb-glass.wb-apps { padding: 0 6px; }
.wb-time { font-size: 13px; font-weight: 600; }
"#;

thread_local! {
    static INSTALLED: Cell<bool> = const { Cell::new(false) };
}

/// Adds the bar's CSS to the display, once.
pub fn install() {
    if INSTALLED.with(|i| i.replace(true)) {
        return;
    }
    if let Some(d) = gtk::gdk::Display::default() {
        wad_gtk::theme::add_css(&d, CSS);
    }
}
