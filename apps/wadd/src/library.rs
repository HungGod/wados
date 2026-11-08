//! The machine's wadspaces, tags and keys (wad_model's [`Library`]), and
//! every edit to them: wad_model's [`Op`]s, applied here for the shell, the
//! apps and the phone alike.
//!
//! An edit is applied to a copy, saved, and only then becomes the library:
//! if saving fails, nothing changed. Watchers hear what changed (each
//! wadspace's document; the whole library when tags or keys did). What an
//! edit took out can be put back (`undo`) while its snapshot is kept: the
//! last [`UNDOS`] of them, for a toast's Undo.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use wad_model::Wadspace;
use wad_model::edit::{self, Ctx, Library, Op, UndoSnapshot};
use wad_proto::{ApiError, Event, Level, OpResult};

use crate::events::Bus;
use crate::store::Store;

/// How many Undos are kept.
pub const UNDOS: usize = 64;

pub struct Docs {
    lib: Mutex<Library>,
    undos: Mutex<VecDeque<UndoSnapshot>>,
    store: Store,
    bus: Bus,
    machine: String,
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Everything but the wadspaces, to tell whether an edit changed it.
fn rest_of(lib: &Library) -> serde_json::Value {
    let mut v = serde_json::to_value(lib).unwrap_or_default();
    if let Some(o) = v.as_object_mut() {
        o.remove("wadspaces");
        o.remove("seq");
    }
    v
}

impl Docs {
    /// The library as saved; the first time (none yet), the WadOS wadspace
    /// every machine starts with.
    pub fn open(store: Store, bus: Bus, machine: &str) -> std::io::Result<Self> {
        Self::open_with(store, bus, machine, true)
    }

    /// As [`Self::open`]; `starter: false` leaves a new library empty (Set
    /// up offers WadOS instead, and the installer stick has no wadspaces).
    pub fn open_with(store: Store, bus: Bus, machine: &str, starter: bool) -> std::io::Result<Self> {
        let mut lib = store.load()?;
        if starter && lib.wadspaces.is_empty() && lib.seq == 0 {
            let w = wad_model::starter::starter_wadspace(machine, now_ms());
            store.save_wadspace(&w)?;
            lib.wadspaces.push(w);
            store.save_rest(&lib)?;
            tracing::info!("a new library, with WadOS in it");
        }
        // WadOS's rustup line, as an old one had it (it never built).
        for w in lib.wadspaces.iter_mut() {
            if wad_model::starter::fix_commands(w) {
                store.save_wadspace(w)?;
                tracing::info!("{}: its rustup command put right (--component rustfmt,clippy)", w.name);
            }
        }
        Ok(Self { lib: Mutex::new(lib), undos: Mutex::default(), store, bus, machine: machine.to_string() })
    }

    /// Set up's WadOS wadspace, if this machine hasn't one.
    pub fn add_starter(&self) -> Result<(), ApiError> {
        let mut lib = self.lib.lock().unwrap();
        let mut next = lib.clone();
        if wad_model::starter::add_starter(&mut next, &Ctx::new(now_ms(), self.machine.clone())) {
            let id = wad_model::starter::STARTER_ID.to_string();
            self.commit(&mut lib, next, std::slice::from_ref(&id))?;
        }
        Ok(())
    }

    pub fn snapshot(&self) -> Library {
        self.lib.lock().unwrap().clone()
    }

    pub fn wadspace(&self, id: &str) -> Option<Wadspace> {
        self.lib.lock().unwrap().wadspace(id).cloned()
    }

    /// This machine's id (what its wadspaces' `machine` is).
    pub fn machine(&self) -> &str {
        &self.machine
    }

    /// This machine's wadspaces not in the Trash, in order.
    pub fn live(&self) -> Vec<Wadspace> {
        let lib = self.lib.lock().unwrap();
        lib.on(&self.machine).into_iter().cloned().collect()
    }

