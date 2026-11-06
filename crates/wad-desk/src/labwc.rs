//! The mouse, the touchpad and the keyboard, through labwc's own config:
//! the person's `~/.config/labwc/rc.xml` (its `<libinput>` and `<keyboard>`
//! repeat, nothing else) and `environment` (the layout's `XKB_DEFAULT_*`),
//! then `labwc --reconfigure`, which reads them again.
//!
//! The session starts labwc with `-m` (`--merge-config`), so the person's
//! rc.xml is read over the machine's (/etc/xdg/labwc), whose keybinds (every
//! one runs wadsctl) stay: without it, theirs would be the only one read.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::settings::{Hand, Keyboard, Mouse, write_atomic};

/// The first line of an rc.xml Settings wrote (one without it is the person's own, and kept aside).
pub const MARK: &str = "<!-- WadOS Settings writes this file (Mouse & Touchpad, Keyboard). labwc reads it over /etc/xdg/labwc/rc.xml (labwc -m). -->";

/// The layouts Keyboard offers: what they're called, and XKB's layout(variant).
pub const LAYOUTS: &[(&str, &str)] = &[
    ("English (US)", "us"),
    ("English (UK)", "gb"),
    ("English (Australian)", "au"),
    ("English (Dvorak)", "us(dvorak)"),
    ("English (Colemak)", "us(colemak)"),
    ("German", "de"),
    ("French (AZERTY)", "fr"),
    ("Spanish", "es"),
    ("Japanese", "jp"),
    ("Māori", "mao"),
];

/// A layout's name ("us(dvorak)" is English (Dvorak)); one not offered is its XKB name.
pub fn layout_label(xkb: &str) -> String {
    LAYOUTS.iter().find(|(_, x)| *x == xkb).map(|(l, _)| l.to_string()).unwrap_or_else(|| xkb.to_string())
}

/// "us(dvorak)": ("us", "dvorak"); "de": ("de", "").
pub fn split_layout(xkb: &str) -> (&str, &str) {
    match xkb.split_once('(') {
        Some((l, v)) => (l.trim(), v.trim_end_matches(')').trim()),
        None => (xkb.trim(), ""),
    }
}

/// `~/.config/labwc`.
pub fn dir() -> PathBuf {
    crate::settings::dir().parent().map(|p| p.join("labwc")).unwrap_or_else(|| PathBuf::from(".config/labwc"))
}

