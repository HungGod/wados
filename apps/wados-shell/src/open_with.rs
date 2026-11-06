//! Open with, from the shell (the spike's `openWithMenu` and
//! `files.openWith`): the desktop's files opened with an app, in a
//! wadspace of this machine's that has it or a new one for it, mounted
//! for that run only. The app is remembered for its kind of file
//! (`wad_gtk::opened`), so a double-click opens the next one with it
//! ([`default_for`]); with none, Files' viewers show it.

use std::rc::Rc;

use wad_model::open_with::{self, Opening, Row};
use wad_model::{AppId, catalog};
use wad_proto::Level;

use crate::shell::Shell;

/// A file to open: its path, its name, its type, whether it's a folder.
#[derive(Debug, Clone)]
pub struct File {
    pub path: String,
    pub name: String,
    pub mime: String,
    pub folder: bool,
}

impl File {
    fn opening(&self) -> Opening<'_> {
        Opening::new(&self.name, Some(&self.mime).filter(|m| !m.is_empty()).map(String::as_str), self.folder)
    }
}

/// Open with's rows for `files`, with the app that last opened their kind ticked.
pub fn rows(shell: &Shell, files: &[File]) -> Vec<Row> {
    let open: Vec<Opening> = files.iter().map(File::opening).collect();
    let last = files.first().and_then(|f| wad_gtk::opened::load().get(&f.opening().mime()));
    open_with::rows(&open, &shell.store.live(), last)
}

/// What a double-click opens `f` with (the app that last opened its kind, in a space that has it).
pub fn default_for(shell: &Shell, f: &File) -> Option<(AppId, String)> {
    open_with::remembered_for(&f.opening(), &shell.store.live(), &wad_gtk::opened::load())
}

/// `files` opened with `app` in `ws` (none: a new wadspace for it).
pub fn open(shell: &Rc<Shell>, files: &[File], app: AppId, ws: Option<String>) {
    let Some(first) = files.first() else { return };
    wad_gtk::opened::remember(&first.opening().mime(), app);
    let what = match files.len() {
        1 => first.name.clone(),
        n => format!("{} and {} more", first.name, n - 1),
    };
    let space = ws.as_deref().and_then(|w| shell.store.wadspace(w)).map(|x| x.name);
    let title = match &space {
        Some(s) => format!("Opening {what} with {} in {s}", catalog::app_name(app)),
        None => format!("Opening {what} with {} in a new space", catalog::app_name(app)),
    };
    shell.toast(Level::Info, &title, None);
    let paths: Vec<String> = files.iter().map(|f| f.path.clone()).collect();
    let me = shell.clone();
    shell.link.call(
        move |w| async move { w.open_files_with(app, ws.as_deref(), &paths, None).await },
        move |r| match r {
            Ok((id, _)) => me.placer.run_changed(&id),
            Err(e) => me.toast(Level::Warn, &format!("{what} didn't open: {}", e.message), None),
        },
    );
}
