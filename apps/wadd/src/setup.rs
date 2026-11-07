//! Set up WadSpaces (`GET|PUT /v1/setup`, `POST /v1/setup/design`): where
//! it's got to, in `<state>/setup.json`. The installer leaves that file on a
//! new machine (provision puts it there). No file: nothing to set up, so a
//! machine the installer didn't make never sees it. The window closed early
//! opens again at the next sign-in, where it was.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use wad_proto::{ApiError, Setup, SetupDesign, SetupStep};

use crate::api_phase8::Updates;
use crate::library::Docs;

pub const FILE: &str = "setup.json";
const DRM: &str = "/sys/class/drm";

pub struct SetupState {
    path: PathBuf,
    step: Mutex<SetupStep>,
    docs: Arc<Docs>,
    updates: Arc<dyn Updates>,
}

/// What `state_dir`'s file says.
pub fn read(state_dir: &Path) -> SetupStep {
    std::fs::read(state_dir.join(FILE))
        .ok()
        .and_then(|b| serde_json::from_slice::<Setup>(&b).ok())
        .map(|s| s.step)
        .unwrap_or_default()
}

impl SetupState {
    pub fn new(state_dir: &Path, docs: Arc<Docs>, updates: Arc<dyn Updates>) -> Arc<Self> {
        Arc::new(Self { path: state_dir.join(FILE), step: Mutex::new(read(state_dir)), docs, updates })
    }

    pub fn get(&self) -> Setup {
        let nvidia_card = crate::remote::gpu(Path::new(DRM)).as_deref() == Some("NVIDIA");
        Setup { step: *self.step.lock().unwrap(), nvidia_card }
    }

    fn save(&self, step: SetupStep) -> Result<(), ApiError> {
        let bytes =
            serde_json::to_vec(&Setup { step, nvidia_card: false }).map_err(|e| ApiError::internal(e.to_string()))?;
        crate::store::write_atomic(&self.path, &bytes).map_err(|e| ApiError::internal(e.to_string()))?;
        *self.step.lock().unwrap() = step;
        Ok(())
    }

    /// Goes to a step. Once it's done it's done; the end is [`Self::design`]'s.
    pub fn put(&self, step: SetupStep) -> Result<Setup, ApiError> {
        if self.get().step == SetupStep::Done {
            return Err(ApiError::conflict("Set up is done"));
        }
        if step == SetupStep::Done {
            return Err(ApiError::bad_request("Set up ends with its last step"));
        }
        self.save(step)?;
        Ok(self.get())
    }

    /// Install or Skip: the WadOS wadspace if asked for, the host image
    /// with NVIDIA's driver if asked for, and the base images start
    /// downloading either way. Those failing is their jobs' to say.
    pub async fn design(&self, req: SetupDesign) -> Result<Setup, ApiError> {
        if req.starter {
            self.docs.add_starter()?;
        }
        if let Err(e) = self.updates.refresh_bases().await {
            tracing::warn!("the base images didn't start downloading: {}", e.message);
        }
        if req.nvidia
            && let Err(e) = self.updates.use_nvidia().await
        {
            tracing::warn!("NVIDIA's host image didn't start downloading: {}", e.message);
        }
        self.save(SetupStep::Done)?;
        Ok(self.get())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_file_means_nothing_to_set_up() {
        let d = std::env::temp_dir().join(format!("wadd-setup-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(read(&d), SetupStep::Done);
        std::fs::write(d.join(FILE), br#"{"step":"welcome"}"#).unwrap();
        assert_eq!(read(&d), SetupStep::Welcome);
        std::fs::write(d.join(FILE), b"not json").unwrap();
        assert_eq!(read(&d), SetupStep::Done);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
