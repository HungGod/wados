//! Long jobs (a wadspace's build), as wad_model's [`Job`]: its stages, its
//! progress, its log. The last [`MAX_LINES`] lines are kept in memory and
//! every line goes to `<state_dir>/jobs/<id>.log` (v1's joblog). Watchers
//! hear each job's state (`job` events, at most every [`PUBLISH_EVERY`]
//! unless something changed that matters: a stage, its status) and its
//! lines as they come (`jobLines`). The last [`KEEP_FINISHED`] finished
//! jobs are kept; older ones are forgotten (`jobGone`), and each one's log
//! goes with it. A job isn't loaded back after a restart, so logs older
//! than [`KEEP_LOGS_FOR`] are swept when wadd starts.
//!
//! Cancelling a job drops its work (its task is aborted): podman's build
//! stream closes, which stops the build.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use wad_model::{Job, JobKind, JobStage, JobStatus, PartState, Source};
use wad_proto::{ApiError, Event, JobInfo, JobLog};

use crate::events::Bus;

pub const MAX_LINES: usize = 4000;
/// Lines a job's state carries (the rest by `GET /v1/jobs/{id}`).
pub const TAIL: usize = 12;
pub const KEEP_FINISHED: usize = 20;
/// How long a job's log file is kept after wadd stops (they're never loaded
/// back, so it's for reading, not for wadd).
pub const KEEP_LOGS_FOR: Duration = Duration::from_secs(7 * 24 * 3600);
pub const PUBLISH_EVERY: Duration = Duration::from_millis(250);

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// A job's log: the last lines in memory, every line in its file.
struct Lines {
    lines: Vec<String>,
    dropped: u64,
    file: PathBuf,
}

impl Lines {
    fn push(&mut self, text: &str) {
        self.lines.push(text.to_string());
        if self.lines.len() > MAX_LINES {
            let cut = self.lines.len() - MAX_LINES;
            self.lines.drain(..cut);
            self.dropped += cut as u64;
        }
        if let Some(d) = self.file.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&self.file) {
            let _ = writeln!(f, "{text}");
        }
    }

    fn total(&self) -> u64 {
        self.dropped + self.lines.len() as u64
    }

    fn since(&self, n: u64) -> Vec<String> {
        let skip = n.saturating_sub(self.dropped) as usize;
        self.lines.iter().skip(skip).cloned().collect()
    }
}

struct Entry {
    job: Job,
    log: Lines,
    /// Lines watchers have had.
    sent: u64,
    published: Option<Instant>,
    /// Its state changed since it was last published.
    dirty: bool,
    task: Option<tokio::task::AbortHandle>,
}

impl Entry {
    fn info(&self) -> JobInfo {
        let mut job = self.job.clone();
        let total = self.log.total();
        job.log = self.log.since(total.saturating_sub(TAIL as u64));
        JobInfo { job, line_count: total }
    }
}

pub struct Jobs {
    bus: Bus,
    dir: PathBuf,
    entries: Mutex<Vec<Entry>>,
    seq: AtomicU64,
}

pub fn finished(s: JobStatus) -> bool {
    matches!(s, JobStatus::Done | JobStatus::Error | JobStatus::Cancelled)
}

impl Jobs {
    pub fn new(bus: Bus, state_dir: &Path) -> Arc<Self> {
        let dir = state_dir.join("jobs");
        // Nothing is loaded back, so a log from a run that's over can't be
        // read again: keep the ones KEEP_LOGS_FOR old, and let the rest go.
        sweep(&dir);
        Arc::new(Self { bus, dir, entries: Mutex::default(), seq: AtomicU64::new(0) })
    }

    /// A new job, queued: its stages (id, label) waiting. Its id.
    pub fn create(&self, kind: JobKind, title: &str, subject: &str, stages: &[(String, String)]) -> String {
        let n = self.seq.fetch_add(1, Ordering::Relaxed);
        let id = format!("job-{:x}{n:03x}", now_ms());
        let job = Job {
            id: id.clone(),
            kind,
            status: JobStatus::Queued,
            title: title.to_string(),
            subject: Some(subject.to_string()),
            params: Default::default(),
            stages: stages
                .iter()
                .map(|(id, label)| JobStage {
                    id: id.clone(),
                    label: label.clone(),
                    state: PartState::Waiting,
                    bytes: 0,
                    done: 0,
                    lines: vec![],
                    held: false,
                    error: None,
                })
                .collect(),
            cursor: 0,
            progress: 0.0,
            done_bytes: 0,
            total_bytes: 0,
            log: vec![],
            error: None,
            started_at: now_ms(),
            ended_at: None,
            source: Source::Machine,
            visible: true,
        };
        let log = Lines { lines: vec![], dropped: 0, file: self.dir.join(format!("{id}.log")) };
        self.entries.lock().unwrap().push(Entry { job, log, sent: 0, published: None, dirty: true, task: None });
        self.trim();
        self.publish(&id, true);
        id
    }

