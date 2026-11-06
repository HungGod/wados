//! The displays as data: a [`Head`] (one display, as labwc's output
//! management says it is), what to ask for one ([`HeadConf`]), the layouts
//! kept for each set of displays (`$XDG_CONFIG_HOME/wados/displays.toml`,
//! as kanshi keeps profiles), which display is the primary, and where a
//! display dragged in the arrangement lands (the spike's `arrange.ts`).
//!
//! A layout is kept for a set of displays (the laptop alone; the laptop
//! and the two desk monitors), each known by its make, model and serial
//! (its connector too when it has no serial), so plugging the desk back in
//! brings back how it was arranged there. Settings saves one when a change
//! is kept; the shell puts it back when the set it's for appears (at its
//! start, or a display plugged in). The primary is the layout's too.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A display mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mode {
    pub width: i32,
    pub height: i32,
    /// mHz (60000: 60 Hz); 0 when it doesn't say.
    pub refresh: i32,
    #[serde(default)]
    pub preferred: bool,
}

impl Mode {
    pub fn same_size(&self, o: &Mode) -> bool {
        self.width == o.width && self.height == o.height
    }

    /// Ignoring `preferred`.
    pub fn same(&self, o: &Mode) -> bool {
        self.same_size(o) && self.refresh == o.refresh
    }
}

/// A display's rotation (wl_output's transform).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Transform {
    #[default]
    Normal,
    R90,
    R180,
    R270,
    Flipped,
    Flipped90,
    Flipped180,
    Flipped270,
}

impl Transform {
    /// wl_output's number for it.
    pub fn wl(self) -> u32 {
        self as u32
    }

    pub fn from_wl(n: u32) -> Self {
        use Transform::*;
        [Normal, R90, R180, R270, Flipped, Flipped90, Flipped180, Flipped270].get(n as usize).copied().unwrap_or(Normal)
    }

    /// On its side: its width is its mode's height.
    pub fn sideways(self) -> bool {
        matches!(self, Transform::R90 | Transform::R270 | Transform::Flipped90 | Transform::Flipped270)
    }
}

/// One display, as labwc says it is.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Head {
    /// Its connector: "eDP-1", "DP-3", "HEADLESS-1".
    pub name: String,
    pub description: String,
    pub make: String,
    pub model: String,
    pub serial: String,
    /// Its size in mm (0 when it doesn't say).
    pub physical: (i32, i32),
    pub enabled: bool,
    pub modes: Vec<Mode>,
    /// Its mode now (when it's on): one of `modes`, or a custom one.
    pub mode: Option<Mode>,
    pub x: i32,
    pub y: i32,
    pub transform: Transform,
    pub scale: f64,
}

impl Head {
    /// Who it is, across starts and connectors (a layout's key).
    pub fn key(&self) -> String {
        let id = [self.make.trim(), self.model.trim(), self.serial.trim()].join("|");
        if self.serial.trim().is_empty() { format!("{id}|{}", self.name) } else { id }
    }

    /// The laptop's own screen.
    pub fn built_in(&self) -> bool {
        ["eDP", "LVDS", "DSI"].iter().any(|p| self.name.starts_with(p))
    }

    /// Its size in the desktop's points (its mode over its scale, turned with it).
    pub fn logical(&self) -> (i32, i32) {
        let Some(m) = self.mode else { return (0, 0) };
        let s = if self.scale > 0.0 { self.scale } else { 1.0 };
        let (w, h) = ((m.width as f64 / s).round() as i32, (m.height as f64 / s).round() as i32);
        if self.transform.sideways() { (h, w) } else { (w, h) }
    }

    /// What it's called: "Built-in Display", or its make and model.
    pub fn label(&self) -> String {
        if self.built_in() {
            return "Built-in Display".into();
        }
        let known =
            |s: &str| !s.trim().is_empty() && !matches!(s.trim().to_lowercase().as_str(), "unknown" | "headless");
        let mm =
            [self.make.as_str(), self.model.as_str()].into_iter().filter(|s| known(s)).collect::<Vec<_>>().join(" ");
        if !mm.is_empty() {
            return mm;
        }
        if known(&self.description) {
            return self.description.clone();
        }
        self.name.clone()
    }

    /// What to ask for to keep it as it is.
    pub fn conf(&self) -> HeadConf {
        HeadConf {
            name: self.name.clone(),
            enabled: self.enabled,
            mode: self.mode,
            x: self.x,
            y: self.y,
            transform: self.transform,
            scale: if self.scale > 0.0 { self.scale } else { 1.0 },
        }
    }

