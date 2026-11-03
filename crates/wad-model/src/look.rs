// A wadspace is one look: what its card shows is all that's in it. Its
// picture, its icon (a square cut out of the picture), its files and
// folders, what opens with it, what it starts straight into, and the order
// its desktop's icons are in.

use serde::{Deserialize, Serialize};

use crate::catalog::is_named;
use crate::types::{AppId, AppIn, Ext, IconLook, Project, Wadspace, Wallpaper, double_option};
use crate::wallpapers::{default_art, is_art};

/// The picture's shape (it fills a 16:10 screen).
pub const ICON_ASPECT: f64 = 1.6;
/// How far an icon can zoom into its picture.
pub const ICON_ZOOM_MIN: f64 = 1.0;
pub const ICON_ZOOM_MAX: f64 = 3.0;

/// A change to an icon's look: each part left as it is (absent), put back
/// (null: zoom 1, the middle, its background) or set.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IconPatch {
    #[serde(default, skip_serializing_if = "Option::is_none", with = "double_option")]
    pub zoom: Option<Option<f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "double_option")]
    pub x: Option<Option<f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "double_option")]
    pub y: Option<Option<f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "double_option")]
    pub picture: Option<Option<Wallpaper>>,
}

impl IconPatch {
    /// `base` with this on top, kept in bounds.
    pub fn apply(&self, base: Option<&IconLook>) -> IconLook {
        let d = IconLook::default();
        let base = base.cloned().unwrap_or_default();
        let pick = |p: &Option<Option<f64>>, was: f64, default: f64| match p {
            None => was,
            Some(None) => default,
            Some(Some(v)) => *v,
        };
        let picture = match &self.picture {
            None => base.picture.clone(),
            Some(p) => p.clone(),
        };
        clamp_icon(IconLook {
            zoom: pick(&self.zoom, base.zoom, d.zoom),
            x: pick(&self.x, base.x, d.x),
            y: pick(&self.y, base.y, d.y),
            picture,
        })
    }
}

/// An icon's look kept in bounds: the square inside the picture (anything not a number: the default).
pub fn clamp_icon(i: IconLook) -> IconLook {
    let finite = |v: f64, d: f64| if v.is_finite() { v } else { d };
    let zoom = finite(i.zoom, 1.0).clamp(ICON_ZOOM_MIN, ICON_ZOOM_MAX);
    // The square's half, in the picture's widths and heights.
    let hx = 1.0 / (2.0 * ICON_ASPECT * zoom);
    let hy = 1.0 / (2.0 * zoom);
    let within = |v: f64, h: f64| finite(v, 0.5).clamp(h, 1.0 - h);
    IconLook { zoom, x: within(i.x, hx), y: within(i.y, hy), picture: i.picture }
}

/// Its icon's look.
pub fn icon_of(x: &Wadspace) -> IconLook {
    clamp_icon(x.look.icon.clone().unwrap_or_default())
}

/// The picture its icon is cut out of: one of its own, or its background.
pub fn icon_picture_of(x: &Wadspace) -> &Wallpaper {
    x.look.icon.as_ref().and_then(|i| i.picture.as_ref()).unwrap_or(&x.look.wallpaper)
}

/// Every one of `list` without one of WadSpaces' pictures (none, or a photo) given the first free one, so no two look alike (`all`: every one of them, afresh).
pub fn give_art(list: &mut [Wadspace], all: bool) {
    let mut taken: Vec<Wallpaper> = if all {
        Vec::new()
    } else {
        list.iter().filter(|x| is_art(&x.look.wallpaper)).map(|x| x.look.wallpaper.clone()).collect()
    };
    for x in list.iter_mut() {
        if !all && is_art(&x.look.wallpaper) {
            continue;
        }
        let wp = default_art(&taken);
        taken.push(wp.clone());
        x.look.wallpaper = wp;
    }
}

/// The icons a wadspace's desktop has to begin with: its apps (the terminal last), its web apps, its named variants, then its projects.
pub fn default_icons(apps: &[AppIn], exts: &[Ext], projects: &[Project]) -> Vec<String> {
    let mut icons: Vec<String> =
        apps.iter().filter(|a| a.app != AppId::Terminal).map(|a| format!("app:{}", a.app)).collect();
    icons
        .extend(exts.iter().filter(|e| e.app == AppId::Wadbrowser && e.url.is_some()).map(|e| format!("web:{}", e.id)));
    icons.extend(exts.iter().filter(|e| is_named(e)).map(|e| format!("variant:{}", e.id)));
    if apps.iter().any(|a| a.app == AppId::Terminal) {
        icons.push("app:terminal".to_string());
    }
    icons.extend(projects.iter().map(|p| format!("project:{}", p.id)));
    icons
}

