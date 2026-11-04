use anyhow::{Context, Result};
use directories::ProjectDirs;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
}

impl AppPaths {
    pub fn discover() -> Result<Self> {
        let dirs =
            ProjectDirs::from("io", "wados", "WadBrowser").context("could not resolve XDG project directories")?;
        let paths = Self {
            config_dir: dirs.config_dir().to_path_buf(),
            data_dir: dirs.data_dir().to_path_buf(),
            cache_dir: dirs.cache_dir().to_path_buf(),
        };
        paths.ensure()?;
        Ok(paths)
    }

    pub fn ensure(&self) -> Result<()> {
        fs::create_dir_all(&self.config_dir)?;
        fs::create_dir_all(&self.data_dir)?;
        fs::create_dir_all(&self.cache_dir)?;
        fs::create_dir_all(self.filter_cache_dir())?;
        fs::create_dir_all(self.downloads_dir())?;
        Ok(())
    }

    pub fn prefs_path(&self) -> PathBuf {
        self.config_dir.join("prefs.toml")
    }

    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("wadbrowser.db")
    }

    pub fn filter_cache_dir(&self) -> PathBuf {
        self.cache_dir.join("filters")
    }

    pub fn downloads_dir(&self) -> PathBuf {
        directories::UserDirs::new()
            .and_then(|u| u.download_dir().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| self.data_dir.join("Downloads"))
    }

    pub fn session_path(&self) -> PathBuf {
        self.data_dir.join("session.json")
    }
}
