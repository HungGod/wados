// Gamespaces' games, the pure parts (downloading, staging and signing in
// are wadd's jobs). A Steam account is a sign-in (a key of kind 'steam');
// signed in on this machine once, its library is one folder on the host,
// mounted into every gamespace that signs in with it: a game downloads once
// however many spaces have it, and Steam runs one client per account, so
// one of them plays at a time. Dolphin's games are discs (.iso, .rvz)
// uploaded to the account's GitHub packages, and need no account.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::catalog::{Game, steam_game, steam_games};
use crate::types::{AppId, DiscImage, SteamLibrary, Wadspace};
use crate::variants::{find_variant, sign_in_of};

/// What a new account owns (the mock store has no real sign-in to ask).
pub const NEW_ACCOUNT_GAMES: [&str; 4] = ["celeste", "hollow-knight", "portal-2", "terraria"];

/// A game by its id: the store's, or one named by hand (no size known).
pub fn game_of(id: &str, name: Option<&str>) -> Game {
    steam_game(id).cloned().unwrap_or_else(|| Game {
        id: id.to_string(),
        name: name.unwrap_or(id).to_string(),
        appid: 0,
        size_mb: 2000,
        cover: "Midnight".to_string(),
        dlc: vec![],
    })
}

/// The Steam game a wadspace plays, and the account it signs in with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SteamTarget {
    pub game: Game,
    pub account: Option<String>,
}

/// The Steam game `x` starts straight into (or `open`, what it was asked to open: 'variant:hades'), and the account it signs in with.
pub fn steam_target(x: &Wadspace, open: Option<&str>) -> Option<SteamTarget> {
    let key = open.or(x.look.launch.as_deref())?;
    let id = key.strip_prefix("variant:")?;
    let e = x.ext(AppId::Steam, id)?;
    let account = find_variant(x, &format!("ext:steam:{}", e.id)).and_then(|v| sign_in_of(x, &v)).map(|s| s.key.id);
    Some(SteamTarget { game: game_of(&e.id, e.name.as_deref()), account })
}

/// The game a gamespace plays (its look's launch), if it's a Steam one.
pub fn plays_steam(x: &Wadspace) -> Option<SteamTarget> {
    steam_target(x, None)
}

/// Another of `running` (wadspaces' ids) playing on the same Steam account as `x` (Steam runs one client per account).
pub fn steam_busy<'a>(list: &'a [Wadspace], running: &[&str], x: &Wadspace) -> Option<&'a Wadspace> {
    let mine = plays_steam(x)?.account?;
    running
        .iter()
        .filter(|id| **id != x.id)
        .filter_map(|id| list.iter().find(|y| y.id == *id))
        .find(|y| steam_target(y, None).and_then(|t| t.account).as_deref() == Some(mine.as_str()))
}

/// Is `game` downloaded into `account`'s library here?
pub fn installed(steam: &IndexMap<String, SteamLibrary>, account: Option<&str>, game: &str) -> bool {
    account.and_then(|a| steam.get(a)).is_some_and(|l| l.has(game))
}

/// A gamespace whose public stream can be multiplayer (Play together): Dolphin's, or a Steam game's local co-op.
pub fn plays_together(x: &Wadspace) -> bool {
    x.has_app(AppId::Dolphin) || x.has_app(AppId::Steam)
}

/// A game an account signed in here owns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnedGame {
    pub account: String,
    pub game: Game,
    pub installed: bool,
}

/// Every Steam game the signed-in accounts own, with the account.
pub fn owned_games(steam: &IndexMap<String, SteamLibrary>) -> Vec<OwnedGame> {
    steam
        .iter()
        .flat_map(|(account, lib)| {
            lib.owned.iter().map(move |id| OwnedGame {
                account: account.clone(),
                game: steam_games().iter().find(|g| g.id == *id).cloned().unwrap_or_else(|| game_of(id, None)),
                installed: lib.has(id),
            })
        })
        .collect()
}

/// Where a game's package goes in the account's GitHub packages.
pub fn game_package_ref(login: &str, game: &str) -> String {
    let tag = steam_game(game).map_or_else(|| "latest".to_string(), |g| g.appid.to_string());
    format!("ghcr.io/{login}/wadspaces-game-{game}:{tag}")
}

// ------------------------------------------------------------ Dolphin's discs

/// A disc's file: an .iso or an .rvz.
pub fn is_disc_file(file: &str) -> bool {
    without_disc_ext(file).len() < file.len()
}

