use anyhow::Result;
use wadbrowser_core::AppPaths;
use webkit6::{NetworkSession, UserContentManager, WebsiteDataManager, WebsiteDataTypes, gio, glib};

/// Shared WebKit network/session objects for a browsing profile.
#[derive(Clone)]
pub struct SessionBundle {
    pub network: NetworkSession,
    pub content: UserContentManager,
    pub private: bool,
}

impl SessionBundle {
    pub fn persistent(paths: &AppPaths) -> Result<Self> {
        let data = paths.data_dir.join("webkit");
        let cache = paths.cache_dir.join("webkit");
        std::fs::create_dir_all(&data)?;
        std::fs::create_dir_all(&cache)?;

        let network = NetworkSession::builder()
            .data_directory(data.to_string_lossy().as_ref())
            .cache_directory(cache.to_string_lossy().as_ref())
            .build();

        tune_session(&network);
        Ok(Self { network, content: UserContentManager::new(), private: false })
    }

    pub fn ephemeral() -> Self {
        let network = NetworkSession::new_ephemeral();
        tune_session(&network);
        Self { network, content: UserContentManager::new(), private: true }
    }

    pub fn clear_http_cache(&self) {
        if let Some(manager) = self.network.website_data_manager() {
            clear_all_data(&manager);
        }
    }

    pub fn clear_cookies_and_site_data(&self) {
        if let Some(manager) = self.network.website_data_manager() {
            clear_all_data(&manager);
        }
    }
}

fn tune_session(network: &NetworkSession) {
    network.set_itp_enabled(true);
}

fn clear_all_data(manager: &WebsiteDataManager) {
    manager.clear(WebsiteDataTypes::ALL, glib::TimeSpan(0), None::<&gio::Cancellable>, |_| {});
}
