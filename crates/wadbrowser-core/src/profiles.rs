//! Browser profiles: where pages keep cookies, storage and cache. Each is
//! one WebKit network session (wadbrowser-webkit's `profiles`); this is
//! what a profile is, without WebKit.
//!
//! - `default`: the browser's one profile, kept in
//!   `<data>/profiles/default` (its cache in `<cache>/profiles/default`).
//!   Every window uses it unless it's private or a key's, so one sign-in
//!   covers every web app. There are no named profiles: an id that isn't
//!   one of the kinds below (a web app made with one once, `--profile work`)
//!   is `default` ([`canonical`]).
//! - `signin-<key>`: a key (a saved sign-in) this wadspace mounts. wadd
//!   lists them in `/run/wadspaces-extra/signins.json` (`WADBROWSER_SIGNINS`
//!   names another file); a key's profile keeps its data in the key's
//!   folder, its cache in this wadspace's own. A web app signs in with its
//!   key, and the browser's own windows with the key whose `apps` lists
//!   `@browser`. A page on one of a key's `sites` opens in its profile
//!   ([`profile_for`]). On the machine there's no signins.json: its web keys
//!   are wadd's ([`SignIns::from_machine`]), each keeping its data in its
//!   folder there (`/var/lib/wados/keys/<id>`), the one wadspaces mount.
//! - `ephemeral-…` keeps nothing at all (a web app with a key that isn't
//!   here, "Don't save"), nor does `private-…` (a private window's).
//! - In an ephemeral run every profile keeps nothing, but a key's: its
//!   data is the key's, not this run's.

use crate::paths::AppPaths;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

pub const DEFAULT: &str = "default";
pub const SIGNIN: &str = "signin-";
pub const EPHEMERAL: &str = "ephemeral-";
pub const PRIVATE: &str = "private-";
/// Where wadd lists a wadspace's keys.
pub const SIGNINS_FILE: &str = "/run/wadspaces-extra/signins.json";
/// Names another signins.json (tests).
pub const SIGNINS_ENV: &str = "WADBROWSER_SIGNINS";
/// The browser's own windows in a key's `apps` (wad-gen's `BROWSER_APP`).
pub const BROWSER_APP: &str = "@browser";
/// The default profile's name and colour, as the spike has them.
pub const DEFAULT_NAME: &str = "Personal";
pub const DEFAULT_COLOR: &str = "#c6ff1f";

/// What a profile id says it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileKind {
    Default,
    /// A key's (its id).
    SignIn(String),
    Ephemeral(String),
    Private(String),
}

/// The profile `id` is: itself, or `default` for any id that isn't a key's,
/// an ephemeral or a private one (the named profiles there once).
pub fn canonical(id: &str) -> String {
    match ProfileKind::of(id) {
        ProfileKind::Default => DEFAULT.to_string(),
        _ => id.to_string(),
    }
}

impl ProfileKind {
    pub fn of(id: &str) -> Self {
        if id == DEFAULT {
            Self::Default
        } else if let Some(k) = id.strip_prefix(SIGNIN) {
            Self::SignIn(k.to_string())
        } else if let Some(e) = id.strip_prefix(EPHEMERAL) {
            Self::Ephemeral(e.to_string())
        } else if let Some(p) = id.strip_prefix(PRIVATE) {
            Self::Private(p.to_string())
        } else {
            Self::Default
        }
    }
}

/// A key's profile id.
pub fn signin_profile(key: &str) -> String {
    format!("{SIGNIN}{key}")
}

/// A private window's profile id (`n` makes each window's its own).
pub fn private_profile(n: u64) -> String {
    format!("{PRIVATE}{n}")
}

pub fn is_private(id: &str) -> bool {
    id.starts_with(PRIVATE)
}

/// Where a profile keeps its cookies, storage and cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Storage {
    /// Nothing kept: in memory, gone when its last view is.
    Ephemeral,
    Disk {
        data: PathBuf,
        cache: PathBuf,
    },
}