    /// Its refresh rates at `size`, highest first.
    pub fn refreshes(&self, size: (i32, i32)) -> Vec<i32> {
        let mut r: Vec<i32> =
            self.modes.iter().filter(|m| (m.width, m.height) == size).map(|m| m.refresh).collect::<Vec<_>>();
        r.sort_unstable_by(|a, b| b.cmp(a));
        r.dedup();
        r
    }

    /// Its sizes, biggest first.
    pub fn sizes(&self) -> Vec<(i32, i32)> {
        let mut s: Vec<(i32, i32)> = self.modes.iter().map(|m| (m.width, m.height)).collect();
        if let Some(m) = self.mode {
            s.push((m.width, m.height));
        }
        s.sort_unstable_by_key(|a| std::cmp::Reverse((a.0 * a.1, a.0)));
        s.dedup();
        s
    }

    /// Its mode at `size` with the highest refresh (or the one nearest `refresh`).
    pub fn mode_at(&self, size: (i32, i32), refresh: Option<i32>) -> Option<Mode> {
        let at: Vec<&Mode> = self.modes.iter().filter(|m| (m.width, m.height) == size).collect();
        match refresh {
            Some(r) => at.iter().min_by_key(|m| (m.refresh - r).abs()).map(|m| **m),
            None => at.iter().max_by_key(|m| (m.preferred, m.refresh)).map(|m| **m),
        }
    }
}

/// What to ask labwc for one display.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeadConf {
    pub name: String,
    pub enabled: bool,
    /// One of its modes (or a custom one when it's none of them).
    pub mode: Option<Mode>,
    pub x: i32,
    pub y: i32,
    pub transform: Transform,
    pub scale: f64,
}

/// "2736 × 1824 (3∶2)".
pub fn size_label(w: i32, h: i32) -> String {
    match aspect(w, h) {
        Some((a, b)) => format!("{w} × {h} ({a}∶{b})"),
        None => format!("{w} × {h}"),
    }
}

/// The common aspect ratio `w`×`h` is (within 2%).
pub fn aspect(w: i32, h: i32) -> Option<(i32, i32)> {
    if w <= 0 || h <= 0 {
        return None;
    }
    let (w, h) = if w >= h { (w, h) } else { (h, w) };
    let r = w as f64 / h as f64;
    [(16, 9), (16, 10), (3, 2), (4, 3), (5, 4), (21, 9), (32, 9)]
        .into_iter()
        .find(|(a, b)| (r - *a as f64 / *b as f64).abs() / r < 0.02)
}

/// "60.00 Hz".
pub fn refresh_label(mhz: i32) -> String {
    if mhz <= 0 { "Default".into() } else { format!("{:.2} Hz", mhz as f64 / 1000.0) }
}

// ------------------------------------------------------------ kept layouts

/// A display in a kept layout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Kept {
    pub key: String,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<Mode>,
    pub x: i32,
    pub y: i32,
    #[serde(default)]
    pub transform: Transform,
    pub scale: f64,
}

/// The layout for one set of displays.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    /// The primary's key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary: Option<String>,
    #[serde(default)]
    pub output: Vec<Kept>,
}

impl Layout {
    /// The set of displays it's for (sorted keys).
    pub fn set(&self) -> Vec<String> {
        let mut k: Vec<String> = self.output.iter().map(|o| o.key.clone()).collect();
        k.sort();
        k
    }

    /// What it asks of `heads` (the set it's for), keeping what it doesn't say.
    pub fn confs(&self, heads: &[Head]) -> Vec<HeadConf> {
        heads
            .iter()
            .map(|h| match self.output.iter().find(|o| o.key == h.key()) {
                Some(o) => HeadConf {
                    name: h.name.clone(),
                    enabled: o.enabled,
                    // Its kept mode, if it still has it.
                    mode: o.mode.filter(|m| h.modes.is_empty() || h.modes.iter().any(|x| x.same(m))).or(h.mode),
                    x: o.x,
                    y: o.y,
                    transform: o.transform,
                    scale: if o.scale > 0.0 { o.scale } else { 1.0 },
                },
                None => h.conf(),
            })
            .collect()
    }

    /// The layout `heads` are in now (with `primary`).
    pub fn of(heads: &[Head], primary: Option<String>) -> Self {
        Self {
            primary,
            output: heads
                .iter()
                .map(|h| Kept {
                    key: h.key(),
                    enabled: h.enabled,
                    mode: h.mode,
                    x: h.x,
                    y: h.y,
                    transform: h.transform,
                    scale: if h.scale > 0.0 { h.scale } else { 1.0 },
                })
                .collect(),
        }
    }
}