    /// Applies one edit (all of it or none), saves it, tells the watchers.
    pub fn apply(&self, op: Op) -> Result<OpResult, ApiError> {
        let mut lib = self.lib.lock().unwrap();
        let mut next = lib.clone();
        let out = edit::apply(&mut next, op, &Ctx::new(now_ms(), self.machine.clone()))?;
        self.commit(&mut lib, next, &out.changed)?;
        let undo = out.undo.map(|u| {
            let id = u.id.clone();
            let mut undos = self.undos.lock().unwrap();
            undos.push_back(u);
            while undos.len() > UNDOS {
                undos.pop_front();
            }
            id
        });
        if let Some(t) = &out.toast {
            self.bus.publish(Event::notice(Level::Info, t.clone(), undo.clone()));
        }
        Ok(OpResult {
            changed: out.changed,
            created: out.created,
            undo,
            toast: out.toast,
            needs_build: out.needs_build,
        })
    }

    /// Deletes a wadspace in the Trash for good (its document; what wadd
    /// keeps for it is the caller's to clear). Its Undos go with it.
    pub fn purge(&self, id: &str) -> Result<OpResult, ApiError> {
        let mut lib = self.lib.lock().unwrap();
        let w = lib.wadspace(id).ok_or_else(|| ApiError::not_found(format!("no wadspace {id:?}")))?;
        if !w.is_trashed() {
            return Err(ApiError::conflict(format!("{} isn't in the Trash: put it there first", w.name)));
        }
        let mut next = lib.clone();
        next.wadspaces.retain(|x| x.id != id);
        let changed = vec![id.to_string()];
        self.commit(&mut lib, next, &changed)?;
        self.undos.lock().unwrap().retain(|u| u.wadspace.id != id);
        Ok(OpResult { changed, ..OpResult::default() })
    }

    /// Puts back what an edit took out (its Undo, once).
    pub fn undo(&self, id: &str) -> Result<OpResult, ApiError> {
        let snapshot = {
            let mut undos = self.undos.lock().unwrap();
            let i = undos
                .iter()
                .position(|u| u.id == id)
                .ok_or_else(|| ApiError::not_found(format!("nothing to undo for {id:?} (it's too old)")))?;
            undos.remove(i).expect("found")
        };
        let mut lib = self.lib.lock().unwrap();
        let mut next = lib.clone();
        let out = match edit::undo(&mut next, &snapshot) {
            Ok(out) => out,
            Err(e) => {
                self.undos.lock().unwrap().push_back(snapshot);
                return Err(e.into());
            }
        };
        self.commit(&mut lib, next, &out.changed)?;
        Ok(OpResult {
            changed: out.changed,
            created: out.created,
            undo: None,
            toast: out.toast,
            needs_build: out.needs_build,
        })
    }

    /// Saves what changed between `lib` and `next`, then makes `next` the library.
    fn commit(&self, lib: &mut Library, next: Library, changed: &[String]) -> Result<(), ApiError> {
        let fail = |e: std::io::Error| ApiError::internal(format!("saving the library: {e}"));
        let mut gone = vec![];
        for id in changed {
            match next.wadspace(id) {
                Some(w) => self.store.save_wadspace(w).map_err(fail)?,
                None => {
                    self.store.delete_wadspace(id).map_err(fail)?;
                    gone.push(id.clone());
                }
            }
        }
        let rest_changed = rest_of(lib) != rest_of(&next);
        if rest_changed || lib.seq != next.seq {
            self.store.save_rest(&next).map_err(fail)?;
        }
        *lib = next;
        for id in changed {
            if let Some(w) = lib.wadspace(id) {
                self.bus.publish(Event::Wadspace(Box::new(w.clone())));
            }
        }
        for id in gone {
            self.bus.publish(Event::WadspaceGone { id });
        }
        if rest_changed {
            self.bus.publish(Event::Library(Box::new(lib.clone())));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wad_proto::{MachineInfo, Profile};

    fn docs(dir: &std::path::Path) -> (Docs, Bus) {
        let bus = Bus::new(MachineInfo {
            id: "m-test".into(),
            name: "Test".into(),
            version: "0".into(),
            profile: Profile::User,
        });
        (Docs::open(Store::new(dir).unwrap(), bus.clone(), "m-test").unwrap(), bus)
    }

    fn create(d: &Docs, name: &str) -> String {
        let op: Op = serde_json::from_value(serde_json::json!({"op": "space.create", "name": name})).unwrap();
        d.apply(op).unwrap().created.expect("a new wadspace")
    }

    #[test]
    fn a_new_machine_starts_with_wados() {
        let dir = tempfile::tempdir().unwrap();
        let (d, _) = docs(dir.path());
        let live = d.live();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].name, "WadOS");
        assert!(live[0].starter);
    }

