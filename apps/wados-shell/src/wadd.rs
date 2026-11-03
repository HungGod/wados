//! The shell's line to wadd: its event stream, followed for as long as the
//! shell runs (reconnecting by itself) into the [`Store`], and calls (edits,
//! open, stop, build…) whose answers come back on the main loop.
//!
//! wad-wadd-client is async (tokio); GTK isn't. The calls run on a small
//! tokio runtime of the shell's own, and what they return is handed to the
//! GTK main loop through a channel.

use std::future::Future;
use std::rc::Rc;
use std::sync::Arc;

use gtk::glib;
use wad_model::SessionDisplay;
use wad_model::edit::Op;
use wad_proto::{BuildAnswer, OpResult, WadspaceRun};
use wad_wadd_client::{Failure, Incoming, Wadd};

use crate::store::Store;

pub struct Link {
    rt: Arc<tokio::runtime::Runtime>,
    client: Wadd,
    store: Rc<Store>,
}

impl Link {
    /// Follows wadd (where it is: WADD_SOCKET, the machine's, or a laptop's)
    /// into `store`.
    pub fn start(store: Rc<Store>) -> Rc<Self> {
        let rt = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("wadd-link")
                .enable_all()
                .build()
                .expect("a tokio runtime"),
        );
        let client = Wadd::here();
        tracing::info!("wadd at {}", client.socket().display());
        let (tx, rx) = async_channel::unbounded::<Incoming>();
        {
            let _enter = rt.enter();
            client.follow(move |i| tx.send_blocking(i).is_ok());
        }
        let s = store.clone();
        glib::spawn_future_local(async move {
            while let Ok(i) = rx.recv().await {
                match i {
                    Incoming::Event(e) => {
                        s.set_connected(true);
                        s.apply(e);
                    }
                    Incoming::Other(kind, _) => tracing::debug!("an event this shell doesn't know: {kind}"),
                    Incoming::Disconnected => s.set_connected(false),
                }
            }
        });
        Rc::new(Self { rt, client, store })
    }

    pub fn store(&self) -> &Rc<Store> {
        &self.store
    }

    /// Runs `call` on wadd, then `done` with its answer on the main loop.
    pub fn call<T, F>(&self, call: impl FnOnce(Wadd) -> F, done: impl FnOnce(Result<T, Failure>) + 'static)
    where
        T: Send + 'static,
        F: Future<Output = Result<T, Failure>> + Send + 'static,
    {
        let fut = call(self.client.clone());
        let handle = self.rt.spawn(fut);
        glib::spawn_future_local(async move {
            let r = match handle.await {
                Ok(r) => r,
                Err(e) => {
                    Err(Failure { status: 0, code: "internal".into(), message: e.to_string(), still_open: vec![] })
                }
            };
            done(r);
        });
    }

    /// One edit. A refusal is said as a toast (wadd's words).
    pub fn op(self: &Rc<Self>, op: Op, done: impl FnOnce(OpResult) + 'static) {
        let me = self.clone();
        self.call(
            move |w| async move { w.op(op).await },
            move |r| match r {
                Ok(out) => done(out),
                Err(e) => me.say_failure(&e),
            },
        );
    }

    pub fn undo(self: &Rc<Self>, id: &str) {
        let (me, id) = (self.clone(), id.to_string());
        self.call(
            move |w| async move { w.undo(&id).await },
            move |r| {
                if let Err(e) = r {
                    me.say_failure(&e)
                }
            },
        );
    }

    pub fn open(self: &Rc<Self>, id: &str, display: SessionDisplay, done: impl FnOnce(WadspaceRun) + 'static) {
        let (me, id) = (self.clone(), id.to_string());
        self.call(
            move |w| async move { w.open(&id, display).await },
            move |r| match r {
                Ok(run) => done(run),
                Err(e) => me.say_failure(&e),
            },
        );
    }

    pub fn stop(self: &Rc<Self>, id: &str) {
        let (me, id) = (self.clone(), id.to_string());
        self.call(
            move |w| async move { w.stop(&id).await },
            move |r| {
                if let Err(e) = r {
                    me.say_failure(&e)
                }
            },
        );
    }

    pub fn build(self: &Rc<Self>, id: &str, done: impl FnOnce(BuildAnswer) + 'static) {
        let (me, id) = (self.clone(), id.to_string());
        self.call(
            move |w| async move { w.build(&id).await },
            move |r| match r {
                Ok(a) => done(a),
                Err(e) => me.say_failure(&e),
            },
        );
    }

    /// Tells wadd where the screen is: this shell's compositor.
    pub fn register_screen(&self) {
        let Some(socket) = wayland_socket() else {
            tracing::warn!("no WAYLAND_DISPLAY: wadd will look for the compositor itself");
            return;
        };
        let uid = nix_uid();
        self.call(
            move |w| async move { w.screen_session(&socket, uid).await },
            |r| match r {
                Ok(s) => tracing::info!("wadd puts wadspaces on this screen ({:?})", s.from),
                Err(e) => tracing::warn!("couldn't tell wadd where the screen is: {e}"),
            },
        );
    }

    /// A failure, as the person sees it: a toast with wadd's words.
    fn say_failure(&self, e: &Failure) {
        tracing::info!("wadd: {} ({})", e.message, e.code);
        self.store.apply(wad_proto::Event::Notice {
            level: if e.is_unreachable() { wad_proto::Level::Warn } else { wad_proto::Level::Error },
            title: e.message.clone(),
            undo: None,
        });
    }
}

/// This session's Wayland socket, as an absolute path.
fn wayland_socket() -> Option<String> {
    let d = std::env::var("WAYLAND_DISPLAY").ok().filter(|d| !d.is_empty())?;
    if d.starts_with('/') {
        return Some(d);
    }
    let rt = std::env::var("XDG_RUNTIME_DIR").ok()?;
    Some(format!("{rt}/{d}"))
}

fn nix_uid() -> u32 {
    // SAFETY: getuid can't fail.
    unsafe { libc::getuid() }
}
