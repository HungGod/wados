//! The lock screen (`shell/lock/LockScreen.svelte`): "Welcome Sam" (the
//! account's name, never typed) and its password, over the lock wallpaper
//! (Silk), and Power off in the corner (in the machine's own session; the
//! power key puts the keys on it). Every start lands here, and so does
//! Lock, idle and suspend (session.rs). A key typed with the focus elsewhere
//! goes to the password box; Esc clears it.
//!
//! It holds the screen with ext-session-lock (wad_gtk::session_lock): if the
//! shell dies while locked, the screen stays locked. While its lock is on,
//! gtk4-layer-shell gives every other surface made a stub that never shows
//! (its `lock-surface.c`: a popup there could break the protocol), so a
//! window of ours shown while locked (the launcher and desktop at start, a
//! toast) is shown again, with a real surface, once it's unlocked. The password is checked
//! by PAM (`/etc/pam.d/wados-lock`, `WADOS_PAM_SERVICE` to use another), off
//! the main thread; pam_unix checks a user's own password without root
//! (unix_chkpwd), and pam_faillock slows guessing.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::{gdk, glib, prelude::*};
use wad_gtk::session_lock::{self, SessionLock};

use crate::session::Power;

type Then = Box<dyn FnOnce(bool)>;

pub struct Lock {
    current: RefCell<Option<SessionLock>>,
    windows: RefCell<Vec<gtk::Window>>,
    /// The compositor said it's locked (every display shows the lock).
    shown: Cell<bool>,
    /// Waiting for that: true once it is, false if it couldn't lock.
    waiting: RefCell<Vec<Then>>,
    /// The corner's Power off (session.rs's).
    power_off: RefCell<Option<Rc<dyn Fn()>>>,
    power_buttons: RefCell<Vec<gtk::Button>>,
    /// From `lock()` until unlocked (or failed): gtk4-layer-shell's lock is on.
    active: Cell<bool>,
    /// The app's windows shown meanwhile (stubs, never seen): shown again after.
    unshown: RefCell<Vec<glib::WeakRef<gtk::Window>>>,
    /// The app's windows are watched for that (once).
    watching: Cell<bool>,
    /// What notifications it shows (the unread ones' icons and titles; none
    /// when Settings' Lock Screen Notifications is off): main.rs's.
    notes: RefCell<Option<Notes>>,
}

type Notes = Rc<dyn Fn() -> Vec<(&'static str, String)>>;

/// The signed-in user: their login and the name they're welcomed by.
fn me() -> (String, String) {
    // SAFETY: getpwuid's static result, read before any other call.
    unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if pw.is_null() {
            return ("".into(), "".into());
        }
        let login = std::ffi::CStr::from_ptr((*pw).pw_name).to_string_lossy().into_owned();
        let gecos = std::ffi::CStr::from_ptr((*pw).pw_gecos).to_string_lossy().into_owned();
        let full = gecos.split(',').next().unwrap_or("").trim().to_string();
        let first = full.split_whitespace().next().map(str::to_string).unwrap_or_else(|| login.clone());
        (login, first)
    }
}

/// Checks `password` for `login` with PAM: Ok, or what to say.
fn check(login: &str, password: &str) -> Result<(), String> {
    let service = std::env::var("WADOS_PAM_SERVICE").unwrap_or_else(|_| "wados-lock".into());
    crate::pam::check(&service, login, password).map_err(|e| {
        tracing::info!("PAM ({service}): {e}");
        "That's not the password".to_string()
    })
}

impl Lock {
    pub fn new() -> Rc<Self> {
        Rc::new(Self {
            current: RefCell::default(),
            windows: RefCell::default(),
            shown: Cell::new(false),
            waiting: RefCell::default(),
            power_off: RefCell::default(),
            power_buttons: RefCell::default(),
            active: Cell::new(false),
            unshown: RefCell::default(),
            watching: Cell::new(false),
            notes: RefCell::default(),
        })
    }