    /// The work doing it (aborted if it's cancelled).
    pub fn attach(&self, id: &str, task: tokio::task::AbortHandle) {
        if let Some(e) = self.entries.lock().unwrap().iter_mut().find(|e| e.job.id == id) {
            e.task = Some(task);
        }
    }

    /// Changes it; `force`: watchers hear it now (else within PUBLISH_EVERY).
    pub fn update(&self, id: &str, force: bool, f: impl FnOnce(&mut Job)) {
        {
            let mut entries = self.entries.lock().unwrap();
            let Some(e) = entries.iter_mut().find(|e| e.job.id == id) else { return };
            // A job that's over stays as it ended.
            if finished(e.job.status) {
                return;
            }
            f(&mut e.job);
            e.dirty = true;
        }
        self.publish(id, force);
    }

    /// One stage's state (and the job's cursor on it while it works).
    pub fn stage(&self, id: &str, stage: &str, state: PartState, error: Option<String>) {
        self.update(id, true, |j| {
            if let Some(i) = j.stages.iter().position(|s| s.id == stage) {
                j.stages[i].state = state;
                j.stages[i].error = error;
                if state == PartState::Working {
                    j.cursor = i;
                }
            }
            j.progress = j.progress.max(progress_of(j, 0.0));
        });
    }

    /// How far into the stage it's on (0 to 1): the job's progress.
    pub fn progress(&self, id: &str, stage: &str, part: f64) {
        self.update(id, false, |j| {
            if let Some(i) = j.stages.iter().position(|s| s.id == stage) {
                j.cursor = i;
            }
            j.progress = j.progress.max(progress_of(j, part.clamp(0.0, 1.0)));
        });
    }

    /// A download's bytes, on its stage.
    pub fn bytes(&self, id: &str, stage: &str, done: u64, total: u64) {
        self.update(id, false, |j| {
            if let Some(s) = j.stages.iter_mut().find(|s| s.id == stage) {
                s.done = done;
                s.bytes = total;
            }
            j.done_bytes = j.stages.iter().map(|s| s.done).sum();
            j.total_bytes = j.stages.iter().map(|s| s.bytes).sum();
        });
        let part = if total > 0 { done as f64 / total as f64 } else { 0.0 };
        self.progress(id, stage, part);
    }

    pub fn line(&self, id: &str, text: &str) {
        if let Some(e) = self.entries.lock().unwrap().iter_mut().find(|e| e.job.id == id) {
            e.log.push(text);
        }
        self.publish(id, false);
    }

    /// It's running (its turn came).
    pub fn running(&self, id: &str) {
        self.update(id, true, |j| j.status = JobStatus::Running);
    }

    /// It ended: done, or failed with why (cancelled ones end in `cancel`).
    pub fn finish(&self, id: &str, result: Result<(), String>) {
        let line = match &result {
            Ok(()) => None,
            Err(e) => Some(format!("✗ {e}")),
        };
        if let Some(l) = &line {
            self.line(id, l);
        }
        self.update(id, true, |j| {
            j.ended_at = Some(now_ms());
            match result {
                Ok(()) => {
                    j.status = JobStatus::Done;
                    j.progress = 1.0;
                    for s in &mut j.stages {
                        if s.state == PartState::Waiting || s.state == PartState::Working {
                            s.state = PartState::Done;
                        }
                    }
                }
                Err(e) => {
                    j.status = JobStatus::Error;
                    for s in &mut j.stages {
                        if s.state == PartState::Working {
                            s.state = PartState::Error;
                        } else if s.state == PartState::Waiting {
                            s.state = PartState::Skipped;
                        }
                    }
                    j.error = Some(e);
                }
            }
        });
        if let Some(e) = self.entries.lock().unwrap().iter_mut().find(|e| e.job.id == id) {
            e.task = None;
        }
        self.flush();
    }

    /// Stops it, if it's still going.
    pub fn cancel(&self, id: &str) -> Result<JobInfo, ApiError> {
        let task = {
            let mut entries = self.entries.lock().unwrap();
            let e = entries.iter_mut().find(|e| e.job.id == id).ok_or_else(|| no_job(id))?;
            if finished(e.job.status) {
                return Ok(e.info());
            }
            e.task.take()
        };
        if let Some(t) = task {
            t.abort();
        }
        self.line(id, "✗ cancelled");
        self.update(id, true, |j| {
            j.status = JobStatus::Cancelled;
            j.ended_at = Some(now_ms());
            for s in &mut j.stages {
                if matches!(s.state, PartState::Working | PartState::Waiting) {
                    s.state = PartState::Skipped;
                }
            }
        });
        self.flush();
        self.get(id, 0).map(|l| l.job)
    }