/// The set of displays `heads` are (sorted keys).
pub fn set_of(heads: &[Head]) -> Vec<String> {
    let mut k: Vec<String> = heads.iter().map(Head::key).collect();
    k.sort();
    k
}

/// True when `heads` are already as `confs` asks (nothing to apply).
pub fn already(heads: &[Head], confs: &[HeadConf]) -> bool {
    confs.iter().all(|c| {
        heads.iter().find(|h| h.name == c.name).is_some_and(|h| {
            if !c.enabled {
                return !h.enabled;
            }
            h.enabled
                && (h.x, h.y) == (c.x, c.y)
                && h.transform == c.transform
                && (h.scale - c.scale).abs() < 0.001
                && match (h.mode, c.mode) {
                    (Some(a), Some(b)) => a.same(&b),
                    (_, None) => true,
                    (None, Some(_)) => false,
                }
        })
    })
}

/// Every kept layout (`displays.toml`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Layouts {
    #[serde(default)]
    pub layout: Vec<Layout>,
}

pub fn path() -> PathBuf {
    crate::settings::dir().join("displays.toml")
}

impl Layouts {
    pub fn load() -> Self {
        Self::load_from(&path())
    }

    pub fn load_from(p: &Path) -> Self {
        std::fs::read_to_string(p)
            .ok()
            .and_then(|s| {
                toml::from_str(&s).map_err(|e| tracing::warn!("{}: {e} (no layouts kept, then)", p.display())).ok()
            })
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        self.save_to(&path())
    }

    pub fn save_to(&self, p: &Path) -> Result<(), String> {
        let text = toml::to_string(self).map_err(|e| e.to_string())?;
        crate::settings::write_atomic(p, &text)
    }

    /// The layout kept for the set `heads` are.
    pub fn for_heads(&self, heads: &[Head]) -> Option<&Layout> {
        let set = set_of(heads);
        self.layout.iter().find(|l| l.set() == set)
    }

    /// Keeps `l` for its set (in place of the one kept before).
    pub fn keep(&mut self, l: Layout) {
        let set = l.set();
        self.layout.retain(|x| x.set() != set);
        self.layout.push(l);
    }

    /// The primary kept for `heads`' set: its connector, if it's one of them and on.
    pub fn primary_for(&self, heads: &[Head]) -> Option<String> {
        let key = self.for_heads(heads)?.primary.as_ref()?;
        heads.iter().find(|h| h.key() == *key && h.enabled).map(|h| h.name.clone())
    }

    /// Sets the primary of `heads`' set (keeping its layout, or theirs now).
    pub fn set_primary(&mut self, heads: &[Head], connector: &str) {
        let key = heads.iter().find(|h| h.name == connector).map(Head::key);
        let mut l = self.for_heads(heads).cloned().unwrap_or_else(|| Layout::of(heads, None));
        l.primary = key;
        self.keep(l);
    }
}

/// The primary among `connectors` (the displays on): the kept one, else
/// `WADOS_PRIMARY`'s, else `fallback`'s, else the first. Its index.
pub fn pick_primary(connectors: &[String], kept: Option<&str>, env: Option<&str>) -> usize {
    pick_primary_or(connectors, kept, env, None)
}

pub fn pick_primary_or(connectors: &[String], kept: Option<&str>, env: Option<&str>, fallback: Option<&str>) -> usize {
    let at = |c: Option<&str>| c.and_then(|c| connectors.iter().position(|x| x == c));
    at(kept).or_else(|| at(env)).or_else(|| at(fallback)).unwrap_or(0)
}

/// The primary when none's been chosen: the laptop's own screen, else the
/// one at the left (then the top) of the layout. The shell and Settings
/// both go by it, whatever order each hears the displays in.
pub fn default_primary(heads: &[Head]) -> Option<String> {
    let on = heads.iter().filter(|h| h.enabled);
    on.clone().find(|h| h.built_in()).or_else(|| on.min_by_key(|h| (h.x, h.y, h.name.clone()))).map(|h| h.name.clone())
}

/// The primary's connector for `heads` (on), as the shell picks it.
pub fn primary_of(heads: &[Head], kept: &Layouts, env: Option<&str>) -> Option<String> {
    let on: Vec<String> = heads.iter().filter(|h| h.enabled).map(|h| h.name.clone()).collect();
    if on.is_empty() {
        return None;
    }
    let kept = kept.primary_for(heads);
    let fallback = default_primary(heads);
    Some(on[pick_primary_or(&on, kept.as_deref(), env, fallback.as_deref())].clone())
}