    #[test]
    fn an_edit_is_saved_and_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let id = {
            let (d, _) = docs(dir.path());
            create(&d, "Studio")
        };
        let (d, _) = docs(dir.path());
        assert_eq!(d.wadspace(&id).map(|w| w.name), Some("Studio".into()));
        assert_eq!(d.live().len(), 2);
    }

    #[test]
    fn the_browsers_site_keys_are_saved_and_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let id = {
            let (d, bus) = docs(dir.path());
            let (_, mut rx) = bus.subscribe();
            let save: Op = serde_json::from_value(
                serde_json::json!({"op": "secrets.saveWeb", "name": "Work", "site": "github.com"}),
            )
            .unwrap();
            let id = d.apply(save).unwrap().created.expect("a key");
            let more: Op = serde_json::from_value(
                serde_json::json!({"op": "secrets.saveWeb", "name": "", "site": "gitlab.com", "existing": id}),
            )
            .unwrap();
            d.apply(more).unwrap();
            let less: Op = serde_json::from_value(
                serde_json::json!({"op": "secrets.removeSite", "key": id, "site": "github.com"}),
            )
            .unwrap();
            d.apply(less).unwrap();
            // Keys aren't a wadspace's: watchers hear the whole library.
            let mut heard = false;
            while let Ok(e) = rx.try_recv() {
                heard |= matches!(&e, Event::Library(l) if l.keys.iter().any(|k| k.id == id));
            }
            assert!(heard);
            id
        };
        let (d, _) = docs(dir.path());
        let k = d.snapshot().keys.into_iter().find(|k| k.id == id).expect("kept");
        assert_eq!(k.sites, ["gitlab.com"]);
    }

    #[test]
    fn what_an_edit_took_out_can_be_put_back_once() {
        let dir = tempfile::tempdir().unwrap();
        let (d, _) = docs(dir.path());
        let id = d.live()[0].id.clone();
        let remove: Op =
            serde_json::from_value(serde_json::json!({"op": "space.remove", "ws": id, "chip": "app:vscode"})).unwrap();
        let out = d.apply(remove).unwrap();
        assert!(!d.wadspace(&id).unwrap().apps.iter().any(|a| a.app == wad_model::AppId::Vscode));
        let undo = out.undo.expect("an Undo");
        d.undo(&undo).unwrap();
        assert!(d.wadspace(&id).unwrap().apps.iter().any(|a| a.app == wad_model::AppId::Vscode));
        assert!(d.undo(&undo).is_err(), "only once");
    }

    #[test]
    fn a_refused_edit_changes_nothing_and_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let (d, _) = docs(dir.path());
        let before = d.snapshot();
        let rename: Op =
            serde_json::from_value(serde_json::json!({"op": "builder.rename", "ws": "no-such", "name": "X"})).unwrap();
        let e = d.apply(rename).unwrap_err();
        assert_eq!(e.code, wad_proto::ErrorCode::NotFound);
        assert_eq!(d.snapshot(), before);
    }

    #[test]
    fn watchers_hear_each_changed_wadspace() {
        let dir = tempfile::tempdir().unwrap();
        let (d, bus) = docs(dir.path());
        let (_, mut rx) = bus.subscribe();
        let id = create(&d, "Writing");
        let mut saw = false;
        while let Ok(e) = rx.try_recv() {
            if matches!(&e, Event::Wadspace(w) if w.id == id) {
                saw = true;
            }
        }
        assert!(saw);
    }
}