    /// Where its notifications come from (the spike's: the last three unread, at the top left).
    pub fn set_notes(&self, f: impl Fn() -> Vec<(&'static str, String)> + 'static) {
        *self.notes.borrow_mut() = Some(Rc::new(f));
    }

    fn notes_box(&self) -> Option<gtk::Box> {
        let f = self.notes.borrow().clone()?;
        let notes = f();
        if notes.is_empty() {
            return None;
        }
        let b = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .halign(gtk::Align::Start)
            .valign(gtk::Align::Start)
            .margin_top(24)
            .margin_start(24)
            .css_classes(["lock-notes"])
            .build();
        b.set_widget_name("lock-notes");
        for (icon, title) in notes.iter().rev().take(3).rev() {
            let icon: &'static str = icon;
            let n = gtk::Box::builder().spacing(8).css_classes(["lock-note"]).build();
            n.append(&wad_gtk::Icon::new(icon, 14));
            n.append(&gtk::Label::builder().label(title).xalign(0.0).build());
            b.append(&n);
        }
        Some(b)
    }

    /// Each of `app`'s windows (and every one to come) noted if it's shown
    /// while the lock is on (not the lock's own).
    fn watch(self: &Rc<Self>, app: &gtk::Application) {
        if self.watching.replace(true) {
            return;
        }
        let hook = {
            let me = Rc::downgrade(self);
            move |w: &gtk::Window| {
                let me = me.clone();
                w.connect_map(move |w| {
                    if let Some(me) = me.upgrade()
                        && me.active.get()
                        && !w.has_css_class("wados-lock")
                    {
                        me.unshown.borrow_mut().push(w.downgrade());
                    }
                });
            }
        };
        for w in app.windows() {
            hook(&w);
        }
        app.connect_window_added(move |_, w| hook(w));
    }

    /// The lock's off: what was shown meanwhile, shown again (after the
    /// library's done unlocking), in the order it was.
    fn show_unshown(&self) {
        self.active.set(false);
        let windows: Vec<gtk::Window> = self.unshown.borrow_mut().drain(..).filter_map(|w| w.upgrade()).collect();
        if windows.is_empty() {
            return;
        }
        glib::idle_add_local_once(move || {
            for w in windows.into_iter().filter(|w| w.is_visible()) {
                w.set_visible(false);
                w.present();
            }
        });
    }

    /// What the corner's Power off does (it's off outside the machine's session).
    pub fn set_power_off(&self, f: impl Fn() + 'static) {
        *self.power_off.borrow_mut() = Some(Rc::new(f));
    }

    /// The power key on the lock screen: the keys go to its Power off (if
    /// it's on here).
    pub fn focus_power(&self) -> bool {
        let mut any = false;
        for b in self.power_buttons.borrow().iter().filter(|b| b.is_sensitive()) {
            any |= b.grab_focus();
            if let Some(w) = b.root().and_downcast::<gtk::Window>() {
                w.set_focus_visible(true);
            }
        }
        any
    }

    /// Locks, then `then(true)` once every display shows the lock (at once
    /// if it does already), or `then(false)` if it can't lock.
    pub fn lock_then(self: &Rc<Self>, app: &gtk::Application, then: impl FnOnce(bool) + 'static) {
        if self.shown.get() {
            then(true);
            return;
        }
        self.waiting.borrow_mut().push(Box::new(then));
        if let Err(e) = self.lock(app) {
            tracing::error!("{e}");
            self.settle(false);
        }
    }

    fn settle(&self, shown: bool) {
        let waiting: Vec<Then> = self.waiting.borrow_mut().drain(..).collect();
        for f in waiting {
            f(shown);
        }
    }

    pub fn is_locked(&self) -> bool {
        self.current.borrow().as_ref().is_some_and(|l| l.is_locked())
    }

    /// Locks the screen (nothing if it's locked already).
    pub fn lock(self: &Rc<Self>, app: &gtk::Application) -> Result<(), String> {
        // WadSpaces Live's person has no password: a lock there couldn't open.
        if crate::session::is_live() {
            return Ok(());
        }
        if self.current.borrow().is_some() {
            return Ok(());
        }
        if !session_lock::is_supported() {
            return Err("the compositor can't lock the session (ext-session-lock-v1)".into());
        }
        self.watch(app);
        let lock = SessionLock::new();
        let me = Rc::downgrade(self);
        let app = app.downgrade();
        lock.connect_monitor(move |lock, monitor| {
            let (Some(me), Some(app)) = (me.upgrade(), app.upgrade()) else { return };
            let w = me.window(&app, monitor);
            lock.assign(&w, monitor);
            w.present();
            me.windows.borrow_mut().push(w);
        });
        let me = Rc::downgrade(self);
        lock.connect_failed(move |_| {
            tracing::error!("the screen couldn't be locked");
            if let Some(me) = me.upgrade() {
                me.current.borrow_mut().take();
                me.show_unshown();
                me.settle(false);
            }
        });
        let me = Rc::downgrade(self);
        lock.connect_locked(move |_| {
            tracing::info!("locked");
            if let Some(me) = me.upgrade() {
                me.shown.set(true);
                me.settle(true);
            }
        });
        let me = Rc::downgrade(self);
        lock.connect_unlocked(move |_| {
            tracing::info!("unlocked");
            if let Some(me) = me.upgrade() {
                me.shown.set(false);
                for w in me.windows.borrow_mut().drain(..) {
                    w.destroy();
                }
                me.power_buttons.borrow_mut().clear();
                me.current.borrow_mut().take();
                me.show_unshown();
            }
        });
        // Kept first: its signals may come while it's locking (a failure
        // takes it back out), and none may find `current` borrowed.
        *self.current.borrow_mut() = Some(lock.clone());
        self.active.set(true);
        lock.lock();
        Ok(())
    }

    fn unlock(&self) {
        // Out of the cell first: `unlocked` comes while it's unlocking, and
        // its handler takes it out (a borrow held across would abort).
        let l = self.current.borrow().clone();
        if let Some(l) = l {
            l.unlock();
        }
    }

    /// One monitor's lock screen.
    fn window(self: &Rc<Self>, app: &gtk::Application, _monitor: &gdk::Monitor) -> gtk::Window {
        let (login, name) = me();
        let w = gtk::Window::builder().application(app).css_classes(["wados-lock"]).build();
        let overlay = gtk::Overlay::new();
        let pic = gtk::Picture::builder().content_fit(gtk::ContentFit::Cover).can_shrink(true).build();
        if let Some(p) = crate::assets::path("wallpapers/art/silk.svg") {
            pic.set_filename(Some(&p));
        }
        overlay.set_child(Some(&pic));
        let col = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(14)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .css_classes(["lock-card"])
            .build();
        let avatar = gtk::Label::builder()
            .label(name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default())
            .css_classes(["lock-avatar", "display"])
            .halign(gtk::Align::Center)
            .build();
        col.append(&avatar);
        col.append(
            &gtk::Label::builder().label(format!("Welcome {name}")).css_classes(["lock-welcome", "display"]).build(),
        );
        let entry = gtk::PasswordEntry::builder()
            .placeholder_text("Password")
            .show_peek_icon(true)
            .css_classes(["lock-entry"])
            .build();
        col.append(&entry);
        let said = gtk::Label::builder().css_classes(["lock-error"]).build();
        col.append(&said);
        overlay.add_overlay(&col);
        overlay.add_overlay(&self.corner());
        if let Some(n) = self.notes_box() {
            overlay.add_overlay(&n);
        }
        w.set_child(Some(&overlay));
        w.add_controller(to_the_password(&entry));

        let me = Rc::downgrade(self);
        entry.connect_activate(move |entry| {
            let password = entry.text().to_string();
            if password.is_empty() {
                return;
            }
            entry.set_sensitive(false);
            said.set_label("");
            let (tx, rx) = async_channel::bounded(1);
            let login = login.clone();
            std::thread::spawn(move || {
                let _ = tx.send_blocking(check(&login, &password));
            });
            let (entry, said, me) = (entry.clone(), said.clone(), me.clone());
            glib::spawn_future_local(async move {
                let result = rx.recv().await.unwrap_or_else(|_| Err("The check stopped".into()));
                entry.set_sensitive(true);
                entry.set_text("");
                match result {
                    Ok(()) => {
                        if let Some(m) = me.upgrade() {
                            m.unlock();
                        }
                    }
                    Err(e) => {
                        tracing::info!("unlock refused: {e}");
                        said.set_label(&e);
                        entry.grab_focus();
                    }
                }
            });
        });
        w.connect_map(glib::clone!(
            #[weak]
            entry,
            move |_| {
                entry.grab_focus();
            }
        ));
        w
    }

    /// The corner (the spike's): Power off, in the machine's own session.
    fn corner(self: &Rc<Self>) -> gtk::Widget {
        let corner = gtk::Box::builder()
            .halign(gtk::Align::End)
            .valign(gtk::Align::End)
            .margin_end(20)
            .margin_bottom(18)
            .css_classes(["lock-corner"])
            .build();
        let Some(off) = crate::session::items(crate::session::is_machine()).into_iter().find(|i| i.power == Power::Off)
        else {
            return corner.upcast();
        };
        let b = gtk::Button::builder()
            .child(&wad_gtk::Icon::new(off.icon, 16))
            .tooltip_text(&off.tip)
            .css_classes(["lock-power"])
            .build();
        if off.on {
            let me = Rc::downgrade(self);
            b.connect_clicked(move |_| {
                let f = me.upgrade().and_then(|m| m.power_off.borrow().clone());
                if let Some(f) = f {
                    f();
                }
            });
            corner.append(&b);
        } else {
            b.set_sensitive(false);
            // Its tooltip still says why.
            let holder = gtk::Box::builder().tooltip_text(&off.tip).build();
            holder.append(&b);
            corner.append(&holder);
        }
        self.power_buttons.borrow_mut().push(b);
        corner.upcast()
    }
}

/// A key typed with the focus elsewhere (on Power off, say) goes to the
/// password box; Esc clears it.
fn to_the_password(entry: &gtk::PasswordEntry) -> gtk::EventControllerKey {
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let entry = entry.downgrade();
    keys.connect_key_pressed(move |c, key, _, mods| {
        let Some(entry) = entry.upgrade() else { return glib::Propagation::Proceed };
        if key == gdk::Key::Escape {
            entry.set_text("");
            entry.grab_focus();
            return glib::Propagation::Stop;
        }
        let in_entry = c
            .widget()
            .and_downcast::<gtk::Window>()
            .and_then(|w| gtk::prelude::GtkWindowExt::focus(&w))
            .is_some_and(|f| f.is_ancestor(&entry) || f == *entry.upcast_ref::<gtk::Widget>());
        let held = gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK | gdk::ModifierType::SUPER_MASK;
        if in_entry || mods.intersects(held) {
            return glib::Propagation::Proceed;
        }
        let typed = key.to_unicode().filter(|c| !c.is_control());
        if typed.is_none() && key != gdk::Key::BackSpace {
            return glib::Propagation::Proceed;
        }
        entry.grab_focus();
        let mut text = entry.text().to_string();
        match typed {
            Some(ch) => text.push(ch),
            None => {
                text.pop();
            }
        }
        entry.set_text(&text);
        entry.set_position(-1);
        glib::Propagation::Stop
    });
    keys
}
