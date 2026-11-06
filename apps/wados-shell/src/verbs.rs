//! What a notice's button does (wad-proto's `NoticeAction`): a verb of the
//! shell's with its arguments. Others aren't known, and their button isn't
//! shown.

use std::rc::Rc;

use wad_model::SessionDisplay;

use crate::session::Power;
use crate::shell::Shell;

const VERBS: [&str; 4] = ["power.restart", "settings.open", "app.open", "wadspace.open"];

pub fn known(verb: &str) -> bool {
    VERBS.contains(&verb)
}

/// The program that is one of the machine's apps (`files`, `updates`…).
fn app_argv(app: &str) -> Vec<&str> {
    match app {
        "files" => vec!["wados-files"],
        "trash" => vec!["wados-files", "--trash"],
        "settings" => vec!["wados-settings"],
        "updates" => vec!["wados-settings", "--page", "updates"],
        "storage" => vec!["wados-settings", "--page", "storage"],
        "browser" | "wadbrowser" => vec!["wadbrowser"],
        _ => vec![],
    }
}

pub fn run(shell: &Rc<Shell>, verb: &str, args: &[String]) {
    let first = args.first().map(String::as_str).unwrap_or("");
    match verb {
        "power.restart" => shell.power(Power::Restart),
        "settings.open" => {
            let mut argv = vec!["wados-settings"];
            if !first.is_empty() {
                argv.extend(["--page", first]);
            }
            crate::peers::open_app(shell, &argv);
        }
        "app.open" => {
            let argv = app_argv(first);
            if argv.is_empty() {
                tracing::info!("app.open: no app {first:?}");
            } else {
                crate::peers::open_app(shell, &argv);
            }
        }
        "wadspace.open" if !first.is_empty() => shell.open(first, SessionDisplay::Screen),
        _ => tracing::info!("a notice's verb {verb:?} isn't the shell's"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_verbs_it_knows() {
        assert!(known("power.restart") && known("wadspace.open") && !known("rm.rf"));
        assert_eq!(app_argv("files"), ["wados-files"]);
        assert_eq!(app_argv("updates"), ["wados-settings", "--page", "updates"]);
        assert!(app_argv("nope").is_empty());
    }
}