    /// The jobs, newest first.
    pub fn list(&self) -> Vec<JobInfo> {
        self.entries.lock().unwrap().iter().rev().map(Entry::info).collect()
    }

    pub fn info(&self, id: &str) -> Option<JobInfo> {
        self.entries.lock().unwrap().iter().find(|e| e.job.id == id).map(Entry::info)
    }

    /// A job and its lines from `since` on.
    pub fn get(&self, id: &str, since: u64) -> Result<JobLog, ApiError> {
        let entries = self.entries.lock().unwrap();
        let e = entries.iter().find(|e| e.job.id == id).ok_or_else(|| no_job(id))?;
        Ok(JobLog { job: e.info(), from: since, lines: e.log.since(since) })
    }

    /// The job not finished whose `params[key]` is `value` (one set's build).
    pub fn running_with(&self, key: &str, value: &str) -> Option<String> {
        self.entries
            .lock()
            .unwrap()
            .iter()
            .find(|e| !finished(e.job.status) && e.job.params.get(key).and_then(|v| v.as_str()) == Some(value))
            .map(|e| e.job.id.clone())
    }

    /// Watchers hear this job (and any other with lines not sent yet), at
    /// most every PUBLISH_EVERY unless `force`.
    fn publish(&self, id: &str, force: bool) {
        let mut out = vec![];
        {
            let mut entries = self.entries.lock().unwrap();
            let now = Instant::now();
            for e in entries.iter_mut() {
                if !e.dirty && e.sent == e.log.total() {
                    continue;
                }
                let due = e.published.is_none_or(|t| now - t >= PUBLISH_EVERY);
                if !due && !(force && e.job.id == id) {
                    continue;
                }
                let total = e.log.total();
                if total > e.sent {
                    out.push(Event::JobLines { id: e.job.id.clone(), from: e.sent, lines: e.log.since(e.sent) });
                    e.sent = total;
                }
                out.push(Event::Job { job: Box::new(e.info()) });
                e.dirty = false;
                e.published = Some(now);
            }
        }
        for ev in out {
            self.bus.publish(ev);
        }
    }

    /// Everything not heard yet, now.
    pub fn flush(&self) {
        let ids: Vec<String> = self
            .entries
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.dirty || e.sent < e.log.total())
            .map(|e| e.job.id.clone())
            .collect();
        for id in ids {
            self.publish(&id, true);
        }
    }

    /// Flushes every PUBLISH_EVERY, so a quiet job's last lines get out.
    pub async fn run(self: Arc<Self>) {
        loop {
            tokio::time::sleep(PUBLISH_EVERY).await;
            self.flush();
        }
    }

    /// Keeps the last KEEP_FINISHED finished jobs.
    fn trim(&self) {
        let gone: Vec<String> = {
            let mut entries = self.entries.lock().unwrap();
            let done: Vec<String> =
                entries.iter().filter(|e| finished(e.job.status)).map(|e| e.job.id.clone()).collect();
            let cut = done.len().saturating_sub(KEEP_FINISHED);
            let gone = done[..cut].to_vec();
            entries.retain(|e| !gone.contains(&e.job.id));
            gone
        };
        for id in gone {
            // Nothing can ask for it now: it's out of the API.
            let _ = std::fs::remove_file(self.dir.join(format!("{id}.log")));
            self.bus.forget(&format!("job:{id}"));
            self.bus.publish(Event::JobGone { id });
        }
    }
}

/// Each stage its share of the bar; `part` of the one at the cursor.
fn progress_of(j: &Job, part: f64) -> f64 {
    let n = j.stages.len().max(1) as f64;
    let done = j.stages.iter().filter(|s| matches!(s.state, PartState::Done | PartState::Skipped)).count() as f64;
    let working = j.stages.get(j.cursor).is_some_and(|s| s.state == PartState::Working);
    ((done + if working { part } else { 0.0 }) / n).min(0.99)
}

fn no_job(id: &str) -> ApiError {
    ApiError::not_found(format!("no job {id:?}"))
}

