//! The lock screen's hold on the screen: gtk4-layer-shell's session lock
//! (ext-session-lock-v1), which its Rust crate doesn't wrap yet. While it's
//! locked, the compositor shows only the windows given to it, one per
//! monitor, and nothing else gets input. If the shell dies while locked, the
//! screen stays locked.
//!
//! ```ignore
//! let lock = SessionLock::new();
//! lock.connect_monitor(|lock, monitor| lock.assign(&window_for(monitor), monitor));
//! lock.lock();
//! // … the password checks out:
//! lock.unlock();
//! ```

use gtk::{gdk, glib, glib::translate::*, prelude::*};

mod ffi {
    use gtk::glib::ffi::{GType, gboolean};

    #[repr(C)]
    pub struct GtkSessionLockInstance {
        _private: [u8; 0],
    }

    #[repr(C)]
    pub struct GtkSessionLockInstanceClass {
        pub parent_class: gtk::glib::gobject_ffi::GObjectClass,
    }

    #[link(name = "gtk4-layer-shell")]
    unsafe extern "C" {
        pub fn gtk_session_lock_is_supported() -> gboolean;
        pub fn gtk_session_lock_instance_get_type() -> GType;
        pub fn gtk_session_lock_instance_new() -> *mut GtkSessionLockInstance;
        pub fn gtk_session_lock_instance_lock(lock: *mut GtkSessionLockInstance) -> gboolean;
        pub fn gtk_session_lock_instance_unlock(lock: *mut GtkSessionLockInstance);
        pub fn gtk_session_lock_instance_is_locked(lock: *mut GtkSessionLockInstance) -> gboolean;
        pub fn gtk_session_lock_instance_assign_window_to_monitor(
            lock: *mut GtkSessionLockInstance,
            window: *mut gtk::ffi::GtkWindow,
            monitor: *mut gtk::gdk::ffi::GdkMonitor,
        );
    }
}

glib::wrapper! {
    /// One lock of the session (a new one for each time it locks).
    pub struct SessionLock(Object<ffi::GtkSessionLockInstance, ffi::GtkSessionLockInstanceClass>);

    match fn {
        type_ => || ffi::gtk_session_lock_instance_get_type(),
    }
}

/// The compositor can lock the session (labwc can).
pub fn is_supported() -> bool {
    // SAFETY: a plain query.
    unsafe { ffi::gtk_session_lock_is_supported() != 0 }
}

impl SessionLock {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        // SAFETY: returns a new, owned instance.
        unsafe { from_glib_full(ffi::gtk_session_lock_instance_new()) }
    }

    /// Asks to lock. `false`: it can't (another lock holds the screen, or no
    /// support); `connect_failed` hears the same.
    pub fn lock(&self) -> bool {
        // SAFETY: self is a valid instance.
        unsafe { ffi::gtk_session_lock_instance_lock(self.to_glib_none().0) != 0 }
    }

    pub fn unlock(&self) {
        // SAFETY: self is a valid instance.
        unsafe { ffi::gtk_session_lock_instance_unlock(self.to_glib_none().0) }
    }

    pub fn is_locked(&self) -> bool {
        // SAFETY: self is a valid instance.
        unsafe { ffi::gtk_session_lock_instance_is_locked(self.to_glib_none().0) != 0 }
    }

    /// `window` covers `monitor` while it's locked (present it after).
    pub fn assign(&self, window: &impl IsA<gtk::Window>, monitor: &gdk::Monitor) {
        // SAFETY: all three are valid; the call keeps what it needs.
        unsafe {
            ffi::gtk_session_lock_instance_assign_window_to_monitor(
                self.to_glib_none().0,
                window.as_ref().to_glib_none().0,
                monitor.to_glib_none().0,
            )
        }
    }

    /// Each monitor while it's locked (those there when it locks, and any
    /// plugged in after): give it a window with [`SessionLock::assign`].
    pub fn connect_monitor(&self, f: impl Fn(&Self, &gdk::Monitor) + 'static) -> glib::SignalHandlerId {
        self.connect_local("monitor", false, move |args| {
            let lock = args[0].get::<Self>().ok()?;
            let monitor = args[1].get::<gdk::Monitor>().ok()?;
            f(&lock, &monitor);
            None
        })
    }

    /// The compositor locked it: only our windows show now.
    pub fn connect_locked(&self, f: impl Fn(&Self) + 'static) -> glib::SignalHandlerId {
        self.connect_local("locked", false, move |args| {
            f(&args[0].get::<Self>().ok()?);
            None
        })
    }

    /// It couldn't lock (another lock screen has it, say).
    pub fn connect_failed(&self, f: impl Fn(&Self) + 'static) -> glib::SignalHandlerId {
        self.connect_local("failed", false, move |args| {
            f(&args[0].get::<Self>().ok()?);
            None
        })
    }

    pub fn connect_unlocked(&self, f: impl Fn(&Self) + 'static) -> glib::SignalHandlerId {
        self.connect_local("unlocked", false, move |args| {
            f(&args[0].get::<Self>().ok()?);
            None
        })
    }
}