impl Storage {
    /// Its cookies' SQLite file (none for an ephemeral one).
    pub fn cookies(&self) -> Option<PathBuf> {
        match self {
            Self::Ephemeral => None,
            Self::Disk { data, .. } => Some(data.join("cookies.sqlite")),
        }
    }
}

/// Where profile `id` keeps things: `signins` are this wadspace's keys,
/// `ephemeral_run` a run that keeps nothing.
pub fn resolve(id: &str, paths: &AppPaths, signins: &[SignIn], ephemeral_run: bool) -> Storage {
    let disk = |name: &str| Storage::Disk {
        data: paths.data_dir.join("profiles").join(name),
        cache: paths.cache_dir.join("profiles").join(name),
    };
    match ProfileKind::of(id) {
        ProfileKind::Private(_) | ProfileKind::Ephemeral(_) => Storage::Ephemeral,
        // A key's data is the key's, even in a run that keeps nothing of its own.
        ProfileKind::SignIn(key) => match signins.iter().find(|s| s.id == key && s.web) {
            Some(s) => Storage::Disk { data: s.dir.clone(), cache: paths.cache_dir.join("profiles").join(id) },
            // A key that isn't here: this keeps nothing rather than quietly using another's.
            None => Storage::Ephemeral,
        },
        _ if ephemeral_run => Storage::Ephemeral,
        ProfileKind::Default => disk(DEFAULT),
    }
}

// ------------------------------------------------------------ signins.json

/// A key this wadspace mounts (an entry of signins.json).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SignIn {
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// A web key: a browser profile (an app's key is files, linked by `wadspaces-signins`).
    #[serde(default)]
    pub web: bool,
    /// What signs in with it: web apps (their ids) and [`BROWSER_APP`].
    #[serde(default)]
    pub apps: Vec<String>,
    /// Its folder: a web key's profile data.
    pub dir: PathBuf,
    #[serde(default)]
    pub paths: Vec<String>,
    /// The sites it signs in to (version 2): a page on one of them opens in its profile.
    #[serde(default)]
    pub sites: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct SignIns {
    #[serde(default = "first_version")]
    pub version: u32,
    #[serde(default)]
    pub signins: Vec<SignIn>,
}

fn first_version() -> u32 {
    1
}

impl SignIns {
    /// signins.json, version 1 (no sites) or 2.
    pub fn parse(text: &str) -> Result<Self> {
        let all: Self = serde_json::from_str(text).context("signins.json")?;
        if all.version == 0 || all.version > 2 {
            anyhow::bail!("signins.json: version {} isn't one this browser reads", all.version);
        }
        Ok(all)
    }

    /// This wadspace's: `WADBROWSER_SIGNINS`'s file, else wadd's. None
    /// (on the machine, say) is no keys.
    pub fn here() -> Self {
        Self::load(&Self::path())
    }

    /// Where this wadspace's would be (`WADBROWSER_SIGNINS`, else wadd's).
    pub fn path() -> PathBuf {
        std::env::var_os(SIGNINS_ENV).map(PathBuf::from).unwrap_or_else(|| SIGNINS_FILE.into())
    }

    /// There's a signins.json: this is a wadspace, whose keys are its
    /// file's. Without one, the machine's are wadd's ([`Self::from_machine`]).
    pub fn file_here() -> bool {
        Self::path().exists()
    }

    /// The machine's web keys (wadd's `GET /v1/keys/web`), as a wadspace's
    /// signins.json lists the ones it mounts: each key's profile keeps its
    /// data in the key's folder (`/var/lib/wados/keys/<id>`, this user's,
    /// reached by name), its cache in the browser's own. Only the ones whose
    /// folder is there for this user; none signs in a web app or the
    /// browser's own windows on the machine (they're the default profile's).
    pub fn from_machine(keys: &[wad_proto::signin::WebKey]) -> Self {
        let signins = keys
            .iter()
            .map(|k| SignIn {
                id: k.id.clone(),
                name: k.name.clone(),
                web: true,
                apps: vec![],
                dir: PathBuf::from(&k.dir),
                paths: vec![],
                sites: k.sites.clone(),
            })
            .filter(|s| {
                let there = s.dir.is_dir();
                if !there {
                    tracing::warn!("key {}: its folder {} isn't there for this user", s.id, s.dir.display());
                }
                there
            })
            .collect();
        Self { version: 2, signins }
    }