/// Drops the logs older than [`KEEP_LOGS_FOR`] that a previous run left.
/// Only `<id>.log` files in `dir` are touched; a machine that checks for
/// updates every 15 minutes makes two of them a cycle, and nothing would
/// ever remove the ones already finished when wadd last stopped.
fn sweep(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let Some(cutoff) = SystemTime::now().checked_sub(KEEP_LOGS_FOR) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("log") {
            continue;
        }
        let old = e.metadata().and_then(|m| m.modified()).is_ok_and(|m| m < cutoff);
        if old {
            let _ = std::fs::remove_file(&p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wad_proto::{MachineInfo, Profile};

    fn jobs() -> (tempfile::TempDir, Arc<Jobs>, Bus) {
        let d = tempfile::tempdir().unwrap();
        let bus =
            Bus::new(MachineInfo { id: "m".into(), name: "M".into(), version: "0".into(), profile: Profile::User });
        let jobs = Jobs::new(bus.clone(), d.path());
        (d, jobs, bus)
    }

    #[test]
    fn a_job_goes_through_its_stages_and_its_log_is_kept() {
        let (d, jobs, bus) = jobs();
        let (_, mut rx) = bus.subscribe();
        let id = jobs.create(
            JobKind::Build,
            "Building Studio",
            "studio",
            &[("a".into(), "A".into()), ("b".into(), "B".into())],
        );
        jobs.running(&id);
        jobs.stage(&id, "a", PartState::Working, None);
        for i in 0..20 {
            jobs.line(&id, &format!("line {i}"));
        }
        jobs.stage(&id, "a", PartState::Done, None);
        jobs.finish(&id, Ok(()));
        let info = jobs.info(&id).unwrap();
        assert_eq!(info.job.status, JobStatus::Done);
        assert_eq!(info.line_count, 20);
        assert_eq!(info.job.log.len(), TAIL);
        assert!(info.job.stages.iter().all(|s| s.state == PartState::Done));
        assert_eq!(jobs.get(&id, 18).unwrap().lines, ["line 18", "line 19"]);
        assert_eq!(std::fs::read_to_string(d.path().join(format!("jobs/{id}.log"))).unwrap().lines().count(), 20);
        // Watchers heard every line once, in order, and the end.
        let mut heard = vec![];
        let mut last = None;
        while let Ok(e) = rx.try_recv() {
            match e {
                Event::JobLines { from, lines, .. } => {
                    assert_eq!(from as usize, heard.len());
                    heard.extend(lines);
                }
                Event::Job { job } => last = Some(job.job.status),
                _ => {}
            }
        }
        assert_eq!(heard.len(), 20);
        assert_eq!(last, Some(JobStatus::Done));
        // A finished job isn't changed or cancelled any more.
        assert_eq!(jobs.cancel(&id).unwrap().job.status, JobStatus::Done);
    }

    #[tokio::test]
    async fn a_cancelled_job_stops_its_work() {
        let (_d, jobs, _bus) = jobs();
        let id = jobs.create(JobKind::Build, "B", "b", &[("a".into(), "A".into())]);
        let work = tokio::spawn(std::future::pending::<()>());
        jobs.attach(&id, work.abort_handle());
        jobs.stage(&id, "a", PartState::Working, None);
        let info = jobs.cancel(&id).unwrap();
        assert_eq!(info.job.status, JobStatus::Cancelled);
        assert_eq!(info.job.stages[0].state, PartState::Skipped);
        assert!(work.await.unwrap_err().is_cancelled());
        assert!(jobs.cancel("nope").is_err());
    }

    #[test]
    fn only_the_last_finished_jobs_are_kept() {
        let (_d, jobs, _bus) = jobs();
        let first = jobs.create(JobKind::Build, "B", "b", &[]);
        jobs.line(&first, "its line");
        jobs.finish(&first, Err("no".into()));
        let first_log = _d.path().join(format!("jobs/{first}.log"));
        assert!(first_log.exists());
        for _ in 0..KEEP_FINISHED {
            let id = jobs.create(JobKind::Build, "B", "b", &[]);
            jobs.finish(&id, Ok(()));
        }
        jobs.create(JobKind::Build, "B", "b", &[]);
        assert!(jobs.info(&first).is_none());
        assert_eq!(jobs.list().len(), KEEP_FINISHED + 1);
        // Its log went with it; the ones kept still have theirs.
        assert!(!first_log.exists(), "a forgotten job's log is removed");
    }

    #[test]
    fn old_logs_are_swept_when_wadd_starts() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("jobs");
        std::fs::create_dir_all(&dir).unwrap();
        let log = |name: &str| {
            let p = dir.join(name);
            std::fs::write(&p, "x\n").unwrap();
            p
        };
        let (old, kept, other) = (log("job-old.log"), log("job-new.log"), log("notes.txt"));
        let before = SystemTime::now().checked_sub(KEEP_LOGS_FOR + Duration::from_secs(60)).unwrap();
        std::fs::File::options().write(true).open(&old).unwrap().set_modified(before).unwrap();
        let bus =
            Bus::new(MachineInfo { id: "m".into(), name: "M".into(), version: "0".into(), profile: Profile::User });
        let _jobs = Jobs::new(bus, d.path());
        assert!(!old.exists(), "older than KEEP_LOGS_FOR");
        assert!(kept.exists(), "recent");
        assert!(other.exists(), "not a job log");
    }
}
