//! The installer's line to wadd: calls on a small tokio runtime of its own,
//! whose answers come back on the GTK main loop.

use std::future::Future;
use std::sync::Arc;

use gtk::glib;
use wad_wadd_client::{Failure, Wadd};

#[derive(Clone)]
pub struct Link {
    rt: Arc<tokio::runtime::Runtime>,
    client: Wadd,
}

impl Link {
    pub fn start() -> Self {
        let rt = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .thread_name("wadd-link")
                .enable_all()
                .build()
                .expect("a tokio runtime"),
        );
        Self { rt, client: Wadd::here() }
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