fn yes(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

/// libinput's pointer speed (-1 to 1) for the slider's 0 to 1.
pub fn pointer_speed(v: f64) -> f64 {
    ((v.clamp(0.0, 1.0) * 2.0 - 1.0) * 100.0).round() / 100.0
}

/// The rc.xml for these settings: the keyboard's repeat, and libinput's
/// mice (`non-touch`) and touchpads.
pub fn rc_xml(m: &Mouse, k: &Keyboard) -> String {
    let left = m.primary == Hand::Right;
    let dev = |category: &str, speed: f64, tap: Option<bool>| {
        let mut s = format!(
            "    <device category=\"{category}\">\n      <naturalScroll>{}</naturalScroll>\n      <leftHanded>{}</leftHanded>\n      <pointerSpeed>{:.2}</pointerSpeed>\n",
            yes(m.natural),
            yes(left),
            pointer_speed(speed)
        );
        if let Some(t) = tap {
            s.push_str(&format!("      <tap>{}</tap>\n", yes(t)));
        }
        s.push_str("    </device>\n");
        s
    };
    format!(
        "<?xml version=\"1.0\"?>\n{MARK}\n<labwc_config>\n  <keyboard>\n    <repeatRate>{}</repeatRate>\n    <repeatDelay>{}</repeatDelay>\n  </keyboard>\n  <libinput>\n{}{}  </libinput>\n</labwc_config>\n",
        k.repeat_rate.clamp(1, 100),
        k.repeat_delay.clamp(100, 2000),
        dev("non-touch", m.speed, None),
        dev("touchpad", m.touchpad_speed, Some(m.tap_to_click)),
    )
}

/// The `environment` file with the layout set in it (its other lines kept).
pub fn environment(old: &str, k: &Keyboard) -> String {
    let (layout, variant) = split_layout(&k.layout);
    let mut out: Vec<String> = old
        .lines()
        .filter(|l| {
            let key = l.split('=').next().unwrap_or("").trim();
            key != "XKB_DEFAULT_LAYOUT" && key != "XKB_DEFAULT_VARIANT"
        })
        .map(str::to_string)
        .collect();
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    out.push(format!("XKB_DEFAULT_LAYOUT={layout}"));
    out.push(format!("XKB_DEFAULT_VARIANT={variant}"));
    out.join("\n") + "\n"
}

/// Writes rc.xml and environment into `dir` (an rc.xml of the person's own
/// moved aside to `rc.xml.before-wados` first).
pub fn write_to(dir: &Path, m: &Mouse, k: &Keyboard) -> Result<(), String> {
    let rc = dir.join("rc.xml");
    if let Ok(old) = std::fs::read_to_string(&rc)
        && !old.contains(MARK)
    {
        let aside = dir.join("rc.xml.before-wados");
        std::fs::rename(&rc, &aside).map_err(|e| format!("{}: {e}", rc.display()))?;
        tracing::info!("{} was the person's own: it's {} now", rc.display(), aside.display());
    }
    write_atomic(&rc, &rc_xml(m, k))?;
    let env = dir.join("environment");
    let old = std::fs::read_to_string(&env).unwrap_or_default();
    write_atomic(&env, &environment(&old, k))
}

/// Writes them (`~/.config/labwc`) and has labwc read them again.
pub fn apply(m: &Mouse, k: &Keyboard) -> Result<(), String> {
    write_to(&dir(), m, k)?;
    reconfigure()
}

/// `labwc --reconfigure` (to this session's labwc: LABWC_PID).
pub fn reconfigure() -> Result<(), String> {
    if std::env::var_os("LABWC_PID").is_none() {
        return Err("labwc isn't this session's compositor (no LABWC_PID): it'll read them when it starts".into());
    }
    let out = Command::new("labwc").arg("--reconfigure").output().map_err(|e| format!("labwc: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("labwc --reconfigure: {}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rc_xml() {
        let m = Mouse { primary: Hand::Right, speed: 0.75, touchpad_speed: 0.25, natural: true, tap_to_click: false };
        let k = Keyboard { layout: "us".into(), repeat_delay: 300, repeat_rate: 40 };
        let x = rc_xml(&m, &k);
        assert!(x.starts_with("<?xml version=\"1.0\"?>\n<!-- WadOS Settings"));
        assert!(x.contains("<repeatRate>40</repeatRate>\n    <repeatDelay>300</repeatDelay>"));
        assert!(x.contains("<device category=\"non-touch\">\n      <naturalScroll>yes</naturalScroll>\n      <leftHanded>yes</leftHanded>\n      <pointerSpeed>0.50</pointerSpeed>\n    </device>"), "{x}");
        assert!(x.contains("<device category=\"touchpad\">\n      <naturalScroll>yes</naturalScroll>\n      <leftHanded>yes</leftHanded>\n      <pointerSpeed>-0.50</pointerSpeed>\n      <tap>no</tap>\n    </device>"), "{x}");
        // Nothing else: the machine's keybinds stay the machine's.
        assert!(!x.contains("keybind") && !x.contains("<core>"));
        let d = rc_xml(&Mouse::default(), &Keyboard::default());
        assert!(d.contains("<leftHanded>no</leftHanded>") && d.contains("<pointerSpeed>0.00</pointerSpeed>"));
        assert!(d.contains("<tap>yes</tap>") && d.contains("<repeatRate>25</repeatRate>"));
        assert_eq!(pointer_speed(0.0), -1.0);
        assert_eq!(pointer_speed(2.0), 1.0);
        assert_eq!(pointer_speed(0.55), 0.1);
    }

    #[test]
    fn the_layout_in_labwc_s_environment() {
        let k = |l: &str| Keyboard { layout: l.into(), ..Keyboard::default() };
        assert_eq!(environment("", &k("de")), "XKB_DEFAULT_LAYOUT=de\nXKB_DEFAULT_VARIANT=\n");
        let old = "# mine\nFOO=bar\nXKB_DEFAULT_LAYOUT=de\nXKB_DEFAULT_VARIANT=\n\n";
        assert_eq!(
            environment(old, &k("us(dvorak)")),
            "# mine\nFOO=bar\nXKB_DEFAULT_LAYOUT=us\nXKB_DEFAULT_VARIANT=dvorak\n"
        );
        assert_eq!(split_layout("us(colemak)"), ("us", "colemak"));
        assert_eq!(split_layout("mao"), ("mao", ""));
        assert_eq!(layout_label("us(dvorak)"), "English (Dvorak)");
        assert_eq!(layout_label("ru"), "ru");
        assert_eq!(LAYOUTS.len(), 10);
    }

    #[test]
    fn a_person_s_own_rc_xml_is_moved_aside() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("rc.xml"), "<labwc_config><theme/></labwc_config>").unwrap();
        write_to(d.path(), &Mouse::default(), &Keyboard::default()).unwrap();
        assert!(std::fs::read_to_string(d.path().join("rc.xml")).unwrap().contains(MARK));
        assert_eq!(
            std::fs::read_to_string(d.path().join("rc.xml.before-wados")).unwrap(),
            "<labwc_config><theme/></labwc_config>"
        );
        // Ours again: written over, nothing more moved aside.
        let k = Keyboard { repeat_rate: 33, ..Keyboard::default() };
        write_to(d.path(), &Mouse::default(), &k).unwrap();
        assert!(std::fs::read_to_string(d.path().join("rc.xml")).unwrap().contains("<repeatRate>33</repeatRate>"));
        assert_eq!(
            std::fs::read_to_string(d.path().join("environment")).unwrap(),
            "XKB_DEFAULT_LAYOUT=us\nXKB_DEFAULT_VARIANT=\n"
        );
    }
}
