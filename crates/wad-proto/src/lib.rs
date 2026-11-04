//! wadd's API (`/v1` on `/run/wadd/wadd.sock`) as types: what the shell, the
//! machine's apps and (over the tailnet) the phone send and get back.
//!
//! The documents themselves are `wad_model`'s: a [`wad_model::Wadspace`] goes
//! over the wire as it's saved. Edits are [`wad_model::edit::Op`]s, applied
//! by wadd (`POST /v1/ops`), so every surface edits the same way.

mod error;
pub mod github;
pub mod peers;
pub mod signin;
pub mod streams;
pub mod tailnet;

pub use error::{ApiError, ErrorCode};
pub use github::{GithubAccount, Repo, SignInRequest};
pub use peers::PeerInfo;
pub use signin::SignInProvider;
pub use streams::{Availability, StreamInfo, StreamList, StreamRequest, StreamShare, StreamsAvailability};
pub use tailnet::{
    Answer, DeviceKind, DriveState, RemoteHello, RemoteMachine, RemoteTrust, Tailnet, TailnetAsk, TailnetDevice,
    TailnetState, TrustAnswer, TrustBook,
};

/// The edits themselves (wad_model's), for callers that only depend on wad-proto.
pub mod wad_model_op {
    pub use wad_model::edit::{Library, Op, Payload};
}

/// The documents and machines (wad_model's), for callers that only depend on wad-proto.
pub mod wad_model_doc {
    pub use wad_model::{Peer, PeerKind, Wadspace};
}

/// What builds, runs and jobs are made of (wad_model's), for callers that only depend on wad-proto.
pub mod wad_model_run {
    pub use wad_model::{BuildState, Built, Job, JobStage, JobStatus, PartState, Phase, RunState, SessionDisplay};
}

use serde::{Deserialize, Serialize};
use wad_model::edit::{Library, Op};
use wad_model::{BuildState, Job, RunState, SessionDisplay, Wadspace};

/// Whose wadd this is: a machine's (root, the system's podman) or a laptop's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    System,
    User,
}

/// The machine wadd runs on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineInfo {
    /// Its stable id (`<id>.wadspaces.com`).
    pub id: String,
    pub name: String,
    /// wadd's version.
    pub version: String,
    pub profile: Profile,
}

/// `POST /v1/ops`: one edit, as the spike's actions are.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpRequest {
    pub op: Op,
}

/// What an edit did (`POST /v1/ops`, `POST /v1/undo/{id}`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpResult {
    /// The wadspaces it changed.
    pub changed: Vec<String>,
    /// What it made (a wadspace, a tag, a key, a command…), by id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    /// Its Undo, for the toast's button (`POST /v1/undo/{id}`), while wadd keeps it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undo: Option<String>,
    /// What to tell the person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toast: Option<String>,
    /// What a changed wadspace's image is made of changed (it's out of date once built).
    pub needs_build: bool,
}

/// A line of wadd's log (`GET /v1/logs`), secrets taken out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogLine {
    /// Milliseconds since the epoch.
    pub time: u64,
    pub level: String,
    pub target: String,
    pub message: String,
}

/// How a toast looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Info,
    Ok,
    Warn,
    Error,
}

