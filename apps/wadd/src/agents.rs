//! Agents (`/v1/agents`, `/v1/sandboxes`): Claude Code at work in wadspaces.
//!
//! **A job** is turns of the real client, `claude -p`, run in its wadspace
//! ([`Runner`]) and followed through its stream ([`stream`]). Between turns it
//! waits for the person: a sandbox to allow, one to try, or a pull request.
//! `git push` and `gh pr` are denied to it until they say ship: a guard on
//! its tools, not a security boundary, for the wadspace has its own credentials.
//!
//! **The `wadspaces` MCP server** ([`mcp`]) is a socket per wadspace with its
//! Agents switch on, so the caller is known by the socket, and a wadspace
//! with it off has none. Approvals happen here, never in the container.
//! What it says is untrusted: cut to size, and shown as data.
//!
//! **A sandbox** is a throwaway wadspace ([`Hands`]), with no keys or
//! projects: code an agent just wrote runs there, never on the machine.

pub mod mcp;
pub mod stream;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use wad_model::agents::{
    Agent, AgentNote, AgentStatus, Ask, CheckItem, Sandbox, SandboxState, StepId, StepState, Verdict, capped, wants,
};
use wad_model::{AppId, SessionDisplay};
use wad_proto::{ApiError, Event, Level, NoticeAction};

use crate::events::Bus;
use crate::jobs::now_ms;
use crate::library::Docs;

/// Where the sockets are, for this wadd.
pub fn sockets_dir(state_dir: &Path, system: bool) -> PathBuf {
    if system { RUN_DIR.into() } else { state_dir.join("agent-sockets") }
}

/// In a wadspace: its socket's folder, and the socket.
pub const SOCKET_DIR: &str = "/run/wadspaces-agents";
pub const SOCKET: &str = "mcp.sock";
/// wadd.service's RuntimeDirectory.
pub const RUN_DIR: &str = "/run/wados/agents";
const MCP_CONFIG: &str = "/run/wadspaces-extra/mcp.json";
const ALLOWED: &str = "Bash,mcp__wadspaces__*";
const NOT_YET: &str = "Bash(git push *),Bash(gh pr *)";
const HOW: &str = "You're working for a person on their machine, in a wadspace. The `wadspaces` MCP tools reach \
their desktop: report with `progress`. For something easier to judge by using it, call `request_sandbox`, then end \
your turn: you'll hear their answer. If you're allowed, call `set_checklist`, then `update_sandbox`. Don't push or \
open a pull request until you're told to.";

pub type OnLine<'a> = dyn FnMut(&str) + Send + 'a;

/// Runs Claude Code in a wadspace.
#[async_trait]
pub trait Runner: Send + Sync + 'static {
    /// `args`, in `ws`, each line of output to `on_line`, until it ends.
    async fn run(&self, ws: &str, args: &[String], on_line: &mut OnLine<'_>) -> Result<(), String>;
    /// Ends the turn, by SIGINT.
    async fn interrupt(&self, ws: &str);
}

/// Sandboxes: throwaway wadspaces.
#[async_trait]
pub trait Hands: Send + Sync + 'static {
    /// One for `task`: its wadspace.
    async fn open(&self, from: &str, task: &str) -> Result<String, String>;
    /// `path` of `from` into `sandbox`, and `run` started there.
    async fn update(&self, from: &str, sandbox: &str, path: Option<&str>, run: Option<&str>) -> Result<(), String>;
    async fn close(&self, sandbox: &str);
    /// Is a wadspace up? Started in the background, if not.
    async fn ensure_up(&self, ws: &str) -> Result<(), String>;
}

/// The arguments of one turn.
pub fn turn_args(prompt: &str, session: Option<&str>, may_ship: bool) -> Vec<String> {
    let mut a: Vec<String> = ["-p", prompt, "--output-format", "stream-json", "--verbose"].map(String::from).to_vec();
    a.extend(["--mcp-config", MCP_CONFIG, "--strict-mcp-config"].map(String::from));
    a.extend(["--permission-mode", "acceptEdits", "--allowedTools", ALLOWED].map(String::from));
    a.extend(["--append-system-prompt", HOW].map(String::from));
    if !may_ship {
        a.extend(["--disallowedTools", NOT_YET].map(String::from));
    }
    if let Some(s) = session {
        a.extend(["--resume", s].map(String::from));
    }
    a
}

