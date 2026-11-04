//! wadd from the UI: wadbrowser-core's blocking client, run on a thread of
//! its own, its answer brought back to the main thread. No wadd (inside a
//! wadspace) is an `Err` whose `is_unreachable()` is true, quickly.

pub use wadbrowser_core::wadd::{Failure, Wadd};
use webkit6::glib;

/// Runs `f` with wadd off the main thread, then `done` with its answer on it.
///
/// ```ignore
/// wadbrowser_webkit::wadd::call(|w| w.web_keys(), |keys| match keys {
///     Ok(keys) => show(keys),
///     Err(e) if e.is_unreachable() => {} // no wadd here
///     Err(e) => toast(&e.message),
/// });
/// ```
pub fn call<T: Send + 'static>(
    f: impl FnOnce(&Wadd) -> Result<T, Failure> + Send + 'static,
    done: impl FnOnce(Result<T, Failure>) + 'static,
) {
    let (tx, rx) = async_channel::bounded(1);
    let spawned = std::thread::Builder::new().name("wadbrowser-wadd".into()).spawn(move || {
        let out = match Wadd::here() {
            Some(w) => f(&w),
            None => Err(Failure {
                status: 0,
                code: "unreachable".into(),
                message: "wadd isn't answering.".into(),
                still_open: vec![],
            }),
        };
        let _ = tx.send_blocking(out);
    });
    if spawned.is_err() {
        return;
    }
    glib::spawn_future_local(async move {
        if let Ok(out) = rx.recv().await {
            done(out);
        }
    });
}
