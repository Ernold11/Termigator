//! The terminal's background opacity and blur (Preferences → Terminal), as Konsole has them: only
//! the default background is see-through, text and coloured cells stay solid.
//!
//! `launch.background_opacity` (percent, [`MIN_OPACITY`]–100) and `launch.background_blur`; a key
//! that is missing (or not a number / bool) takes the value of the global colour scheme
//! (`launch.color_scheme`, else the Konsole default profile's), read from its `[General]`
//! `Opacity=` (0–1) and `Blur=`; without those, 100 % and no blur.
//!
//! Tests: `tests/terminal_background.rs`.

use crate::config::doc::Value;
use crate::config::model::{strip_space, truthy, value_str};
use crate::konsole::{XdgDirs, default_appearance, parse_ini, scheme_dirs};

/// The `launch` key of the opacity in percent.
pub const OPACITY_KEY: &str = "background_opacity";
/// The `launch` key of the blur flag.
pub const BLUR_KEY: &str = "background_blur";
/// The lowest opacity offered: below it the text loses its background.
pub const MIN_OPACITY: i64 = 30;

/// What the terminal draws its background with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Background {
    /// Percent, [`MIN_OPACITY`]..=100.
    pub opacity: i64,
    /// Blur what shows through (KWin); only matters below 100 %.
    pub blur: bool,
}

/// `Opacity` (as a percent) and `Blur` of the Konsole scheme `name`, `None` where the scheme does
/// not set them or cannot be read.
pub fn scheme_effects(name: &str, xdg: &XdgDirs) -> (Option<i64>, Option<bool>) {
    let name = name.strip_suffix(".colorscheme").unwrap_or(name);
    let Some(ini) = scheme_dirs(xdg).into_iter().find_map(|dir| {
        let text = std::fs::read_to_string(dir.join(format!("{name}.colorscheme"))).ok()?;
        parse_ini(&text)
    }) else {
        return (None, None);
    };
    let opacity = ini
        .get("General", "Opacity")
        .and_then(|v| strip_space(v).parse::<f64>().ok())
        .filter(|v| v.is_finite())
        .map(|v| (v * 100.0).round() as i64);
    let blur = ini
        .get("General", "Blur")
        .map(|v| strip_space(v).eq_ignore_ascii_case("true"));
    (opacity, blur)
}

/// The background in effect for the `launch` section.
pub fn configured(launch: &Value, xdg: &XdgDirs) -> Background {
    let scheme = launch
        .get("color_scheme")
        .filter(|v| truthy(v))
        .map(|v| strip_space(&value_str(v)).to_owned())
        .filter(|s| !s.is_empty())
        .or_else(|| default_appearance(xdg).color_scheme);
    let (scheme_opacity, scheme_blur) = scheme
        .map(|s| scheme_effects(&s, xdg))
        .unwrap_or((None, None));
    let opacity = match launch.get(OPACITY_KEY) {
        Some(Value::Int(n)) => Some(*n),
        Some(Value::Float(f)) if f.is_finite() => Some(f.round() as i64),
        _ => None,
    }
    .or(scheme_opacity)
    .unwrap_or(100);
    let blur = match launch.get(BLUR_KEY) {
        Some(Value::Bool(b)) => Some(*b),
        _ => None,
    }
    .or(scheme_blur)
    .unwrap_or(false);
    Background {
        opacity: opacity.clamp(MIN_OPACITY, 100),
        blur,
    }
}
