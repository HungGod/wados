//! The app's line to wadd (as the shell's `wadd.rs`): its event stream,
//! followed for as long as the app runs (reconnecting by itself), handed to
//! the GTK main loop; and calls, whose answers come back there.
//!
//! wad-wadd-client is async (tokio); GTK isn't. The calls run on a small
//! tokio runtime of the app's own.

use std::future::Future;
use std::sync::Arc;

use gtk::glib;
use wad_wadd_client::{Failure, Incoming, Wadd};

#[derive(Clone)]
pub struct Link {
    rt: Arc<tokio::runtime::Runtime>,
    client: Wadd,
}

impl Link {
    /// Follows wadd (WADD_SOCKET, the machine's, or a laptop's): each of
    /// its events to `on`, on the main loop.
    pub fn start(on: impl Fn(Incoming) + 'static) -> Self {
        let rt = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
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
        glib::spawn_future_local(async move {
            while let Ok(i) = rx.recv().await {
                on(i);
            }
        });
        Self { rt, client }
    }

    /// Runs `call` on wadd, then `done` with its answer on the main loop.
    pub fn call<T, F>(&self, call: impl FnOnce(Wadd) -> F, done: impl FnOnce(Result<T, Failure>) + 'static)
    where
        T: Send + 'static,
        F: Future<Output = Result<T, Failure>> + Send + 'static,
    {
        let handle = self.rt.spawn(call(self.client.clone()));
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
}
