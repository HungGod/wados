//! Steam's own text files (VDF: `"key" "value"` and `"key" { … }`), and what
//! a signed-in client's folder says: who's signed in, what the account has
//! played or owns, and what's downloaded. No Web API key, and one sign-in:
//! everything here comes from the client's files.

use indexmap::IndexMap;

/// A value: text, or a section. Keys are matched without case, as Steam does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Vdf {
    Text(String),
    Section(IndexMap<String, Vdf>),
}

impl Vdf {
    pub fn get(&self, key: &str) -> Option<&Vdf> {
        match self {
            Vdf::Section(m) => m.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v),
            Vdf::Text(_) => None,
        }
    }

    /// Down a path of sections.
    pub fn at(&self, path: &[&str]) -> Option<&Vdf> {
        path.iter().try_fold(self, |v, k| v.get(k))
    }

    pub fn text(&self, key: &str) -> Option<&str> {
        match self.get(key) {
            Some(Vdf::Text(t)) => Some(t),
            _ => None,
        }
    }

    pub fn keys(&self) -> Vec<&str> {
        match self {
            Vdf::Section(m) => m.keys().map(String::as_str).collect(),
            Vdf::Text(_) => vec![],
        }
    }
}

struct Reader<'a> {
    rest: &'a str,
}

impl Reader<'_> {
    fn skip(&mut self) {
        loop {
            self.rest = self.rest.trim_start();
            if self.rest.starts_with("//") {
                self.rest = self.rest.split_once('\n').map_or("", |(_, r)| r);
            } else {
                return;
            }
        }
    }

    fn string(&mut self) -> Option<String> {
        let body = self.rest.strip_prefix('"')?;
        let mut out = String::new();
        let mut chars = body.char_indices();
        while let Some((i, c)) = chars.next() {
            match c {
                '"' => {
                    self.rest = &body[i + 1..];
                    return Some(out);
                }
                '\\' => match chars.next()?.1 {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    other => out.push(other),
                },
                c => out.push(c),
            }
        }
        None
    }

    fn section(&mut self, top: bool) -> Option<Vdf> {
        let mut m = IndexMap::new();
        loop {
            self.skip();
            if let Some(r) = self.rest.strip_prefix('}') {
                self.rest = r;
                return (!top).then_some(Vdf::Section(m));
            }
            if self.rest.is_empty() {
                return top.then_some(Vdf::Section(m));
            }
            let key = self.string()?;
            self.skip();
            if let Some(r) = self.rest.strip_prefix('{') {
                self.rest = r;
                m.insert(key, self.section(false)?);
            } else {
                m.insert(key, Vdf::Text(self.string()?));
            }
        }
    }
}

/// A whole file, or none if it isn't VDF.
pub fn parse(text: &str) -> Option<Vdf> {
    Reader { rest: text.trim_start_matches('\u{feff}') }.section(true)
}

/// The signed-in account in `loginusers.vdf`: its SteamID and name. The most
/// recent, or the only one.
pub fn signed_in(text: &str) -> Option<(String, String)> {
    let v = parse(text)?;
    let users = v.get("users")?;
    let ids = users.keys();
    let recent = ids.iter().find(|id| users.get(id).and_then(|u| u.text("MostRecent")) == Some("1"));
    let id = recent.or(ids.first())?;
    let u = users.get(id)?;
    let name = u.text("PersonaName").or(u.text("AccountName")).unwrap_or(id);
    Some((id.to_string(), name.to_string()))
}

/// The apps in `localconfig.vdf`: what this account has on this client.
pub fn apps_of(text: &str) -> Vec<u32> {
    let Some(v) = parse(text) else { return vec![] };
    let Some(apps) = v.at(&["UserLocalConfigStore", "Software", "Valve", "Steam", "apps"]) else { return vec![] };
    apps.keys().iter().filter_map(|k| k.parse().ok()).collect()
}

/// A game's `appmanifest_<appid>.acf`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub appid: u32,
    pub name: String,
    /// Its folder in `steamapps/common`.
    pub dir: String,
    /// Fully downloaded.
    pub installed: bool,
    pub bytes: u64,
}

pub fn manifest(text: &str) -> Option<Manifest> {
    let v = parse(text)?;
    let a = v.get("AppState")?;
    let flags: u32 = a.text("StateFlags").and_then(|f| f.parse().ok()).unwrap_or(0);
    Some(Manifest {
        appid: a.text("appid")?.parse().ok()?,
        name: a.text("name")?.to_string(),
        dir: a.text("installdir")?.to_string(),
        installed: flags & 4 != 0,
        bytes: a.text("SizeOnDisk").and_then(|s| s.parse().ok()).unwrap_or(0),
    })
}

/// A game's id for an app: the catalog's, else `app-<appid>`.
pub fn game_id(appid: u32) -> String {
    crate::catalog::steam_games()
        .iter()
        .find(|g| g.appid == appid)
        .map_or_else(|| format!("app-{appid}"), |g| g.id.clone())
}

/// The app a game's id means.
pub fn appid_of(id: &str) -> Option<u32> {
    let known = crate::catalog::steam_games().iter().find(|g| g.id == id).map(|g| g.appid);
    known.or_else(|| id.strip_prefix("app-")?.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOGIN: &str = r#"
"users"
{
	"76561198000000001"
	{
		"AccountName"		"old"
		"PersonaName"		"Old"
		"MostRecent"		"0"
	}
	"76561198000000002"
	{
		"AccountName"		"sam"
		"PersonaName"		"Sam \"S\" Doe"
		"MostRecent"		"1"
	}
}
"#;

    #[test]
    fn the_most_recent_account_is_whos_signed_in() {
        let (id, name) = signed_in(LOGIN).unwrap();
        assert_eq!((id.as_str(), name.as_str()), ("76561198000000002", "Sam \"S\" Doe"));
        assert_eq!(signed_in("\"users\"\n{\n}\n"), None);
        assert_eq!(signed_in("not vdf"), None);
    }

    #[test]
    fn a_clients_apps_ignore_case_and_what_isnt_an_app() {
        let t = r#""UserLocalConfigStore" { "Software" { "valve" { "Steam" { "apps" {
            "413150" { "LastPlayed" "1" } "620" { } "notanumber" { } } } } } }"#;
        assert_eq!(apps_of(t), [413150, 620]);
        assert!(apps_of("\"x\" \"y\"").is_empty());
    }

    #[test]
    fn a_manifest_says_if_its_all_there() {
        let t = r#""AppState" { "appid" "620" "name" "Portal 2" "installdir" "Portal 2"
            "StateFlags" "4" "SizeOnDisk" "12000000000" }"#;
        let m = manifest(t).unwrap();
        assert_eq!((m.appid, m.dir.as_str(), m.installed, m.bytes), (620, "Portal 2", true, 12_000_000_000));
        let part = t.replace("\"4\"", "\"1026\"");
        assert!(!manifest(&part).unwrap().installed);
    }

    #[test]
    fn comments_and_broken_files_are_handled() {
        assert!(parse("// a comment\n\"a\" \"b\"\n").is_some());
        assert!(parse("\"a\" {").is_none());
        assert!(parse("\"a\" \"b").is_none());
    }

    #[test]
    fn a_game_keeps_a_way_back_to_its_app() {
        let known = crate::catalog::steam_games().first().unwrap();
        assert_eq!(game_id(known.appid), known.id);
        assert_eq!(appid_of(&known.id), Some(known.appid));
        assert_eq!(game_id(999_999_999), "app-999999999");
        assert_eq!(appid_of("app-999999999"), Some(999_999_999));
        assert_eq!(appid_of("nothing"), None);
    }
}