/// Does `key` ('app:krita', 'web:gmail', 'variant:hades', 'project:iq') name something in `x`?
pub fn resolves(x: &Wadspace, key: &str) -> bool {
    let Some((kind, id)) = key.split_once(':') else { return false };
    match kind {
        "app" => AppId::parse(id).is_some_and(|a| x.has_app(a)),
        "web" => x.exts.iter().any(|e| e.app == AppId::Wadbrowser && e.id == id && e.url.is_some()),
        "variant" => x.exts.iter().any(|e| e.id == id),
        "project" => x.look.projects.iter().any(|p| p.id == id),
        _ => false,
    }
}

/// Can `x` start straight into `key`: one of its apps, web apps or games ('app:krita', 'web:gmail', 'variant:hades')?
pub fn can_launch(x: &Wadspace, key: &str) -> bool {
    match key.split_once(':') {
        Some(("app" | "web", _)) => resolves(x, key),
        Some(("variant", id)) => x.exts.iter().any(|e| e.id == id && is_named(e)),
        _ => false,
    }
}

/// What's gone from `x` gone from its look too: its desktop's icons, what opens with it, and what it starts straight into.
pub fn prune(x: &mut Wadspace) {
    let keep = |x: &Wadspace, k: &str| resolves(x, k);
    let icons: Vec<String> = x.look.layout.icons.iter().filter(|k| keep(x, k)).cloned().collect();
    let autostart: Vec<String> = x.look.autostart.iter().filter(|k| keep(x, k)).cloned().collect();
    x.look.layout.icons = icons;
    x.look.autostart = autostart;
    if x.look.launch.as_deref().is_some_and(|k| !can_launch(x, k)) {
        x.look.launch = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallpapers::art_of;

    #[test]
    fn an_icon_stays_inside_its_picture() {
        // At 1×, the square's as tall as the picture: it can only move across, as far as its edges.
        let i = IconPatch { x: Some(Some(0.0)), y: Some(Some(0.9)), ..Default::default() }.apply(None);
        assert_eq!(i, IconLook { zoom: 1.0, x: 0.3125, y: 0.5, picture: None });
        let i = IconPatch { zoom: Some(Some(9.0)), y: Some(Some(0.9)), ..Default::default() }.apply(Some(&i));
        assert_eq!(i.zoom, 3.0);
        assert!((i.y - (1.0 - 1.0 / 6.0)).abs() < 1e-9);
        // Put back: the middle, zoom 1; a picture of its own kept unless it's unset too.
        let own = art_of("ember");
        let i = IconPatch { picture: Some(Some(own.clone())), ..Default::default() }.apply(Some(&i));
        let i = IconPatch { zoom: Some(None), x: Some(None), y: Some(None), ..Default::default() }.apply(Some(&i));
        assert_eq!(i, IconLook { picture: Some(own), ..IconLook::default() });
        assert_eq!(
            clamp_icon(IconLook { zoom: f64::NAN, x: f64::INFINITY, ..IconLook::default() }),
            IconLook::default()
        );
    }

    #[test]
    fn icon_patches_read_absent_null_and_set() {
        let p: IconPatch = serde_json::from_str(r#"{"x": 0.2, "picture": null}"#).expect("a patch");
        assert_eq!(p, IconPatch { x: Some(Some(0.2)), picture: Some(None), ..Default::default() });
        assert_eq!(serde_json::to_string(&p).ok().as_deref(), Some(r#"{"x":0.2,"picture":null}"#));
    }

    #[test]
    fn default_icons_put_the_terminal_after_the_web_apps_and_games() {
        let apps = [AppIn::new(AppId::Terminal), AppIn::new(AppId::Wadbrowser), AppIn::new(AppId::Steam)];
        let exts = [
            Ext::new(AppId::Wadbrowser, "gmail").at("https://mail.google.com"),
            crate::catalog::focus(),
            Ext::new(AppId::Steam, "hades"),
        ];
        assert_eq!(
            default_icons(&apps, &exts, &[]),
            ["app:wadbrowser", "app:steam", "web:gmail", "variant:hades", "app:terminal"]
        );
    }
}
