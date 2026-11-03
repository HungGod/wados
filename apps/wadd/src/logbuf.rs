//! wadd's log: to stderr (the journal) and into a ring buffer for /v1/logs,
//! both with secrets taken out first. Tokens, passwords and the like never
//! reach the journal or the API, however a message was written.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use wad_proto::LogLine;

/// The last `cap` log lines.
#[derive(Clone)]
pub struct LogBuffer {
    lines: Arc<Mutex<VecDeque<LogLine>>>,
    cap: usize,
}

impl LogBuffer {
    pub fn new(cap: usize) -> Self {
        Self { lines: Arc::new(Mutex::new(VecDeque::with_capacity(cap.min(4096)))), cap: cap.max(1) }
    }

    pub fn push(&self, line: LogLine) {
        let mut l = self.lines.lock().unwrap();
        if l.len() == self.cap {
            l.pop_front();
        }
        l.push_back(line);
    }

    /// The last `n` lines, oldest first.
    pub fn tail(&self, n: usize) -> Vec<LogLine> {
        let l = self.lines.lock().unwrap();
        l.iter().skip(l.len().saturating_sub(n)).cloned().collect()
    }
}

/// A tracing layer that writes each event, redacted, to stderr and the buffer.
pub struct BufferLayer {
    pub buffer: LogBuffer,
    pub to_stderr: bool,
}

struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.0, "{value:?}");
        } else {
            let _ = write!(self.0, " {}={value:?}", field.name());
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0.push_str(value);
        } else {
            let _ = write!(self.0, " {}={value}", field.name());
        }
    }
}

impl<S: Subscriber> Layer<S> for BufferLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut m = Message(String::new());
        event.record(&mut m);
        let meta = event.metadata();
        let line = LogLine {
            time: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0),
            level: meta.level().as_str().to_lowercase(),
            target: meta.target().to_string(),
            message: redact(m.0.trim_start()),
        };
        if self.to_stderr {
            // The journal adds the time; the level is enough to scan by.
            eprintln!("{:<5} {}: {}", line.level, line.target, line.message);
        }
        self.buffer.push(line);
    }
}

const TOKEN_PREFIXES: &[&str] = &["github_pat_", "ghp_", "gho_", "ghu_", "ghs_", "ghr_"];
const SECRET_KEYS: &[&str] = &["password", "passwd", "token", "secret", "psk", "authorization", "api_key", "apikey"];
const REDACTED: &str = "[redacted]";

/// Does `s` start with `prefix` (ASCII), in any case? Compared as bytes, so a
/// line whose first bytes are part of a character (`→`) can't panic.
fn starts_with_word(s: &str, prefix: &str) -> bool {
    s.as_bytes().get(..prefix.len()).is_some_and(|b| b.eq_ignore_ascii_case(prefix.as_bytes()))
}

fn token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | '+' | '=')
}

/// The message with secrets replaced by [redacted]: GitHub tokens anywhere,
/// `Bearer <token>`, and the value after a key that names a secret
/// (`password=…`, `"token": "…"`, `refresh_token: …`).
pub fn redact(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while !rest.is_empty() {
        // GitHub tokens.
        if let Some(p) = TOKEN_PREFIXES.iter().find(|p| rest.starts_with(**p))
            && rest[p.len()..].chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        {
            out.push_str(REDACTED);
            rest = rest[p.len()..].trim_start_matches(|c: char| c.is_ascii_alphanumeric() || c == '_');
            continue;
        }
        // Bearer tokens.
        if starts_with_word(rest, "bearer ") {
            out.push_str(&rest[..7]);
            let after = &rest[7..];
            let n = after.len() - after.trim_start_matches(token_char).len();
            if n > 0 {
                out.push_str(REDACTED);
            }
            rest = &after[n..];
            continue;
        }
        // key=value / key: value / "key": "value" for secret-sounding keys.
        let word_len = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_alphanumeric() || c == '_').len();
        if word_len > 0 {
            let word = &rest[..word_len];
            let lower = word.to_ascii_lowercase();
            out.push_str(word);
            rest = &rest[word_len..];
            if SECRET_KEYS.iter().any(|k| lower.contains(k)) {
                let sep = rest.len() - rest.trim_start_matches(['"', '\'', ' ', ':', '=']).len();
                let seps = &rest[..sep];
                if seps.contains([':', '=']) {
                    out.push_str(seps);
                    rest = &rest[sep..];
                    // `Authorization: Bearer <token>`: the scheme and the token both go.
                    for scheme in ["bearer ", "basic "] {
                        if starts_with_word(rest, scheme) {
                            rest = rest[scheme.len()..].trim_start();
                        }
                    }
                    let n = rest.len()
                        - rest
                            .trim_start_matches(|c: char| {
                                !c.is_whitespace() && !matches!(c, '"' | '\'' | ',' | '}' | ')' | ']')
                            })
                            .len();
                    if n > 0 {
                        out.push_str(REDACTED);
                    }
                    rest = &rest[n..];
                }
            }
            continue;
        }
        let c = rest.chars().next().unwrap();
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}
