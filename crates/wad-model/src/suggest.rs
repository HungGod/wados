// What the Builder suggests adding to a wadspace: add-ons for the apps it
// has, that it hasn't yet. What goes with what it has comes first (a
// language's tools and its editor's extension go together, a toolchain's
// docs in the browser), then the ones most wadspaces like that want.

use serde::{Deserialize, Serialize};

use crate::catalog::{WEB_APPS, app_name, extension, extensions, site, tool, tools};
use crate::types::{AppId, Wadspace};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Suggestion {
    pub app: AppId,
    /// The add-on, as the Builder adds it: 'vscode:rust-lang.rust-analyzer', 'terminal:rust', 'wadbrowser:github'.
    pub item: String,
    pub id: String,
    pub name: String,
    /// Why it's suggested ("Goes with Rust"), if it goes with something in it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

/// A terminal tool and the VS Code extensions for the same language.
const PAIRS: [(&str, &[&str]); 6] = [
    ("rust", &["rust-lang.rust-analyzer"]),
    ("python", &["ms-python.python"]),
    ("cpp", &["ms-vscode.cpptools"]),
    ("nodejs", &["dbaeumer.vscode-eslint", "esbenp.prettier-vscode"]),
    ("firebase", &["dbaeumer.vscode-eslint"]),
    ("claude-code", &["anthropic.claude-code"]),
];

/// Web apps that go with a tool or an app.
const SITES: [(&str, &[&str]); 6] = [
    ("firebase", &["google-firebase"]),
    ("gcloud", &["google-cloud"]),
    ("claude-code", &["claude"]),
    ("vscode", &["github", "claude"]),
    ("obs", &["youtube"]),
    ("openshot", &["youtube"]),
];

/// Each app's usual add-ons, after what goes with what's in it.
const USUAL_VSCODE: [&str; 3] = ["anthropic.claude-code", "eamodio.gitlens", "esbenp.prettier-vscode"];
const USUAL_TERMINAL: [&str; 3] = ["claude-code", "nodejs", "python"];
const USUAL_WADBROWSER: [&str; 3] = ["github", "claude", "google-drive"];

fn tool_name(id: &str) -> String {
    tool(id).map_or_else(|| id.to_string(), |t| t.name.clone())
}

fn ext_name(id: &str) -> String {
    extension(id).map_or_else(|| id.to_string(), |e| e.name.clone())
}

/// Up to `per` suggestions for each of `x`'s apps that has add-ons (VS Code, the terminal, WadBrowser), the likeliest first.
pub fn suggestions(x: &Wadspace, per: usize) -> Vec<Suggestion> {
    let has = |app: AppId, id: &str| x.exts.iter().any(|e| e.app == app && e.id == id);
    let has_site = |id: &str| {
        let url = site(id).map(|s| s.url);
        x.exts.iter().any(|e| e.app == AppId::Wadbrowser && (e.id == id || (url.is_some() && e.url.as_deref() == url)))
    };
    let tools_in: Vec<&str> = x.exts.iter().filter(|e| e.app == AppId::Terminal).map(|e| e.id.as_str()).collect();
    let exts_in: Vec<&str> = x.exts.iter().filter(|e| e.app == AppId::Vscode).map(|e| e.id.as_str()).collect();
    let mut out = vec![];
    let mut take = |app: AppId,
                    candidates: Vec<(String, Option<String>)>,
                    name: &dyn Fn(&str) -> String,
                    missing: &dyn Fn(&str) -> bool| {
        let mut seen: Vec<String> = vec![];
        for (id, why) in candidates {
            if seen.contains(&id) || !missing(&id) {
                continue;
            }
            seen.push(id.clone());
            out.push(Suggestion { app, item: format!("{app}:{id}"), name: name(&id), id, why });
            if seen.len() >= per {
                break;
            }
        }
    };
    if x.has_app(AppId::Vscode) {
        let mut c: Vec<(String, Option<String>)> = PAIRS
            .iter()
            .filter(|(t, _)| tools_in.contains(t))
            .flat_map(|(t, es)| es.iter().map(move |id| (id.to_string(), Some(format!("Goes with {}", tool_name(t))))))
            .collect();
        c.extend(USUAL_VSCODE.iter().map(|id| (id.to_string(), None)));
        c.extend(extensions().iter().map(|e| (e.id.clone(), None)));
        take(AppId::Vscode, c, &ext_name, &|id| !has(AppId::Vscode, id));
    }
    if x.has_app(AppId::Terminal) {
        let mut c: Vec<(String, Option<String>)> = PAIRS
            .iter()
            .filter_map(|(t, es)| {
                es.iter()
                    .find(|e| exts_in.contains(e))
                    .map(|e| (t.to_string(), Some(format!("Goes with {}", ext_name(e)))))
            })
            .collect();
        c.extend(USUAL_TERMINAL.iter().map(|id| (id.to_string(), None)));
        c.extend(tools().iter().map(|t| (t.id.clone(), None)));
        take(AppId::Terminal, c, &tool_name, &|id| !has(AppId::Terminal, id));
    }
    if x.has_app(AppId::Wadbrowser) {
        // What goes with its tools, then with its apps.
        let keys: Vec<(String, String)> = tools_in
            .iter()
            .map(|t| (t.to_string(), tool_name(t)))
            .chain(x.apps.iter().map(|a| (a.app.as_str().to_string(), app_name(a.app).to_string())))
            .collect();
        let mut c: Vec<(String, Option<String>)> = keys
            .iter()
            .flat_map(|(k, name)| {
                SITES.iter().filter(move |(s, _)| s == k).flat_map(move |(_, ids)| {
                    ids.iter().map(move |id| (id.to_string(), Some(format!("Goes with {name}"))))
                })
            })
            .collect();
        c.extend(USUAL_WADBROWSER.iter().map(|id| (id.to_string(), None)));
        c.extend(WEB_APPS.iter().map(|s| (s.id.to_string(), None)));
        let name = |id: &str| site(id).map_or_else(|| id.to_string(), |s| s.name.to_string());
        take(AppId::Wadbrowser, c, &name, &|id| site(id).is_some() && !has_site(id));
    }
    out
}