    pub fn load(path: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else { return Self::default() };
        match Self::parse(&text) {
            Ok(mut all) => {
                // Only the web keys that are mounted here.
                all.signins.retain(|s| s.web && s.dir.is_dir());
                all
            }
            Err(e) => {
                tracing::warn!("{}: {e:#}", path.display());
                Self::default()
            }
        }
    }

    /// The profile the browser's own windows use: the key that lists
    /// [`BROWSER_APP`], else `default`.
    pub fn browser_profile(&self) -> String {
        self.signins
            .iter()
            .find(|s| s.web && s.apps.iter().any(|a| a == BROWSER_APP))
            .map_or_else(|| DEFAULT.to_string(), |s| signin_profile(&s.id))
    }

    /// A web app's profile (its id, as in webapps.json): its key's, if it has one here.
    pub fn app_profile(&self, web_app: &str) -> Option<String> {
        self.signins.iter().find(|s| s.web && s.apps.iter().any(|a| a == web_app)).map(|s| signin_profile(&s.id))
    }

    /// Each web key with sites, as [`profile_for`] takes them.
    pub fn site_keys(&self) -> Vec<SiteKey> {
        self.signins
            .iter()
            .filter(|s| s.web && !s.sites.is_empty())
            .map(|s| SiteKey { profile: signin_profile(&s.id), sites: s.sites.clone() })
            .collect()
    }
}

// ------------------------------------------------------------ per-site keys

/// A key's profile and the sites it signs in to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteKey {
    pub profile: String,
    pub sites: Vec<String>,
}

/// "WWW.GitHub.com:443" → "github.com".
pub fn bare_host(host: &str) -> String {
    let h = host.trim().trim_end_matches('.').to_ascii_lowercase();
    let h = match h.rsplit_once(':') {
        Some((name, port)) if !name.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => name.to_string(),
        _ => h,
    };
    h.strip_prefix("www.").map(str::to_string).unwrap_or(h)
}

/// Whether `host` is `site` or under it ("gist.github.com" is github.com's).
pub fn on_site(host: &str, site: &str) -> bool {
    let (h, s) = (bare_host(host), bare_host(site));
    !s.is_empty() && (h == s || h.ends_with(&format!(".{s}")))
}

/// The profile a top-level page on `host` should be in, when it isn't
/// `current`'s: the key's whose sites have it (the most particular site, if
/// more than one does). None: it stays where it is. A tab never leaves a key
/// by itself (a sign-in goes through other sites), and a private window's
/// tabs never go into a key (a private window keeps nothing).
pub fn profile_for(host: &str, current: &str, keys: &[SiteKey]) -> Option<String> {
    if is_private(current) || host.is_empty() {
        return None;
    }
    let best = keys
        .iter()
        .filter_map(|k| {
            let len = k.sites.iter().filter(|s| on_site(host, s)).map(|s| bare_host(s).len()).max()?;
            Some((len, k))
        })
        // The most particular site; of equals, the first key.
        .fold(None::<(usize, &SiteKey)>, |best, (len, k)| match best {
            Some((b, _)) if b >= len => best,
            _ => Some((len, k)),
        })?
        .1;
    (best.profile != current).then(|| best.profile.clone())
}