/// What changed, on `GET /v1/events` (server-sent events, one JSON each). A
/// new watcher gets the present first: the machine, the library, every
/// wadspace's state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Event {
    Machine(MachineInfo),
    /// Everything the machine keeps: its wadspaces, tags, keys, web apps.
    Library(Box<Library>),
    /// One wadspace's document, as it is now.
    Wadspace(Box<Wadspace>),
    /// A wadspace deleted for good (out of the Trash).
    WadspaceGone {
        id: String,
    },
    /// Something to tell the person (a toast), with an Undo when it took something out.
    Notice {
        level: Level,
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        undo: Option<String>,
    },
    /// A job (a wadspace's build) as it is now: its stages, progress and the last of its log.
    /// Nested under `job`: a job has a `kind` of its own (build…), which would clash with the tag.
    Job {
        job: Box<JobInfo>,
    },
    /// Lines a job printed, from line `from` on (`GET /v1/jobs/{id}?since=` has them all).
    JobLines {
        id: String,
        from: u64,
        lines: Vec<String>,
    },
    /// A finished job forgotten (only the last few are kept).
    JobGone {
        id: String,
    },
    /// A wadspace's image on this machine: built, out of date, or never.
    Build(Box<WadspaceBuild>),
    /// A wadspace's run on this machine: how far it is, and how it's shown.
    Run(Box<WadspaceRun>),
    /// The screen wadd puts wadspaces on (the compositor it found, or none).
    Screen(ScreenInfo),
    /// The tailnet as it is now: this machine on it, its devices, the ones asking.
    Tailnet(Tailnet),
    /// My machines on the tailnet, as `GET /v1/peers` has them (nested: a list can't sit beside the tag).
    Peers {
        peers: Vec<PeerInfo>,
    },
    /// The GitHub account wadd has a token for; none: signed out (nested, as `peers`).
    Github {
        account: Option<GithubAccount>,
    },
    /// A sign-in waiting in the browser: the shell opens `url` in WadBrowser
    /// (`code`: the one to type there, GitHub's device code). The present until its `signInDone`.
    SignIn {
        provider: SignInProvider,
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<String>,
    },
    /// That sign-in is over (signed in, failed or cancelled): its window closes.
    SignInDone {
        provider: SignInProvider,
    },
    /// The streams here and which modes can start, as `GET /v1/streams` has
    /// them ([`StreamList`]'s fields: nested, as `peers`). Never a key or password.
    Streams {
        streams: Vec<StreamInfo>,
        available: StreamsAvailability,
    },
}

impl Event {
    /// Events whose latest is the present (a new watcher gets it): by this key.
    pub fn latest_key(&self) -> Option<String> {
        match self {
            Event::Machine(_) => Some("machine".into()),
            Event::Job { job } => Some(format!("job:{}", job.job.id)),
            Event::Build(b) => Some(format!("build:{}", b.id)),
            Event::Run(r) => Some(format!("run:{}", r.id)),
            Event::Screen(_) => Some("screen".into()),
            Event::Tailnet(_) => Some("tailnet".into()),
            Event::Peers { .. } => Some("peers".into()),
            Event::Github { .. } => Some("github".into()),
            Event::SignIn { provider, .. } => Some(format!("signIn:{provider}")),
            Event::Streams { .. } => Some("streams".into()),
            Event::Library(_)
            | Event::Wadspace(_)
            | Event::WadspaceGone { .. }
            | Event::Notice { .. }
            | Event::JobLines { .. }
            | Event::JobGone { .. }
            | Event::SignInDone { .. } => None,
        }
    }

    /// The present this event ends (a new watcher doesn't get it any more): a sign-in's, when it's done.
    pub fn ends_key(&self) -> Option<String> {
        match self {
            Event::SignInDone { provider } => Some(format!("signIn:{provider}")),
            _ => None,
        }
    }
}

// ------------------------------------------------------------ jobs

/// A job as it is now (`GET /v1/jobs`, the `job` event): wad_model's
/// [`Job`] with only the last of its log in `log`, and how many lines it
/// has printed in all.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobInfo {
    #[serde(flatten)]
    pub job: Job,
    pub line_count: u64,
}

/// `GET /v1/jobs/{id}?since=n`: a job, and its lines from line `from` on
/// (as many as wadd still keeps in memory; the rest are in its log file).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobLog {
    pub job: JobInfo,
    pub from: u64,
    pub lines: Vec<String>,
}

// ------------------------------------------------------------ builds

/// A wadspace's image on this machine (the `build` event, `GET /v1/builds`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WadspaceBuild {
    pub id: String,
    /// Built, update (it runs the image it built last meanwhile) or never; and that image's size.
    #[serde(flatten)]
    pub state: BuildState,
    /// What its image is made of now: `set:<key>`, or `base` (the base as it is).
    pub layer: String,
    /// The image it runs when it's opened (none: it can't be, yet).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// The build making its image, while one is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<String>,
    /// What it has that isn't packaged for Fedora yet: its image is built without these.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unpackaged: Vec<String>,
}

/// `POST /v1/wadspaces/{id}/build`: its image as it is now, and the build
/// started (or the one already making it); none when it's built already.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildAnswer {
    pub state: WadspaceBuild,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<JobInfo>,
}

// ------------------------------------------------------------ runs

/// `POST /v1/wadspaces/{id}/open`: how it's shown. `screen` (full screen),
/// `window` or `background` (started, its window minimized): the shell
/// places it. `stream` is a later phase's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenRequest {
    pub display: SessionDisplay,
}