/// `{"mcpServers": …}`, in a wadspace with its Agents switch on.
pub fn mcp_config() -> String {
    serde_json::json!({ "mcpServers": { "wadspaces": { "command": "wadspaces-mcp", "args": [] } } }).to_string()
}

/// A path under a wadspace's home, if it's only that.
pub fn in_home(path: &str) -> bool {
    !path.is_empty() && !path.starts_with('/') && !path.split('/').any(|p| p == "..")
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentsState {
    pub agents: Vec<Agent>,
    pub sandboxes: Vec<Sandbox>,
}

#[derive(Debug, Clone, Deserialize)]
struct ItemIn {
    id: String,
    label: String,
    how: String,
    done: u8,
}

pub struct MachineAgents {
    me: Weak<MachineAgents>,
    file: PathBuf,
    run_dir: PathBuf,
    st: Mutex<AgentsState>,
    served: Mutex<HashMap<String, tokio::task::JoinHandle<()>>>,
    seq: AtomicU64,
    runner: Arc<dyn Runner>,
    hands: Arc<dyn Hands>,
    docs: Arc<Docs>,
    bus: Bus,
}

fn busy_with(name: &str, task: &str) -> ApiError {
    ApiError::conflict(format!("Claude Code in {name} is busy with “{task}”: tell it in its chat"))
}

impl MachineAgents {
    pub fn new(
        state_dir: &Path,
        run_dir: &Path,
        runner: Arc<dyn Runner>,
        hands: Arc<dyn Hands>,
        docs: Arc<Docs>,
        bus: Bus,
    ) -> Arc<Self> {
        let file = state_dir.join("agents.json");
        let mut st: AgentsState =
            std::fs::read(&file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        // wadd restarted: no turn is running now.
        for a in st.agents.iter_mut().filter(|a| a.status == AgentStatus::Working) {
            a.status = AgentStatus::Stopped;
            a.now = "wadd restarted while it worked".into();
        }
        let seq = st.agents.len() as u64 + st.sandboxes.len() as u64;
        Arc::new_cyclic(|me| Self {
            me: me.clone(),
            file,
            run_dir: run_dir.to_path_buf(),
            st: Mutex::new(st),
            served: Mutex::default(),
            seq: AtomicU64::new(seq),
            runner,
            hands,
            docs,
            bus,
        })
    }

    pub fn state(&self) -> AgentsState {
        self.st.lock().unwrap().clone()
    }

    fn changed(&self) {
        let st = self.state();
        if let Ok(b) = serde_json::to_vec(&st)
            && let Err(e) = crate::streams::write_private(&self.file, &b, None)
        {
            tracing::warn!("keeping the agents: {e}");
        }
        self.bus.publish(Event::Agents(Box::new(wad_proto::Agents { agents: st.agents, sandboxes: st.sandboxes })));
    }

    fn with<T>(&self, id: &str, f: impl FnOnce(&mut Agent) -> T) -> Result<T, ApiError> {
        let out = {
            let mut st = self.st.lock().unwrap();
            let a = st.agents.iter_mut().find(|a| a.id == id);
            f(a.ok_or_else(|| ApiError::not_found(format!("no agent {id:?}")))?)
        };
        self.changed();
        Ok(out)
    }

    fn live_on(&self, ws: &str) -> Option<Agent> {
        self.st.lock().unwrap().agents.iter().find(|a| a.ws == ws && a.live()).cloned()
    }

    fn toast(&self, level: Level, title: &str, body: &str, agent: &str, verb: &str) {
        self.bus.publish(Event::Notice {
            level,
            title: title.to_string(),
            undo: None,
            body: Some(body.to_string()),
            icon: Some("bot".into()),
            action: Some(NoticeAction::new("Review", "app.open", &[verb, agent])),
            toast: true,
        });
    }

    /// `send` from the Agents app, or a word to the one at work.
    pub async fn send(&self, ws: &str, text: &str) -> Result<Agent, ApiError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(ApiError::bad_request("Say what it is first"));
        }
        let doc = self.docs.wadspace(ws).ok_or_else(|| ApiError::not_found(format!("no wadspace {ws:?}")))?;
        if doc.machine != self.docs.machine() {
            return Err(ApiError::conflict(format!(
                "{} lives on another machine: copy it here to send it work",
                doc.name
            )));
        }
        let has = doc.ext(AppId::Vscode, "anthropic.claude-code").is_some()
            || doc.ext(AppId::Terminal, "claude-code").is_some();
        if !has {
            return Err(ApiError::conflict(format!("{} has no Claude Code: add it in the Builder first", doc.name)));
        }
        if let Some(a) = self.live_on(ws) {
            return Err(busy_with(&doc.name, &a.task));
        }
        let id = format!("agent-{}", self.seq.fetch_add(1, Ordering::SeqCst) + 1);
        let mut a = Agent::new(id.clone(), ws.to_string(), text.to_string(), now_ms());
        a.note(AgentNote::You { text: text.to_string() });
        self.st.lock().unwrap().agents.insert(0, a.clone());
        self.changed();
        self.turn(&id, text.to_string());
        Ok(a)
    }

    /// A turn of Claude Code, in the background.
    fn turn(&self, id: &str, prompt: String) {
        let Some(me) = self.me.upgrade() else { return };
        // At work from now, not from when its task gets to run.
        let _ = self.with(id, |a| {
            a.status = AgentStatus::Working;
            a.ask = None;
            a.now = "Working…".into();
        });
        let id = id.to_string();
        tokio::spawn(async move { me.run_turn(&id, prompt).await });
    }

    async fn run_turn(&self, id: &str, prompt: String) {
        let got = self.with(id, |a| (a.ws.clone(), a.session.clone(), a.may_ship));
        let Ok((ws, session, may_ship)) = got else { return };
        if let Err(e) = self.serve(&ws) {
            tracing::warn!("{ws}: its MCP socket: {e}");
        }
        if let Err(e) = self.hands.ensure_up(&ws).await {
            let _ = self.with(id, |a| {
                a.status = AgentStatus::Stopped;
                a.now = e.clone();
            });
            return;
        }
        let args = turn_args(&prompt, session.as_deref(), may_ship);
        let mut ended: Option<(String, bool)> = None;
        let mut on_line = |line: &str| {
            for seen in stream::read(line) {
                match seen {
                    stream::Seen::Session(s) => {
                        let _ = self.with(id, |a| a.session = Some(s));
                    }
                    stream::Seen::Note(n) => {
                        let _ = self.with(id, |a| a.note(n));
                    }
                    stream::Seen::Result { text, failed } => ended = Some((text, failed)),
                }
            }
        };
        let ran = self.runner.run(&ws, &args, &mut on_line).await;
        let stopped = self.state().agents.iter().any(|a| a.id == id && a.status == AgentStatus::Stopped);
        if stopped {
            return;
        }
        // A failure inside the run is its result, and the client exits non-zero with it.
        let why = match (&ran, &ended) {
            (_, Some((_, false))) => None,
            (_, Some((text, true))) if !text.is_empty() => Some(text.clone()),
            (_, Some(_)) => Some("It stopped with an error".to_string()),
            (Err(e), None) => Some(e.clone()),
            (Ok(()), None) => Some("It ended without saying how it went".to_string()),
        };
        if let Some(why) = why {
            let _ = self.with(id, |a| {
                a.status = AgentStatus::Stopped;
                a.now = capped(&why);
            });
            self.toast(Level::Error, "Claude Code stopped", &why, id, "agents");
            return;
        }
        let text = ended.map(|(t, _)| t).unwrap_or_default();
        let shipped = self.with(id, |a| {
            if !text.is_empty() {
                a.note(AgentNote::Final { text: capped(&text) });
            }
            if a.may_ship {
                a.status = AgentStatus::Done;
                a.now = "Done".into();
                a.set_step(StepId::Ship, StepState::Done);
                return true;
            }
            a.status = AgentStatus::NeedsYou;
            if a.ask.is_none() && a.sandbox.is_none() {
                a.ask = Some(Ask::Ship);
                a.set_step(StepId::Try, StepState::Waiting);
                a.now = "It's waiting for you".into();
            } else if a.sandbox.is_some() {
                a.set_step(StepId::Try, StepState::Waiting);
            }
            false
        });
        match shipped {
            Ok(true) => self.toast(Level::Ok, "Claude Code is done", &text_line(&self.task_of(id)), id, "agents"),
            Ok(false) => self.waiting(id),
            Err(_) => {}
        }
    }

    fn task_of(&self, id: &str) -> String {
        self.st.lock().unwrap().agents.iter().find(|a| a.id == id).map(|a| a.task.clone()).unwrap_or_default()
    }

    /// What wants the person now, as a toast.
    fn waiting(&self, id: &str) {
        let st = self.state();
        let Some(a) = st.agents.iter().find(|a| a.id == id) else { return };
        let task = text_line(&a.task);
        match &a.ask {
            Some(Ask::Sandbox { .. }) => {
                self.toast(
                    Level::Info,
                    "Claude Code wants a sandbox",
                    &format!("To try “{task}” with you"),
                    id,
                    "agents",
                );
            }
            Some(Ask::Ship) => self.toast(Level::Info, "Claude Code is waiting for you", &task, id, "agents"),
            None => {
                let s = st.sandboxes.iter().find(|s| Some(&s.id) == a.sandbox.as_ref());
                let line = wants(a, s).unwrap_or_default();
                self.toast(Level::Info, &line, &task, id, "agents");
            }
        }
    }

    /// A word to the job, between turns: it carries on with it.
    pub fn say(&self, id: &str, text: &str) -> Result<(), ApiError> {
        let text = text.trim().to_string();
        if text.is_empty() {
            return Err(ApiError::bad_request("Say what it is first"));
        }
        let working = self.with(id, |a| {
            let w = a.status == AgentStatus::Working;
            if !w {
                a.note(AgentNote::You { text: text.clone() });
            }
            w
        })?;
        if working {
            return Err(ApiError::conflict("It's working: its next turn can hear you, or stop it"));
        }
        self.turn(id, text);
        Ok(())
    }

    pub async fn stop(&self, id: &str) -> Result<(), ApiError> {
        let ws = self.with(id, |a| {
            a.status = AgentStatus::Stopped;
            a.now = "Stopped".into();
            a.ask = None;
            a.ws.clone()
        })?;
        self.runner.interrupt(&ws).await;
        self.close_sandbox_of(id).await;
        Ok(())
    }

    /// Allow: its sandbox is made, and it's told.
    pub async fn allow(&self, id: &str) -> Result<(), ApiError> {
        let (ws, task) = self
            .with(id, |a| {
                let asked = matches!(a.ask, Some(Ask::Sandbox { .. }) | Some(Ask::Ship));
                asked.then(|| (a.ws.clone(), a.task.clone()))
            })?
            .ok_or_else(|| ApiError::conflict("It didn't ask for a sandbox"))?;
        let box_ws = self.hands.open(&ws, &task).await.map_err(ApiError::internal)?;
        let n = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let sandbox = Sandbox {
            id: format!("sandbox-{n}"),
            agent: id.to_string(),
            title: task,
            ws: box_ws,
            build: 0,
            state: SandboxState::Starting,
            items: vec![],
        };
        let name = sandbox.id.clone();
        self.st.lock().unwrap().sandboxes.push(sandbox);
        self.with(id, |a| {
            a.sandbox = Some(name.clone());
            a.note(AgentNote::You { text: "Allowed the sandbox.".into() });
        })?;
        self.turn(
            id,
            format!("The person allowed it: your sandbox is {name}. Call set_checklist, then update_sandbox."),
        );
        Ok(())
    }

    /// Not now: open a pull request as it is, or a sandbox after all.
    pub fn deny(&self, id: &str) -> Result<(), ApiError> {
        self.with(id, |a| {
            a.ask = Some(Ask::Ship);
            a.note(AgentNote::You { text: "No sandbox.".into() });
            a.note(AgentNote::Say {
                text: "OK, no sandbox. Should I open a pull request as it is, so you can try it another way?".into(),
            });
        })
    }

    /// Ship: only now may it push, and its sandbox goes.
    pub async fn ship(&self, id: &str) -> Result<(), ApiError> {
        self.with(id, |a| {
            a.may_ship = true;
            a.ask = None;
            a.set_step(StepId::Try, StepState::Done);
            a.set_step(StepId::Ship, StepState::Active);
            a.note(AgentNote::You { text: "Open a pull request.".into() });
        })?;
        self.close_sandbox_of(id).await;
        self.turn(id, "The person says it's good: commit, push, and open a pull request.".into());
        Ok(())
    }

    async fn close_sandbox_of(&self, id: &str) {
        let boxes: Vec<String> = {
            let mut st = self.st.lock().unwrap();
            let open = st.sandboxes.iter_mut().filter(|s| s.agent == id && s.state != SandboxState::Closed);
            open.map(|s| {
                s.state = SandboxState::Closed;
                s.ws.clone()
            })
            .collect()
        };
        for ws in boxes {
            self.hands.close(&ws).await;
        }
        self.changed();
    }

    fn sandbox(&self, id: &str, f: impl FnOnce(&mut Sandbox)) -> Result<(), ApiError> {
        {
            let mut st = self.st.lock().unwrap();
            let s = st.sandboxes.iter_mut().find(|s| s.id == id);
            f(s.ok_or_else(|| ApiError::not_found(format!("no sandbox {id:?}")))?);
        }
        self.changed();
        Ok(())
    }

    /// A verdict on an item.
    pub fn mark(&self, id: &str, item: &str, verdict: Option<Verdict>, note: &str) -> Result<(), ApiError> {
        self.sandbox(id, |s| {
            if let Some(i) = s.items.iter_mut().find(|i| i.id == item) {
                i.verdict = verdict;
                i.note = capped(note);
            }
        })
    }

    /// Send to Claude Code: what the person found, for the next build.
    pub fn send_back(&self, id: &str) -> Result<(), ApiError> {
        let (agent, report) = {
            let st = self.st.lock().unwrap();
            let s = st.sandboxes.iter().find(|s| s.id == id);
            let s = s.ok_or_else(|| ApiError::not_found(format!("no sandbox {id:?}")))?;
            (s.agent.clone(), s.report())
        };
        self.with(&agent, |a| a.note(AgentNote::You { text: report.clone() }))?;
        self.sandbox(id, |s| s.state = SandboxState::Updating)?;
        self.turn(
            &agent,
            format!("From the sandbox: {report}\nFix what's flagged, then update_sandbox with the next build."),
        );
        Ok(())
    }

    /// Close sandbox: it's removed, and it asks what next.
    pub async fn close(&self, id: &str) -> Result<(), ApiError> {
        let agent = self.state().sandboxes.iter().find(|s| s.id == id).map(|s| s.agent.clone());
        let agent = agent.ok_or_else(|| ApiError::not_found(format!("no sandbox {id:?}")))?;
        self.close_sandbox_of(&agent).await;
        self.with(&agent, |a| {
            a.sandbox = None;
            a.ask = Some(Ask::Ship);
            a.status = AgentStatus::NeedsYou;
            a.note(AgentNote::You { text: "Closed the sandbox.".into() });
        })
    }

    // ---------------------------------------------------- the MCP server

    /// `ws`'s socket: its folder is the wadspace's, and a socket's only here
    /// while its Agents switch is on.
    pub fn serve(&self, ws: &str) -> Result<(), String> {
        if !crate::store::safe_id(ws) || !self.docs.wadspace(ws).is_some_and(|w| w.agents) {
            return Ok(());
        }
        let mut served = self.served.lock().unwrap();
        if served.get(ws).is_some_and(|t| !t.is_finished()) {
            return Ok(());
        }
        let dir = self.run_dir.join(ws);
        let say = |e: std::io::Error| format!("{}: {e}", dir.display());
        std::fs::create_dir_all(&dir).map_err(say)?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).map_err(say)?;
        let path = dir.join(SOCKET);
        let _ = std::fs::remove_file(&path);
        // As a screen's socket: SELinux lets a container reach these.
        crate::selinux::label(&dir, crate::selinux::CONTAINER_FILE)?;
        let labels = wad_wayland::security_context::Labels {
            socket: Some(crate::selinux::CONTAINER_SOCKET),
            file: Some(crate::selinux::CONTAINER_FILE),
        };
        let listener = wad_wayland::security_context::bind(&path, &labels)?;
        listener.set_nonblocking(true).map_err(say)?;
        let listener = tokio::net::UnixListener::from_std(listener).map_err(say)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).map_err(say)?;
        let (me, who) = (self.me.clone(), ws.to_string());
        served.insert(
            ws.to_string(),
            tokio::spawn(async move {
                while let Ok((sock, _)) = listener.accept().await {
                    let (me, who) = (me.clone(), who.clone());
                    tokio::spawn(async move { connection(me, who, sock).await });
                }
            }),
        );
        Ok(())
    }

    /// The switch went off: its socket goes, and its folder.
    pub fn unserve(&self, ws: &str) {
        if let Some(t) = self.served.lock().unwrap().remove(ws) {
            t.abort();
        }
        if !crate::store::safe_id(ws) {
            return;
        }
        let dir = self.run_dir.join(ws);
        let _ = std::fs::remove_file(dir.join(SOCKET));
        // Left behind, it's one empty folder per wadspace that ever had agents.
        let _ = std::fs::remove_dir(&dir);
    }

    /// Follows every wadspace's switch.
    pub async fn run(self: Arc<Self>) {
        loop {
            let lib = self.docs.snapshot();
            for w in &lib.wadspaces {
                if w.agents {
                    if let Err(e) = self.serve(&w.id) {
                        tracing::warn!("{}: its MCP socket: {e}", w.id);
                    }
                } else if self.served.lock().unwrap().contains_key(&w.id) {
                    self.unserve(&w.id);
                }
            }
            // One gone for good (purged) never reaches the loop above: its
            // folder has no wadspace to turn the switch off for, so clear
            // what's left of it here.
            for id in self.stray_dirs(&lib) {
                self.unserve(&id);
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    /// The folders in `run_dir` no wadspace of `lib` has any more.
    fn stray_dirs(&self, lib: &wad_model::edit::Library) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(&self.run_dir) else { return vec![] };
        entries
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|id| crate::store::safe_id(id) && lib.wadspace(id).is_none())
            .collect()
    }

    /// A tool, called from `ws`: a refusal shows in the job's work too.
    pub async fn tool(&self, ws: &str, name: &str, args: &Value) -> Result<String, String> {
        let r = self.tool_now(ws, name, args).await;
        if let (Err(e), Some(a)) = (&r, self.live_on(ws)) {
            tracing::warn!("{ws}: {name}: {e}");
            let note = AgentNote::Mcp { tool: name.to_string(), text: format!("refused: {}", capped(e)) };
            let _ = self.with(&a.id, |a| a.note(note));
        }
        r
    }

    /// What it says is shown, and nothing else.
    async fn tool_now(&self, ws: &str, name: &str, args: &Value) -> Result<String, String> {
        if !self.docs.wadspace(ws).is_some_and(|w| w.agents) {
            return Err("This wadspace doesn't let agents message the desktop".into());
        }
        let text = |k: &str| args[k].as_str().map(capped).unwrap_or_default();
        let Some(agent) = self.live_on(ws) else { return Err("There's no job here".into()) };
        let id = agent.id.clone();
        let log = |t: &str, what: String| {
            let _ = self.with(&id, |a| a.note(AgentNote::Mcp { tool: t.to_string(), text: what }));
        };
        match name {
            "notify" => {
                let (title, body) = (text("title"), text("body"));
                if title.is_empty() {
                    return Err("It needs a title".into());
                }
                log("notify", title.clone());
                self.toast(Level::Info, &title, &body, &id, "agents");
                Ok("Sent".into())
            }
            "progress" => {
                let step = serde_json::from_value::<StepId>(args["step"].clone()).map_err(|_| "No such step")?;
                let state = match args["state"].as_str() {
                    Some("done") => StepState::Done,
                    Some("failed") => StepState::Failed,
                    _ => StepState::Active,
                };
                let now = text("now");
                self.with(&id, |a| {
                    a.set_step(step, state);
                    if !now.is_empty() {
                        a.now = now.clone();
                    }
                })
                .map_err(|e| e.message)?;
                Ok("Noted".into())
            }
            "request_sandbox" => {
                let (title, why) = (text("title"), text("why"));
                log("request_sandbox", title.clone());
                self.with(&id, |a| {
                    a.ask = Some(Ask::Sandbox { title, why });
                    a.now = "Wants a sandbox to try it with you".into();
                })
                .map_err(|e| e.message)?;
                Ok("Asked. End your turn now: you'll be told their answer.".into())
            }
            "set_checklist" => {
                let items: Vec<ItemIn> =
                    serde_json::from_value(args["items"].clone()).map_err(|_| "That isn't a list of items")?;
                let items: Vec<CheckItem> = items
                    .into_iter()
                    .map(|i| CheckItem {
                        id: capped(&i.id),
                        label: capped(&i.label),
                        how: capped(&i.how),
                        done: i.done,
                        verdict: None,
                        note: String::new(),
                        fresh: false,
                    })
                    .collect();
                let sandbox = agent.sandbox.clone().ok_or("It has no sandbox yet: ask for one first")?;
                log("set_checklist", format!("{} items", items.len()));
                self.sandbox(&sandbox, |s| {
                    let build = s.build.max(1);
                    s.set_items(items, build);
                })
                .map_err(|e| e.message)?;
                Ok("Noted".into())
            }
            "update_sandbox" => {
                let sandbox = agent.sandbox.clone().ok_or("It has no sandbox yet: ask for one first")?;
                let build = args["build"].as_u64().unwrap_or(1) as u32;
                let path = args["path"].as_str().filter(|p| !p.is_empty());
                if path.is_some_and(|p| !in_home(p)) {
                    return Err("`path` is a folder under this wadspace's home".into());
                }
                let target = self.state().sandboxes.iter().find(|s| s.id == sandbox).map(|s| s.ws.clone());
                let target = target.ok_or("The sandbox went")?;
                self.hands.update(ws, &target, path, args["run"].as_str()).await?;
                log("update_sandbox", format!("build {build}"));
                self.sandbox(&sandbox, |s| {
                    s.build = build;
                    s.state = SandboxState::Ready;
                })
                .map_err(|e| e.message)?;
                let title = format!("Build {build} is in your sandbox");
                self.toast(Level::Info, &title, &text_line(&agent.task), &sandbox, "sandbox");
                Ok("It's in. End your turn: the person tries it now.".into())
            }
            other => Err(format!("No tool {other}")),
        }
    }
}

