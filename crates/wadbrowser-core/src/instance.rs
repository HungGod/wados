//! Instances: several WadBrowsers at once, each a browser of its own (v1's
//! `instance.rs`).
//!
//! Without `--instance` there's one WadBrowser per user: application id
//! [`APP_ID`], so a second launch hands its request to the first over D-Bus
//! (GApplication's uniqueness). With `--instance <id>`, the application id
//! ([`app_id`]) and the data are that instance's own:
//!
//! ```text
//! io.wados.Browser.<id>                          its D-Bus name
//! $XDG_CONFIG_HOME/wadbrowser/instances/<id>/    its settings
//! $XDG_DATA_HOME/wadbrowser/instances/<id>/      its store, session and profiles
//! $XDG_CACHE_HOME/wadbrowser/instances/<id>/     its caches
//! ```
//!
//! A later launch with the same id still reaches that instance, so its links
//! keep working. Its windows keep WadBrowser's Wayland app_id (a web app's,
//! `wadspaces-webapp-<id>`), so they go with the same launchers.
//! `wadbrowser.conf` is the user's whatever the instance (`crate::conf`).

use crate::{APP_ID, AppPaths};
use anyhow::{Context, Result};
use directories::ProjectDirs;

/// The application id (D-Bus name) of the browser, or of an instance of it.
/// An instance id is `cli::web_app_id`'s (a-z, 0-9, -); an element of an
/// application id can't start with a digit, so such an id gets a `_` first.
pub fn app_id(instance: Option<&str>) -> String {
    match instance {
        None => APP_ID.into(),
        Some(id) => {
            let lead = if id.starts_with(|c: char| c.is_ascii_digit()) { "_" } else { "" };
            let id: String = id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
            format!("{APP_ID}.{lead}{id}")
        }
    }
}

/// The browser's folders (`~/.config/wadbrowser`…), or an instance's under
/// them; made if they aren't there.
pub fn paths(instance: Option<&str>) -> Result<AppPaths> {
    let dirs = ProjectDirs::from("io", "wados", "WadBrowser").context("could not resolve XDG project directories")?;
    let user = AppPaths {
        config_dir: dirs.config_dir().to_path_buf(),
        data_dir: dirs.data_dir().to_path_buf(),
        cache_dir: dirs.cache_dir().to_path_buf(),
    };
    let paths = under(&user, instance);
    paths.ensure()?;
    Ok(paths)
}

/// `user`'s folders, or the instance's in them.
pub fn under(user: &AppPaths, instance: Option<&str>) -> AppPaths {
    match instance {
        None => user.clone(),
        Some(id) => AppPaths {
            config_dir: user.config_dir.join("instances").join(id),
            data_dir: user.data_dir.join("instances").join(id),
            cache_dir: user.cache_dir.join("instances").join(id),
        },
    }
}

/// The user's own WadBrowser settings folder (`~/.config/wadbrowser`),
/// whatever the instance: where its `wadbrowser.conf` is.
pub fn user_config_dir() -> Option<std::path::PathBuf> {
    ProjectDirs::from("io", "wados", "WadBrowser").map(|d| d.config_dir().to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn app_ids() {
        assert_eq!(app_id(None), "io.wados.Browser");
        assert_eq!(app_id(Some("browser-mail")), "io.wados.Browser.browser-mail");
        assert_eq!(app_id(Some("9lives")), "io.wados.Browser._9lives");
    }

    #[test]
    fn an_instances_folders_are_its_own() {
        let user = AppPaths {
            config_dir: "/h/.config/wadbrowser".into(),
            data_dir: "/h/.local/share/wadbrowser".into(),
            cache_dir: "/h/.cache/wadbrowser".into(),
        };
        assert_eq!(under(&user, None).data_dir, user.data_dir);
        let i = under(&user, Some("focus"));
        assert_eq!(i.config_dir, PathBuf::from("/h/.config/wadbrowser/instances/focus"));
        assert_eq!(i.data_dir, PathBuf::from("/h/.local/share/wadbrowser/instances/focus"));
        assert_eq!(i.cache_dir, PathBuf::from("/h/.cache/wadbrowser/instances/focus"));
        assert_eq!(i.session_path(), PathBuf::from("/h/.local/share/wadbrowser/instances/focus/session.json"));
    }
}