// ------------------------------------------------------------ named profiles

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(dir: &Path) -> AppPaths {
        AppPaths { config_dir: dir.join("config"), data_dir: dir.join("data"), cache_dir: dir.join("cache") }
    }

    fn key(id: &str, apps: &[&str], sites: &[&str]) -> SignIn {
        SignIn {
            id: id.into(),
            name: id.into(),
            web: true,
            apps: apps.iter().map(|s| s.to_string()).collect(),
            dir: PathBuf::from(format!("/run/wadspaces-signins/{id}")),
            paths: vec![],
            sites: sites.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn where_each_profile_keeps_things() {
        let p = paths(Path::new("/h"));
        let keys = [key("work", &["@browser"], &[])];
        assert_eq!(
            resolve("default", &p, &keys, false),
            Storage::Disk { data: "/h/data/profiles/default".into(), cache: "/h/cache/profiles/default".into() }
        );
        // A named profile's id (there are none now) is the default one's.
        assert_eq!(resolve("school", &p, &keys, false), resolve("default", &p, &keys, false));
        // A key's data is its folder; its cache this wadspace's.
        assert_eq!(
            resolve("signin-work", &p, &keys, false),
            Storage::Disk { data: "/run/wadspaces-signins/work".into(), cache: "/h/cache/profiles/signin-work".into() }
        );
        assert_eq!(
            resolve("default", &p, &keys, false).cookies(),
            Some(PathBuf::from("/h/data/profiles/default/cookies.sqlite"))
        );
        // Nothing kept: private windows, ephemeral profiles, a key that isn't here.
        for id in ["private-1", "ephemeral-gmail", "signin-gone"] {
            assert_eq!(resolve(id, &p, &keys, false), Storage::Ephemeral, "{id}");
        }
        // An ephemeral run keeps nothing of its own, but a key is still the key.
        assert_eq!(resolve("default", &p, &keys, true), Storage::Ephemeral);
        assert_eq!(resolve("school", &p, &keys, true), Storage::Ephemeral);
        assert!(matches!(resolve("signin-work", &p, &keys, true), Storage::Disk { .. }));
    }

    #[test]
    fn profile_ids_say_what_they_are() {
        assert_eq!(ProfileKind::of("default"), ProfileKind::Default);
        assert_eq!(ProfileKind::of("signin-k1"), ProfileKind::SignIn("k1".into()));
        assert_eq!(ProfileKind::of("ephemeral-gmail"), ProfileKind::Ephemeral("gmail".into()));
        assert_eq!(ProfileKind::of("private-3"), ProfileKind::Private("3".into()));
        assert_eq!(ProfileKind::of("work"), ProfileKind::Default);
        assert_eq!(canonical("work"), "default");
        assert_eq!(canonical("../x"), "default");
        assert_eq!(canonical("signin-k1"), "signin-k1");
        assert_eq!(canonical("private-3"), "private-3");
    }

    #[test]
    fn signins_version_1_and_2() {
        let v1 = r#"{"version": 1, "signins": [
            {"id": "vscode-k3x9q2", "name": "VS Code", "web": false, "apps": ["vscode"],
             "dir": "/run/wadspaces-signins/vscode-k3x9q2", "paths": [".config/Code/User/globalStorage/"]},
            {"id": "work", "name": "Work", "web": true, "apps": ["gmail", "@browser"],
             "dir": "/run/wadspaces-signins/work", "paths": []}
        ]}"#;
        let all = SignIns::parse(v1).expect("version 1");
        assert_eq!(all.version, 1);
        assert_eq!(all.signins.len(), 2);
        assert!(all.signins[1].sites.is_empty());
        assert_eq!(all.browser_profile(), "signin-work");
        assert_eq!(all.app_profile("gmail").as_deref(), Some("signin-work"));
        assert_eq!(all.app_profile("vscode"), None, "an app's key isn't a profile");
        assert!(all.site_keys().is_empty());

        let v2 = r#"{"version": 2, "signins": [
            {"id": "hunggod", "name": "hunggod", "web": true, "apps": ["github"],
             "dir": "/run/wadspaces-signins/hunggod", "paths": [], "sites": ["github.com"]}
        ]}"#;
        let all = SignIns::parse(v2).expect("version 2");
        assert_eq!(all.browser_profile(), "default");
        assert_eq!(all.site_keys(), [SiteKey { profile: "signin-hunggod".into(), sites: vec!["github.com".into()] }]);
        // No version is version 1; a later one isn't read.
        assert_eq!(SignIns::parse(r#"{"signins": []}"#).expect("read").version, 1);
        assert!(SignIns::parse(r#"{"version": 3, "signins": []}"#).is_err());
        assert!(SignIns::parse("not json").is_err());
    }

    #[test]
    fn only_mounted_web_keys_are_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let mounted = dir.path().join("k1");
        std::fs::create_dir_all(&mounted).unwrap();
        let file = dir.path().join("signins.json");
        let text = serde_json::json!({"version": 2, "signins": [
            {"id": "k1", "web": true, "apps": [], "dir": mounted, "sites": ["a.test"]},
            {"id": "k2", "web": true, "apps": [], "dir": dir.path().join("missing"), "sites": ["b.test"]},
            {"id": "k3", "web": false, "apps": ["vscode"], "dir": mounted}
        ]});
        std::fs::write(&file, text.to_string()).unwrap();
        let all = SignIns::load(&file);
        assert_eq!(all.signins.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["k1"]);
        assert_eq!(SignIns::load(&dir.path().join("none.json")), SignIns::default());
    }

    #[test]
    fn the_machines_keys_are_its_folders() {
        let dir = tempfile::tempdir().unwrap();
        let there = dir.path().join("keys/gh");
        std::fs::create_dir_all(&there).unwrap();
        let key = |id: &str, dir: &Path, sites: &[&str]| wad_proto::signin::WebKey {
            id: id.into(),
            name: id.to_uppercase(),
            sites: sites.iter().map(|s| s.to_string()).collect(),
            used_by: vec![],
            dir: dir.to_string_lossy().into_owned(),
        };
        let all = SignIns::from_machine(&[
            key("gh", &there, &["github.com"]),
            key("gone", &dir.path().join("keys/gone"), &["gitlab.com"]),
        ]);
        assert_eq!(all.version, 2);
        assert_eq!(all.signins.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["gh"]);
        assert_eq!(all.site_keys(), [SiteKey { profile: "signin-gh".into(), sites: vec!["github.com".into()] }]);
        // The browser's own windows and web apps stay the default profile's.
        assert_eq!(all.browser_profile(), "default");
        // As in a wadspace: its data the key's folder, its cache the browser's.
        let p = paths(Path::new("/h"));
        assert_eq!(
            resolve("signin-gh", &p, &all.signins, false),
            Storage::Disk { data: there.clone(), cache: "/h/cache/profiles/signin-gh".into() }
        );
        assert_eq!(resolve("signin-gh", &p, &all.signins, false).cookies(), Some(there.join("cookies.sqlite")));
    }

    #[test]
    fn a_page_goes_into_the_key_for_its_site() {
        let keys = [
            SiteKey { profile: "signin-work".into(), sites: vec!["google.com".into(), "claude.ai".into()] },
            SiteKey { profile: "signin-mail".into(), sites: vec!["mail.google.com".into()] },
            SiteKey { profile: "signin-gh".into(), sites: vec!["github.com".into()] },
        ];
        assert_eq!(profile_for("github.com", "default", &keys).as_deref(), Some("signin-gh"));
        assert_eq!(profile_for("WWW.GitHub.com", "default", &keys).as_deref(), Some("signin-gh"));
        assert_eq!(profile_for("gist.github.com", "work", &keys).as_deref(), Some("signin-gh"));
        // The most particular site wins.
        assert_eq!(profile_for("mail.google.com", "default", &keys).as_deref(), Some("signin-mail"));
        assert_eq!(profile_for("docs.google.com", "signin-mail", &keys).as_deref(), Some("signin-work"));
        // Already there; no key's site; a look-alike host.
        assert_eq!(profile_for("github.com", "signin-gh", &keys), None);
        assert_eq!(profile_for("example.com", "signin-gh", &keys), None, "a tab doesn't leave its key");
        assert_eq!(profile_for("notgithub.com", "default", &keys), None);
        assert_eq!(profile_for("github.com.evil.test", "default", &keys), None);
        // A private window's tabs stay private.
        assert_eq!(profile_for("github.com", "private-2", &keys), None);
        // An ephemeral profile's go into the key.
        assert_eq!(profile_for("claude.ai", "ephemeral-x", &keys).as_deref(), Some("signin-work"));
        assert_eq!(profile_for("localhost", "default", &[]), None);
        let local = [SiteKey { profile: "signin-k".into(), sites: vec!["localhost:8000".into()] }];
        assert_eq!(profile_for("localhost", "default", &local).as_deref(), Some("signin-k"));
    }
}