/// A wadspace's run on this machine (the `run` event, `GET /v1/runs`):
/// wad_model's [`RunState`] (idle, starting, waiting for its desktop,
/// ready, stopping, error; since when; what it's doing or what went wrong)
/// and how it's shown, which the shell follows to place its window (full
/// screen, a window, minimized). Wadd never places windows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WadspaceRun {
    pub id: String,
    #[serde(flatten)]
    pub state: RunState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<SessionDisplay>,
}

// ------------------------------------------------------------ the screen

/// `PUT /v1/screen/session`: the shell, as it starts, tells wadd its
/// compositor: its Wayland socket (an absolute path) and whose session it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenSession {
    pub wayland_socket: String,
    pub uid: u32,
}

/// Where wadd found the compositor it serves wadspaces on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScreenFrom {
    /// The shell said (`PUT /v1/screen/session`).
    Shell,
    /// `[display] socket` in wadd's configuration.
    Config,
    /// The newest `wayland-*` in `[display] runtime_dir`.
    RuntimeDir,
    /// Nowhere: no wadspace can be shown.
    None,
}

/// The screen (the `screen` event, `GET /v1/screen`): the compositor wadd
/// serves wadspaces on, and the wadspaces it serves now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub socket: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uid: Option<u32>,
    pub from: ScreenFrom,
    /// Wadspaces with a socket of their own on it now.
    pub served: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use wad_model::{Built, Phase};

    #[test]
    fn builds_runs_and_the_screen_are_the_present_by_wadspace() {
        let run = WadspaceRun {
            id: "studio".into(),
            state: RunState { phase: Phase::Ready, since: 5, ..Default::default() },
            display: Some(SessionDisplay::Window),
        };
        let e = Event::Run(Box::new(run.clone()));
        let json = serde_json::to_value(&e).unwrap();
        // Flat: the shell reads phase and display side by side.
        assert_eq!(json["kind"], "run");
        assert_eq!(json["phase"], "ready");
        assert_eq!(json["display"], "window");
        assert_eq!(serde_json::from_value::<Event>(json).unwrap(), e);
        assert_eq!(e.latest_key().as_deref(), Some("run:studio"));

        let build = WadspaceBuild {
            id: "studio".into(),
            state: BuildState { built: Built::Update, size_mb: 1200 },
            layer: "set:0123456789ab".into(),
            image: Some("localhost/wados-set-00aa00aa00aa:latest".into()),
            job: None,
            unpackaged: vec![],
        };
        let json = serde_json::to_value(Event::Build(Box::new(build.clone()))).unwrap();
        assert_eq!((json["built"].as_str(), json["sizeMb"].as_u64()), (Some("update"), Some(1200)));
        assert_eq!(Event::Build(Box::new(build)).latest_key().as_deref(), Some("build:studio"));

        let screen = Event::Screen(ScreenInfo { socket: None, uid: None, from: ScreenFrom::None, served: vec![] });
        assert_eq!(screen.latest_key().as_deref(), Some("screen"));
        // A job's lines aren't the present: its state is.
        assert_eq!(Event::JobLines { id: "j".into(), from: 0, lines: vec![] }.latest_key(), None);
        let open: OpenRequest = serde_json::from_str(r#"{"display": "background"}"#).unwrap();
        assert_eq!(open.display, SessionDisplay::Background);
        let s: ScreenSession =
            serde_json::from_str(r#"{"waylandSocket": "/run/user/1000/wayland-0", "uid": 1000}"#).unwrap();
        assert_eq!(s.uid, 1000);
    }

    #[test]
    fn a_job_event_comes_back_as_one() {
        // A job has a `kind` of its own; the event's tag mustn't meet it.
        let job: Job = serde_json::from_value(serde_json::json!({
            "id": "j1", "kind": "build", "status": "running", "title": "Building Studio",
            "subject": "studio", "progress": 0.5, "startedAt": 10, "source": "machine", "visible": true,
        }))
        .unwrap();
        let e = Event::Job { job: Box::new(JobInfo { job, line_count: 3 }) };
        let json = serde_json::to_string(&e).unwrap();
        assert_eq!(serde_json::from_str::<Event>(&json).unwrap(), e);
        assert_eq!(e.latest_key().as_deref(), Some("job:j1"));
    }
}