/// The file's name without its .iso or .rvz.
fn without_disc_ext(file: &str) -> &str {
    let at = file.len().saturating_sub(4);
    match (file.get(..at), file.get(at..)) {
        (Some(head), Some(ext)) if ext.eq_ignore_ascii_case(".iso") || ext.eq_ignore_ascii_case(".rvz") => head,
        _ => file,
    }
}

/// Its variant's id, from its file ('Mario-Kart_Double Dash.rvz' → 'mario-kart-double-dash').
pub fn disc_id(file: &str) -> String {
    crate::names::slug(without_disc_ext(file))
}

/// Its name, from its file ('mario-kart-double-dash.rvz' → 'Mario Kart Double Dash').
pub fn disc_name(file: &str) -> String {
    let spaced: String =
        without_disc_ext(file).chars().map(|c| if matches!(c, '-' | '_' | '.') { ' ' } else { c }).collect();
    let mut out = String::with_capacity(spaced.len());
    let mut word = false;
    for c in spaced.split_whitespace().collect::<Vec<_>>().join(" ").chars() {
        let w = c.is_ascii_alphanumeric() || c == '_';
        if w && !word {
            out.extend(c.to_uppercase())
        } else {
            out.push(c)
        }
        word = w;
    }
    out
}

/// Where a disc's package goes.
pub fn disc_package_ref(login: &str, id: &str) -> String {
    format!("ghcr.io/{login}/wadspaces-disc-{id}:latest")
}

/// Why a disc can't be uploaded now (GitHub's login, if it's linked; whether this machine is online), or none.
pub fn disc_problem(file: &str, login: Option<&str>, online: bool) -> Option<&'static str> {
    if !is_disc_file(file) {
        Some("A disc is an .iso or an .rvz file")
    } else if login.is_none() {
        Some("Link GitHub first: discs go to your packages")
    } else if !online {
        Some("This machine is offline")
    } else {
        None
    }
}

/// The Dolphin game a wadspace plays, and its disc in your packages if it's there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DolphinTarget {
    pub name: String,
    pub disc: Option<DiscImage>,
}

/// The Dolphin game `x` starts straight into (or `open`, what it was asked to open), and its disc among `discs`.
pub fn dolphin_target(discs: &IndexMap<String, DiscImage>, x: &Wadspace, open: Option<&str>) -> Option<DolphinTarget> {
    let key = open.or(x.look.launch.as_deref())?;
    let id = key.strip_prefix("variant:")?;
    let e = x.ext(AppId::Dolphin, id)?;
    let disc = discs.values().find(|d| d.id == e.id || e.name.as_deref() == Some(d.name.as_str())).cloned();
    Some(DolphinTarget { name: e.name.clone().unwrap_or_else(|| e.id.clone()), disc })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discs_by_their_files() {
        assert!(is_disc_file("super-smash-bros-melee.RVZ"));
        assert!(!is_disc_file("iso") && !is_disc_file("ľiso") && !is_disc_file("disc.isoé"));
        assert_eq!(disc_id("ééé.iso"), "");
        assert!(!is_disc_file("notes.txt"));
        assert_eq!(disc_id("Mario-Kart_Double Dash.rvz"), "mario-kart-double-dash");
        assert_eq!(disc_name("mario-kart-double-dash.rvz"), "Mario Kart Double Dash");
        assert_eq!(disc_name("super_smash.bros  melee.iso"), "Super Smash Bros Melee");
        assert_eq!(disc_problem("a.zip", Some("me"), true), Some("A disc is an .iso or an .rvz file"));
        assert_eq!(disc_problem("a.iso", None, true), Some("Link GitHub first: discs go to your packages"));
        assert_eq!(disc_problem("a.iso", Some("me"), false), Some("This machine is offline"));
        assert_eq!(disc_problem("a.iso", Some("me"), true), None);
        assert_eq!(disc_package_ref("hunggod", "mario-party-4"), "ghcr.io/hunggod/wadspaces-disc-mario-party-4:latest");
        assert_eq!(game_package_ref("hunggod", "hollow-knight"), "ghcr.io/hunggod/wadspaces-game-hollow-knight:367520");
    }

    #[test]
    fn a_game_by_hand_has_a_guess_at_its_size() {
        assert_eq!(game_of("stardew-valley", None).appid, 413_150);
        let g = game_of("my-game", Some("My Game"));
        assert_eq!((g.name.as_str(), g.size_mb, g.appid), ("My Game", 2000, 0));
    }
}