// ------------------------------------------------------------ the arrangement (arrange.ts)

/// A display in the arrangement, in points.
#[derive(Debug, Clone, PartialEq)]
pub struct Mon {
    pub id: String,
    pub w: f64,
    pub h: f64,
    pub x: f64,
    pub y: f64,
    pub primary: bool,
}

/// How much of an edge two displays must share, at least.
const OVERLAP: f64 = 0.12;

fn overlaps(a: &Mon, b: &Mon) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

fn align(v: f64, size: f64, other: f64, other_size: f64) -> f64 {
    let lines = [other, other + other_size - size, other + (other_size - size) / 2.0];
    lines.into_iter().find(|l| (l - v).abs() < other_size * 0.08).unwrap_or(v)
}

/// The displays with `id` dropped at (x, y), snapped to touch the nearest
/// other along an edge (lined up with its top, middle or bottom when it's
/// close), then all moved so the primary's at 0,0; as they were if it can't fit.
pub fn drop_monitor(mons: &[Mon], id: &str, x: f64, y: f64) -> Vec<Mon> {
    let Some(me) = mons.iter().find(|m| m.id == id) else { return mons.to_vec() };
    let others: Vec<&Mon> = mons.iter().filter(|m| m.id != id).collect();
    if others.is_empty() {
        return mons.to_vec();
    }
    let mut best: Option<(f64, f64, f64)> = None;
    for o in &others {
        let clamp_y = |v: f64| (o.y + o.h - me.h * OVERLAP).min((o.y - me.h + me.h * OVERLAP).max(v));
        let clamp_x = |v: f64| (o.x + o.w - me.w * OVERLAP).min((o.x - me.w + me.w * OVERLAP).max(v));
        let side_y = align(clamp_y(y), me.h, o.y, o.h);
        let side_x = align(clamp_x(x), me.w, o.x, o.w);
        for (cx, cy) in [(o.x - me.w, side_y), (o.x + o.w, side_y), (side_x, o.y - me.h), (side_x, o.y + o.h)] {
            let moved = Mon { x: cx, y: cy, ..me.clone() };
            if others.iter().any(|m| overlaps(&moved, m)) {
                continue;
            }
            let d = (cx - x).hypot(cy - y);
            if best.is_none_or(|b| d < b.2) {
                best = Some((cx, cy, d));
            }
        }
    }
    let Some((bx, by, _)) = best else { return mons.to_vec() };
    let placed: Vec<Mon> = mons
        .iter()
        .map(|m| if m.id == id { Mon { x: bx.round(), y: by.round(), ..m.clone() } } else { m.clone() })
        .collect();
    let p = placed.iter().find(|m| m.primary).unwrap_or(&placed[0]);
    let (px, py) = (p.x, p.y);
    placed.into_iter().map(|m| Mon { x: m.x - px, y: m.y - py, ..m }).collect()
}

/// "Primary", or where `id` sits by the primary ("Right of 1"; `n` is the primary's number).
pub fn where_is(mons: &[Mon], id: &str) -> String {
    let Some(me) = mons.iter().find(|m| m.id == id) else { return String::new() };
    let Some((i, p)) = mons.iter().enumerate().find(|(_, m)| m.primary).or_else(|| mons.first().map(|m| (0, m))) else {
        return String::new();
    };
    if me.id == p.id {
        return "Primary".into();
    }
    let n = i + 1;
    if me.x >= p.x + p.w {
        format!("Right of {n}")
    } else if me.x + me.w <= p.x {
        format!("Left of {n}")
    } else if me.y + me.h <= p.y {
        format!("Above {n}")
    } else {
        format!("Below {n}")
    }
}

