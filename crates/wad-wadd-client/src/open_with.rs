//! Open with, as the shell and Files both do it (the spike's
//! `files.openWith`): files opened with an app, in a wadspace that has it or
//! a new one made for it (`space.forApp`), built first if it has to be,
//! then opened on the screen with them for this run (`POST …/open` with a
//! `file` each).

use std::time::Duration;

use wad_model::edit::Op;
use wad_model::{AppId, JobStatus, SessionDisplay};
use wad_proto::WadspaceRun;

use crate::{Failure, Wadd};

/// How long a build may take before Open with gives up waiting.
const BUILD_WAIT: Duration = Duration::from_secs(3600);

fn failed(message: String) -> Failure {
    Failure { status: 409, code: "conflict".into(), message, still_open: vec![] }
}

impl Wadd {
    /// `files` (paths on this machine, or `machine`'s) opened with `app` in
    /// `ws`, or in a new wadspace for it (none). Its id and its run.
    pub async fn open_files_with(
        &self,
        app: AppId,
        ws: Option<&str>,
        files: &[String],
        machine: Option<&str>,
    ) -> Result<(String, WadspaceRun), Failure> {
        let id = match ws {
            Some(w) => w.to_string(),
            None => self
                .op(Op::ForApp { app })
                .await?
                .created
                .ok_or_else(|| failed("wadd made no wadspace for it".into()))?,
        };
        self.built(&id).await?;
        let mut run = None;
        for f in files {
            run = Some(self.open_with(&id, SessionDisplay::Screen, f, machine).await?);
        }
        let run = match run {
            Some(r) => r,
            None => self.open(&id, SessionDisplay::Screen).await?,
        };
        Ok((id, run))
    }

    /// Until `id` has an image to run (built now, if it has none).
    async fn built(&self, id: &str) -> Result<(), Failure> {
        let has = |b: &wad_proto::WadspaceBuild| b.image.is_some() && b.job.is_none();
        let now = self.builds().await?.into_iter().find(|b| b.id == id);
        if now.as_ref().is_some_and(|b| has(b) || (b.image.is_some() && !b.state.needs_build())) {
            return Ok(());
        }
        let answer = self.build(id).await?;
        let Some(job) = answer.job else {
            return if answer.state.image.is_some() { Ok(()) } else { Err(failed("it can't be built here".into())) };
        };
        let deadline = tokio::time::Instant::now() + BUILD_WAIT;
        loop {
            let j = self.job(&job.job.id, u64::MAX).await?;
            match j.job.job.status {
                JobStatus::Done => return Ok(()),
                JobStatus::Error | JobStatus::Cancelled => {
                    return Err(failed(j.job.job.error.unwrap_or_else(|| "its build didn't finish".into())));
                }
                _ => {}
            }
            if tokio::time::Instant::now() > deadline {
                return Err(failed("its build is taking too long".into()));
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}
