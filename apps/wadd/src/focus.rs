//! Focus (`GET|POST /v1/focus`): one wadspace on the screen for a time set,
//! with no leaving it until then. Restarting the machine is the only way
//! out early, so it's kept in `/run` ([`Settings::file`]): it survives wadd
//! restarting, and not the machine.
//!
//! wadd holds the rule, so the phone and other machines can't get around
//! it: [`MachineFocus::check_open`], `check_stop` and `check_stream` are
//! called on the paths people use (the API, the remote API). wadd's own stops
//! aren't checked. Streams of other wadspaces go on: they aren't on this
//! screen. The shell does the rest: its keys, the bar, the other displays.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use wad_model::{Focus, Phase, SessionDisplay};
use wad_proto::{ApiError, Event, Level};

use crate::events::Bus;
use crate::jobs::now_ms;
use crate::library::Docs;
use crate::runs::Runs;

pub const MAX_MINUTES: u32 = 12 * 60;
/// wadd.service's RuntimeDirectory.
pub const RUN_DIR: &str = "/run/wados/focus";

struct Now {
    focus: Focus,
    /// It's been on the screen: it stopping now isn't it failing to open.
    opened: bool,
}

pub struct MachineFocus {
    file: PathBuf,
    now: Mutex<Option<Now>>,
    docs: Arc<Docs>,
    runs: Arc<Runs>,
    bus: Bus,
}

/// "25 minutes", "1 hour", "2 hours 5 minutes".
pub fn span_words(minutes: i64) -> String {
    let plural = |n: i64, what: &str| format!("{n} {what}{}", if n == 1 { "" } else { "s" });
    match (minutes / 60, minutes % 60) {
        (0, m) => plural(m, "minute"),
        (h, 0) => plural(h, "hour"),
        (h, m) => format!("{} {}", plural(h, "hour"), plural(m, "minute")),
    }
}

impl MachineFocus {
    pub fn new(dir: &Path, docs: Arc<Docs>, runs: Arc<Runs>, bus: Bus) -> Arc<Self> {
        let file = dir.join("focus.json");
        let kept = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice::<Focus>(&b).ok());
        let now = kept.filter(|f| f.until > now_ms()).map(|focus| Now { focus, opened: true });
        let me = Arc::new(Self { file, now: Mutex::new(now), docs, runs, bus });
        me.publish();
        me
    }

    pub fn get(&self) -> Option<Focus> {
        self.now.lock().unwrap().as_ref().map(|n| n.focus.clone())
    }

    fn publish(&self) {
        self.bus.publish(Event::Focus { focus: self.get() });
    }

    fn name(&self, id: &str) -> String {
        self.docs.wadspace(id).map_or_else(|| id.to_string(), |w| w.name)
    }

    fn held(&self, f: &Focus) -> ApiError {
        ApiError::conflict(format!(
            "Focusing in {}: it stays on the screen until the time’s up, or the machine restarts",
            self.name(&f.ws)
        ))
    }

    /// May `id` open on `display` now?
    pub fn check_open(&self, id: &str, display: SessionDisplay) -> Result<(), ApiError> {
        let Some(f) = self.get() else { return Ok(()) };
        let allowed = if id == f.ws { display == SessionDisplay::Screen } else { display == SessionDisplay::Stream };
        if allowed { Ok(()) } else { Err(self.held(&f)) }
    }

    pub fn check_stop(&self, id: &str) -> Result<(), ApiError> {
        match self.get() {
            Some(f) if f.ws == id => Err(self.held(&f)),
            _ => Ok(()),
        }
    }

    /// A stream of the wadspace on the screen would take it off.
    pub fn check_stream(&self, id: &str) -> Result<(), ApiError> {
        self.check_stop(id)
    }

    /// Server mode, and anything else that would end it from outside.
    pub fn check_free(&self) -> Result<(), ApiError> {
        self.get().map_or(Ok(()), |f| Err(self.held(&f)))
    }

    fn save(&self, focus: Option<&Focus>) {
        let r = match focus {
            Some(f) => serde_json::to_vec(f)
                .map_err(std::io::Error::other)
                .and_then(|b| crate::store::write_atomic_mkdir(&self.file, &b)),
            None => match std::fs::remove_file(&self.file) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            },
        };
        if let Err(e) = r {
            tracing::warn!("keeping the focus: {e}");
        }
    }

    pub fn start(&self, ws: &str, minutes: u32) -> Result<Focus, ApiError> {
        if minutes == 0 {
            return Err(ApiError::bad_request("Set at least a minute"));
        }
        if minutes > MAX_MINUTES {
            return Err(ApiError::bad_request("At most 12 hours"));
        }
        if let Some(f) = self.get() {
            return Err(self.held(&f));
        }
        let w = self.docs.wadspace(ws).ok_or_else(|| ApiError::not_found(format!("no wadspace {ws:?}")))?;
        if w.machine != self.docs.machine() {
            return Err(ApiError::conflict(format!(
                "Focus runs a wadspace on this machine, and {} lives on another: copy it here, then focus in the copy",
                w.name
            )));
        }
        // Everything else full screen goes to the background.
        for r in self.runs.states() {
            if r.id != ws && r.display == Some(SessionDisplay::Screen) {
                self.runs.open(&r.id, SessionDisplay::Background)?;
            }
        }
        self.runs.open(ws, SessionDisplay::Screen)?;
        let started = now_ms();
        let focus = Focus { ws: ws.to_string(), started, until: started + i64::from(minutes) * 60_000 };
        self.save(Some(&focus));
        *self.now.lock().unwrap() = Some(Now { focus: focus.clone(), opened: false });
        self.publish();
        Ok(focus)
    }

    fn end(&self, title: String, body: String) {
        *self.now.lock().unwrap() = None;
        self.save(None);
        self.publish();
        self.bus.publish(Event::Notice {
            level: Level::Info,
            title,
            undo: None,
            body: Some(body),
            icon: Some("timer".into()),
            action: None,
            toast: true,
        });
    }

    /// The clock, and the wadspace: called every second.
    pub fn tick(&self, now: i64) {
        let Some((f, opened)) = self.now.lock().unwrap().as_ref().map(|n| (n.focus.clone(), n.opened)) else {
            return;
        };
        let name = self.name(&f.ws);
        if now >= f.until {
            let span = span_words((f.until - f.started) / 60_000);
            return self.end("Focus is over".into(), format!("{span} in {name}. You can leave it now."));
        }
        let run = self.runs.state(&f.ws);
        match run.state.phase {
            Phase::Ready if !opened => {
                if let Some(n) = self.now.lock().unwrap().as_mut() {
                    n.opened = true;
                }
            }
            Phase::Idle | Phase::Error if !opened => {
                self.end("Focus didn’t start".into(), format!("{name} didn’t open, so there was nothing to focus in."));
            }
            Phase::Idle | Phase::Error => {
                self.end("Focus is over".into(), format!("{name} stopped, so there’s nothing to stay in."));
            }
            _ => {}
        }
    }

    pub async fn run(self: Arc<Self>) {
        loop {
            self.tick(now_ms());
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::span_words;

    #[test]
    fn a_span_reads_as_a_person_would_say_it() {
        assert_eq!(span_words(1), "1 minute");
        assert_eq!(span_words(25), "25 minutes");
        assert_eq!(span_words(60), "1 hour");
        assert_eq!(span_words(125), "2 hours 5 minutes");
    }
}