/// Moves every display so they touch, nothing overlaps and the leftmost,
/// topmost is at 0,0 (what labwc's layout wants: no gaps it can't fill).
pub fn normalize(mons: &[Mon]) -> Vec<Mon> {
    let min_x = mons.iter().map(|m| m.x).fold(f64::INFINITY, f64::min);
    let min_y = mons.iter().map(|m| m.y).fold(f64::INFINITY, f64::min);
    if !min_x.is_finite() {
        return mons.to_vec();
    }
    mons.iter().map(|m| Mon { x: m.x - min_x, y: m.y - min_y, ..m.clone() }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mon(id: &str, w: f64, h: f64, x: f64, y: f64, primary: bool) -> Mon {
        Mon { id: id.into(), w, h, x, y, primary }
    }

    fn built() -> Mon {
        mon("edp", 2736.0, 1824.0, 0.0, 0.0, true)
    }

    fn dell() -> Mon {
        mon("dp", 3840.0, 2160.0, 2736.0, -200.0, false)
    }

    fn at<'a>(ms: &'a [Mon], id: &str) -> &'a Mon {
        ms.iter().find(|m| m.id == id).unwrap()
    }

    // arrange.test.ts, ported.
    #[test]
    fn snaps_a_monitor_dropped_near_the_left_edge_to_touch_it() {
        let out = drop_monitor(&[built(), dell()], "dp", -3700.0, 300.0);
        let d = at(&out, "dp");
        assert_eq!(d.x, -3840.0);
        assert!(d.y > -2160.0);
        assert!(d.y < 1824.0);
    }

    #[test]
    fn lines_tops_up_when_they_are_close() {
        let out = drop_monitor(&[built(), dell()], "dp", 2800.0, 60.0);
        let d = at(&out, "dp");
        assert_eq!((d.x, d.y), (2736.0, 0.0));
    }

    #[test]
    fn keeps_the_primary_at_0_0_when_the_primary_moves() {
        let out = drop_monitor(&[built(), dell()], "edp", 3000.0, 2500.0);
        let p = out.iter().find(|m| m.primary).unwrap();
        assert_eq!((p.x, p.y), (0.0, 0.0));
        let d = at(&out, "dp");
        assert_eq!(d.y + d.h, 0.0);
    }

    #[test]
    fn leaves_a_lone_monitor_alone() {
        assert_eq!(drop_monitor(&[built()], "edp", 500.0, 500.0), vec![built()]);
    }

    #[test]
    fn says_where_each_is() {
        let ms = [built(), dell()];
        assert_eq!(where_is(&ms, "edp"), "Primary");
        assert_eq!(where_is(&ms, "dp"), "Right of 1");
        let left = drop_monitor(&ms, "dp", -3700.0, 300.0);
        assert_eq!(where_is(&left, "dp"), "Left of 1");
        let below = drop_monitor(&ms, "dp", 0.0, 1900.0);
        assert_eq!(where_is(&below, "dp"), "Below 1");
        let n = normalize(&left);
        assert_eq!((at(&n, "dp").x, at(&n, "edp").x), (0.0, 3840.0));
    }

    fn head(name: &str, serial: &str, w: i32, h: i32, scale: f64) -> Head {
        let m = Mode { width: w, height: h, refresh: 60000, preferred: true };
        Head {
            name: name.into(),
            make: "Dell Inc.".into(),
            model: "U2720Q".into(),
            serial: serial.into(),
            enabled: true,
            modes: vec![m, Mode { width: 1920, height: 1080, refresh: 60000, preferred: false }],
            mode: Some(m),
            scale,
            ..Default::default()
        }
    }

    #[test]
    fn a_head_in_points_and_words() {
        let mut h = head("eDP-1", "", 3000, 2000, 2.0);
        assert!(h.built_in());
        assert_eq!(h.label(), "Built-in Display");
        assert_eq!(h.logical(), (1500, 1000));
        h.transform = Transform::R90;
        assert_eq!(h.logical(), (1000, 1500));
        assert_eq!(h.key(), "Dell Inc.|U2720Q||eDP-1", "no serial: its connector too");
        let d = head("DP-1", "ABC", 3840, 2160, 1.5);
        assert_eq!(d.key(), "Dell Inc.|U2720Q|ABC");
        assert_eq!(d.label(), "Dell Inc. U2720Q");
        assert_eq!(d.sizes(), [(3840, 2160), (1920, 1080)]);
        assert_eq!(d.refreshes((3840, 2160)), [60000]);
        let hl = Head { name: "HEADLESS-1".into(), make: "headless".into(), model: "headless".into(), ..d.clone() };
        assert_eq!(hl.label(), "HEADLESS-1");
        assert_eq!(size_label(2736, 1824), "2736 × 1824 (3∶2)");
        assert_eq!(size_label(1280, 800), "1280 × 800 (16∶10)");
        assert_eq!(size_label(1366, 768), "1366 × 768 (16∶9)");
        assert_eq!(size_label(1000, 999), "1000 × 999");
        assert_eq!(refresh_label(59951), "59.95 Hz");
        assert_eq!(refresh_label(120000), "120.00 Hz");
        assert_eq!(Transform::from_wl(3), Transform::R270);
        assert_eq!(Transform::R270.wl(), 3);
    }

    #[test]
    fn a_layout_is_kept_for_its_set_of_displays() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("displays.toml");
        let laptop = head("eDP-1", "", 3000, 2000, 2.0);
        let mut desk = head("DP-1", "ABC", 3840, 2160, 1.5);
        desk.x = 1500;
        let mut all = Layouts::load_from(&p);
        assert!(all.for_heads(std::slice::from_ref(&laptop)).is_none());
        all.keep(Layout::of(&[laptop.clone(), desk.clone()], Some(desk.key())));
        all.keep(Layout::of(std::slice::from_ref(&laptop), None));
        all.save_to(&p).unwrap();
        let back = Layouts::load_from(&p);
        assert_eq!(back, all);
        assert_eq!(back.layout.len(), 2);
        // In either order, the same set.
        let pair = [desk.clone(), laptop.clone()];
        assert_eq!(back.primary_for(&pair), Some("DP-1".to_string()));
        assert_eq!(back.primary_for(std::slice::from_ref(&laptop)), None);
        // Plugged into another connector, it's still known (it has a serial).
        let mut moved = desk.clone();
        moved.name = "DP-3".into();
        moved.x = 0;
        let now = [laptop.clone(), moved.clone()];
        let l = back.for_heads(&now).unwrap();
        let confs = l.confs(&now);
        assert_eq!(confs[1].name, "DP-3");
        assert_eq!(confs[1].x, 1500);
        assert!(!already(&now, &confs));
        let mut there = moved.clone();
        there.x = 1500;
        assert!(already(&[laptop.clone(), there], &confs));
        // A primary set keeps the layout it's in.
        let mut all = back.clone();
        all.set_primary(&now, "eDP-1");
        assert_eq!(all.primary_for(&now), Some("eDP-1".to_string()));
        assert_eq!(all.for_heads(&now).unwrap().output[1].x, 1500);
    }

    #[test]
    fn the_primary_is_the_kept_one_then_the_environment_s_then_the_first() {
        let c = vec!["HEADLESS-1".to_string(), "HEADLESS-2".to_string()];
        assert_eq!(pick_primary(&c, None, None), 0);
        assert_eq!(pick_primary(&c, None, Some("HEADLESS-2")), 1);
        assert_eq!(pick_primary(&c, Some("HEADLESS-2"), Some("HEADLESS-1")), 1);
        assert_eq!(pick_primary(&c, Some("DP-9"), Some("nope")), 0);
        assert_eq!(pick_primary_or(&c, None, None, Some("HEADLESS-2")), 1);
        // None chosen: the laptop's, else the leftmost.
        let mut a = head("HEADLESS-1", "", 1280, 720, 1.0);
        a.x = 1280;
        let b = head("HEADLESS-2", "", 1280, 720, 1.0);
        assert_eq!(default_primary(&[a.clone(), b.clone()]), Some("HEADLESS-2".into()));
        let e = head("eDP-1", "", 3000, 2000, 2.0);
        let mut e2 = e.clone();
        e2.x = 5000;
        assert_eq!(default_primary(&[b.clone(), e2]), Some("eDP-1".into()));
        assert_eq!(primary_of(&[a.clone(), b.clone()], &Layouts::default(), None), Some("HEADLESS-2".into()));
        assert_eq!(
            primary_of(&[a.clone(), b.clone()], &Layouts::default(), Some("HEADLESS-1")),
            Some("HEADLESS-1".into())
        );
        let mut off = b.clone();
        off.enabled = false;
        assert_eq!(primary_of(&[a, off], &Layouts::default(), None), Some("HEADLESS-1".into()));
    }

    #[test]
    fn a_kept_mode_it_no_longer_has_is_left_as_it_is() {
        let h = head("DP-1", "ABC", 3840, 2160, 1.0);
        let mut l = Layout::of(std::slice::from_ref(&h), None);
        l.output[0].mode = Some(Mode { width: 5120, height: 2880, refresh: 60000, preferred: false });
        assert_eq!(l.confs(std::slice::from_ref(&h))[0].mode, h.mode);
        assert_eq!(h.mode_at((1920, 1080), None).unwrap().width, 1920);
        assert_eq!(h.mode_at((1920, 1080), Some(59000)).unwrap().refresh, 60000);
        assert!(h.mode_at((800, 600), None).is_none());
    }
}