/// A short line of `text`.
fn text_line(text: &str) -> String {
    text.lines().next().unwrap_or_default().chars().take(120).collect()
}

async fn connection(me: Weak<MachineAgents>, ws: String, sock: tokio::net::UnixStream) {
    let (read, mut write) = sock.into_split();
    let mut lines = BufReader::new(read).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.len() > 1 << 20 {
            return;
        }
        let answer = match mcp::read(&line) {
            mcp::Call::Answer(a) => a,
            mcp::Call::Nothing => continue,
            mcp::Call::Tool { id, name, args } => {
                let Some(m) = me.upgrade() else { return };
                mcp::answer(id, m.tool(&ws, &name, &args).await)
            }
        };
        let mut text = answer.to_string();
        text.push('\n');
        if write.write_all(text.as_bytes()).await.is_err() {
            return;
        }
    }
}

/// The machine's: Claude Code by `podman exec`.
pub struct Exec;

#[async_trait]
impl Runner for Exec {
    async fn run(&self, ws: &str, args: &[String], on_line: &mut OnLine<'_>) -> Result<(), String> {
        let name = wad_gen::units::container_name(ws);
        let mut argv: Vec<String> =
            ["exec", "--user", "abc", "--workdir", "/config", "-e", "HOME=/config"].map(String::from).to_vec();
        argv.extend([name, "claude".to_string()]);
        argv.extend_from_slice(args);
        crate::updates::podman(&argv, on_line).await.map(drop)
    }

