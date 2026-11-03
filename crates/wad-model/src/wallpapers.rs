// Backgrounds: pictures (WadSpaces' own art, a photo from ~/Pictures, or
// one uploaded there), a plain colour, or none. The old pictures made from
// the projects' art, the gradients, the textures, Glow and Prism are gone.

use crate::types::Wallpaper;

/// One of WadSpaces' pictures: original art (scripts/art.py, SVG scenes and patterns, each its own palette).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Picture {
    pub id: &'static str,
    pub name: &'static str,
}

impl Picture {
    /// Its file ('/wallpapers/art/dunes.svg').
    pub fn src(&self) -> String {
        art_src(self.id)
    }

    pub fn wallpaper(&self) -> Wallpaper {
        Wallpaper::image(self.src())
    }
}

/// The pictures, in the order a new wadspace takes them. The last two are
/// WadSpaces' own, in its colours (lime, pink, deep violet): Duo is the
/// desktop's, Silk the lock screen's.
pub const PICTURES: [Picture; 16] = [
    Picture { id: "dunes", name: "Dunes" },
    Picture { id: "fjord", name: "Fjord" },
    Picture { id: "aurora", name: "Northern Lights" },
    Picture { id: "circuit", name: "Circuit" },
    Picture { id: "bloom", name: "Bloom" },
    Picture { id: "tide", name: "Tide" },
    Picture { id: "mesa", name: "Mesa" },
    Picture { id: "skyline", name: "Skyline" },
    Picture { id: "koi", name: "Koi" },
    Picture { id: "moss", name: "Moss" },
    Picture { id: "retro", name: "Retro" },
    Picture { id: "lagoon", name: "Lagoon" },
    Picture { id: "ember", name: "Embers" },
    Picture { id: "orchard", name: "Orchard" },
    Picture { id: "duo", name: "Duo" },
    Picture { id: "silk", name: "Silk" },
];

fn art_src(id: &str) -> String {
    format!("/wallpapers/art/{id}.svg")
}

/// A picture, by its id (its file in /wallpapers/art).
pub fn art_of(id: &str) -> Wallpaper {
    Wallpaper::image(art_src(id))
}

/// The picture a wallpaper is, if it's one of WadSpaces'.
pub fn picture_of(w: &Wallpaper) -> Option<&'static Picture> {
    let Wallpaper::Image { value } = w else { return None };
    PICTURES.iter().find(|p| p.src() == *value)
}

/// One of WadSpaces' pictures.
pub fn is_art(w: &Wallpaper) -> bool {
    picture_of(w).is_some()
}

/// The picture a wadspace gets by default: the first none of `taken` has (then round again).
pub fn default_art<'a>(taken: impl IntoIterator<Item = &'a Wallpaper>) -> Wallpaper {
    let mut used = Vec::new();
    let mut n = 0;
    for w in taken {
        n += 1;
        if let Wallpaper::Image { value } = w {
            used.push(value.as_str());
        }
    }
    let free = PICTURES.iter().find(|p| !used.contains(&p.src().as_str())).unwrap_or(&PICTURES[n % PICTURES.len()]);
    free.wallpaper()
}

/// A background picked (not a new wadspace's none: its icon's glass, empty; its desktop, plain).
pub fn has_picture(w: &Wallpaper) -> bool {
    !matches!(w, Wallpaper::Empty)
}

/// The desktop's background to start with: Duo (WadSpaces' own art: its pink and lime lights).
pub fn default_desktop() -> Wallpaper {
    art_of("duo")
}

/// The lock screen's: Silk (its threads).
pub fn default_lock() -> Wallpaper {
    art_of("silk")
}

/// A short name for it ("Northern Lights", "moodboard.png", "Empty").
pub fn wallpaper_name(w: &Wallpaper) -> String {
    match w {
        Wallpaper::Empty => "Empty".to_string(),
        Wallpaper::Color { value } => value.clone(),
        Wallpaper::Image { value } => match picture_of(w) {
            Some(p) => p.name.to_string(),
            None if value.starts_with("data:") => "Picture".to_string(),
            None => value.rsplit('/').next().unwrap_or(value).to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_only() {
        let names: Vec<_> = PICTURES.iter().map(|p| p.name).collect();
        assert!(!names.contains(&"Glow") && !names.contains(&"Prism"));
        assert_eq!(default_desktop(), Wallpaper::image("/wallpapers/art/duo.svg"));
        assert_eq!(default_lock(), art_of("silk"));
        assert_eq!(wallpaper_name(&art_of("aurora")), "Northern Lights");
        assert_eq!(wallpaper_name(&Wallpaper::image("~/Pictures/moodboard.png")), "moodboard.png");
        assert_eq!(wallpaper_name(&Wallpaper::Empty), "Empty");
        assert!(!has_picture(&Wallpaper::Empty));
        assert!(has_picture(&Wallpaper::color("#17112a")));
    }

    #[test]
    fn a_new_one_gets_the_first_picture_nobody_has() {
        assert_eq!(default_art([]), art_of("dunes"));
        let taken = [art_of("dunes"), Wallpaper::Empty, art_of("aurora")];
        assert_eq!(default_art(&taken), art_of("fjord"));
        let all: Vec<_> = PICTURES.iter().map(Picture::wallpaper).collect();
        // All taken: round again, by how many there are.
        assert_eq!(default_art(&all), PICTURES[0].wallpaper());
    }
}
