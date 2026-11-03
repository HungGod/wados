//! The lock screen (`shell/lock/LockScreen.svelte`): "Welcome Sam" (the
//! account's name, never typed) and its password, over the lock wallpaper
//! (Silk). Every start lands here, and so does Lock, idle and suspend.
//!
//! It holds the screen with ext-session-lock (wad_gtk::session_lock): if the
//! shell dies while locked, the screen stays locked. The password is checked
//! by PAM (`/etc/pam.d/wados-lock`, `WADOS_PAM_SERVICE` to use another), off
//! the main thread; pam_unix checks a user's own password without root
//! (unix_chkpwd), and pam_faillock slows guessing.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::{gdk, glib, prelude::*};
use wad_gtk::session_lock::{self, SessionLock};

pub struct Lock {
    current: RefCell<Option<SessionLock>>,
    windows: RefCell<Vec<gtk::Window>>,
}

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
        Rc::new(Self { current: RefCell::default(), windows: RefCell::default() })
    }

    pub fn is_locked(&self) -> bool {
        self.current.borrow().as_ref().is_some_and(|l| l.is_locked())
    }

    /// Locks the screen (nothing if it's locked already).
    pub fn lock(self: &Rc<Self>, app: &gtk::Application) -> Result<(), String> {
        if self.current.borrow().is_some() {
            return Ok(());
        }
        if !session_lock::is_supported() {
            return Err("the compositor can't lock the session (ext-session-lock-v1)".into());
        }
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
            }
        });
        lock.connect_locked(|_| tracing::info!("locked"));
        let me = Rc::downgrade(self);
        lock.connect_unlocked(move |_| {
            tracing::info!("unlocked");
            if let Some(me) = me.upgrade() {
                for w in me.windows.borrow_mut().drain(..) {
                    w.destroy();
                }
                me.current.borrow_mut().take();
            }
        });
        lock.lock();
        *self.current.borrow_mut() = Some(lock);
        Ok(())
    }

    fn unlock(&self) {
        if let Some(l) = self.current.borrow().as_ref() {
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
        w.set_child(Some(&overlay));

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
}