    async fn interrupt(&self, ws: &str) {
        let name = wad_gen::units::container_name(ws);
        let _ = podman(&["exec", "--user", "abc", &name, "pkill", "-INT", "-x", "claude"]).await;
    }
}

/// The machine's sandboxes.
pub struct RealHands {
    pub docs: Arc<Docs>,
    pub builds: Arc<crate::builds::Builds>,
    pub runs: Arc<crate::runs::Runs>,
}

fn op(v: Value) -> Result<wad_model::Op, String> {
    serde_json::from_value(v).map_err(|e| e.to_string())
}

async fn podman(args: &[&str]) -> Result<(), String> {
    let argv: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    crate::updates::podman(&argv, &mut |_| {}).await.map(drop)
}

#[async_trait]
impl Hands for RealHands {
    async fn open(&self, from: &str, task: &str) -> Result<String, String> {
        let name = format!("Sandbox · {}", text_line(task).chars().take(40).collect::<String>());
        let made =
            self.docs.apply(op(serde_json::json!({ "op": "space.create", "name": name }))?).map_err(|e| e.message)?;
        let ws = made.created.ok_or("no wadspace was made")?;
        self.docs.apply(wad_model::Op::Seal { ws: ws.clone() }).map_err(|e| e.message)?;
        // What it runs, with none of what it's kept: no keys and no projects.
        let apps = self.docs.wadspace(from).map(|w| w.apps).unwrap_or_default();
        for a in apps {
            let add = serde_json::json!({ "op": "apps.add", "ws": ws, "app": a.app });
            self.docs.apply(op(add)?).map_err(|e| e.message)?;
        }
        self.builds.request(&ws).await.map_err(|e| e.message)?;
        for _ in 0..1800 {
            let doc = self.docs.wadspace(&ws).ok_or("the wadspace went")?;
            if self.builds.image_for(&doc).is_some() {
                self.runs.open(&ws, SessionDisplay::Window).map_err(|e| e.message)?;
                self.runs.settled(&ws).await;
                return Ok(ws);
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        self.close(&ws).await;
        Err("The sandbox didn’t get built".into())
    }

    async fn update(&self, from: &str, sandbox: &str, path: Option<&str>, run: Option<&str>) -> Result<(), String> {
        let (a, b) = (wad_gen::units::container_name(from), wad_gen::units::container_name(sandbox));
        if let Some(p) = path {
            // By tar, as each wadspace's own user: `podman cp` can't write
            // into an id-mapped /config.
            let from_dir = shell_word(&format!("/config/{p}"));
            let script = format!(
                "set -o pipefail; podman exec --user abc {a} tar -C {from_dir} -cf - . | podman exec -i --user abc {b} sh -c 'mkdir -p /config/build && tar -C /config/build -xf -'"
            );
            crate::updates::outside(&["bash", "-c", &script], &mut |_| {}).await?;
        }
        if let Some(cmd) = run {
            // Its own desktop's environment, and left running.
            let script = format!(
                "export $(tr '\\0' '\\n' < /proc/$(pgrep -nx labwc)/environ | grep -E '^(WAYLAND_DISPLAY|XDG_RUNTIME_DIR)='); cd /config/build 2>/dev/null; nohup sh -c {} > /tmp/sandbox.log 2>&1 &",
                shell_word(cmd)
            );
            podman(&["exec", "--user", "abc", &b, "sh", "-c", &script]).await?;
        }
        Ok(())
    }

    async fn close(&self, sandbox: &str) {
        let _ = self.runs.stop(sandbox).await;
        let _ = self.docs.apply(wad_model::Op::Trash { ws: sandbox.to_string() });
        let _ = self.docs.purge(sandbox);
    }

    async fn ensure_up(&self, ws: &str) -> Result<(), String> {
        let phase = self.runs.state(ws).state.phase;
        if !matches!(phase, wad_model::Phase::Ready) {
            self.runs.open(ws, SessionDisplay::Background).map_err(|e| e.message)?;
            self.runs.settled(ws).await;
        }
        Ok(())
    }
}

/// `text`, quoted for sh.
pub fn shell_word(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests;
