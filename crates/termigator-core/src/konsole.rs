//! Konsole default profile (colour scheme, font), colour scheme choice and monospace font
//! fallback.
//!
//! [`default_appearance`] follows KDE: `konsolerc` -> `[Desktop Entry] DefaultProfile` -> that
//! profile file in a Konsole data directory ([`scheme_dirs`]) -> `[Appearance] ColorScheme` and
//! `Font` (`Family,pointSize,...`). Everything the machine provides comes in as a parameter
//! ([`XdgDirs`], the installed font families, the available schemes), so tests use temporary
//! trees. The INI reader ([`parse_ini`]) reads INI as follows: non-strict, no interpolation,
//! case-sensitive keys, `=` or `:` delimiters, `#`/`;` comment lines, indented continuation lines,
//! `[DEFAULT]` fallback; a file it cannot read (bad UTF-8, text before the first section, a line
//! that is no option) counts as unreadable. Picking the colour scheme and font on a widget, and
//! asking the palette or Qt for the installed fonts, is UI work (U-tasks).
//!
//! Notes (none occur with sane files): a font size with digit separators (`1_0`) is ignored;
//! an infinite size is ignored; `~` is not expanded in `XDG_*` values. Golden fixtures:
//! `compat/golden/tools_konsole.json`.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use crate::config::model::strip_space;
use crate::proto::ssh_keys::normalize_path;

/// Preferred colour schemes for a dark UI, best first.
pub const COLOR_SCHEME_PREFS_DARK: [&str; 8] = [
    "DarkPastels",
    "Breeze",
    "Solarized",
    "Nord",
    "Linux",
    "Tango",
    "Ubuntu",
    "WhiteOnBlack",
];

/// Preferred colour schemes for a light UI, best first.
pub const COLOR_SCHEME_PREFS_LIGHT: [&str; 4] = [
    "BlackOnWhite",
    "SolarizedLight",
    "BlackOnLightYellow",
    "GreenOnBlack",
];

/// Monospace families to try, first installed wins ("Consolas" is rare on Linux and would fall
/// back to a sans font, so a proper fixed-pitch font comes first).
pub const MONO_FONT_CANDIDATES: [&str; 14] = [
    // Nerd Font builds first: a plain mono font lacks the prompt's Powerline/private-use glyphs,
    // and Qt then falls back to another family with different advances (a visibly broken prompt).
    "JetBrainsMono Nerd Font Mono",
    "JetBrainsMono Nerd Font",
    "MesloLGS Nerd Font",
    "JetBrains Mono",
    "Hack",
    "Fira Code",
    "Cascadia Code",
    "Source Code Pro",
    "DejaVu Sans Mono",
    "Noto Sans Mono",
    "Liberation Mono",
    "Ubuntu Mono",
    "Consolas",
    "Monospace",
];

/// The scheme used when the profile names none.
pub const DEFAULT_COLOR_SCHEME: &str = "Breeze";

/// The font size used when neither the settings nor the profile give a usable one.
pub const DEFAULT_FONT_SIZE: i64 = 11;

/// The directories and variables Konsole's files are looked up by.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct XdgDirs {
    /// `$HOME`.
    pub home: PathBuf,
    /// `$XDG_CONFIG_HOME` (empty or unset: `home/.config`).
    pub config_home: Option<String>,
    /// `$XDG_DATA_HOME`.
    pub data_home: Option<String>,
    /// `$XDG_DATA_DIRS` (empty or unset: `/usr/local/share:/usr/share`).
    pub data_dirs: Option<String>,
}

impl XdgDirs {
    /// The real environment.
    pub fn from_env() -> XdgDirs {
        let var = |k: &str| std::env::var(k).ok();
        XdgDirs {
            home: var("HOME")
                .map(PathBuf::from)
                .or_else(std::env::home_dir)
                .unwrap_or_default(),
            config_home: var("XDG_CONFIG_HOME"),
            data_home: var("XDG_DATA_HOME"),
            data_dirs: var("XDG_DATA_DIRS"),
        }
    }
}

/// What the default profile says; any field may be absent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KonsoleAppearance {
    pub color_scheme: Option<String>,
    pub font_family: Option<String>,
    pub font_size: Option<i64>,
}

/// The existing directories where Konsole keeps `*.colorscheme` and profile files, in lookup
/// order: `$XDG_DATA_HOME/konsole`, `~/.local/share/konsole`, `$XDG_DATA_DIRS/*/konsole`.
pub fn scheme_dirs(xdg: &XdgDirs) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(d) = xdg.data_home.as_deref().filter(|d| !d.is_empty()) {
        dirs.push(Path::new(d).join("konsole"));
    }
    dirs.push(xdg.home.join(".local/share/konsole"));
    let bases = xdg
        .data_dirs
        .as_deref()
        .filter(|d| !d.is_empty())
        .unwrap_or("/usr/local/share:/usr/share");
    for base in bases.split(':') {
        let base = strip_space(base);
        if !base.is_empty() {
            dirs.push(Path::new(base).join("konsole"));
        }
    }
    let mut seen = HashSet::new();
    dirs.into_iter()
        .map(|d| normalize_path(&d))
        .filter(|d| seen.insert(d.clone()) && d.is_dir())
        .collect()
}

/// A parsed INI file: `[DEFAULT]` values and the named sections.
#[derive(Debug, Default)]
pub struct Ini {
    defaults: BTreeMap<String, String>,
    sections: BTreeMap<String, BTreeMap<String, String>>,
}

impl Ini {
    /// `configparser.get` after `has_option`: the section's value, else the `[DEFAULT]` one.
    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        let s = self.sections.get(section)?;
        s.get(key)
            .or_else(|| self.defaults.get(key))
            .map(String::as_str)
    }
}

fn indent_of(line: &str) -> usize {
    line.chars().take_while(|c| c.is_whitespace()).count()
}

/// Lines of a text file (`\n`, `\r\n` and `\r` each end a line).
fn file_lines(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                out.push(&text[start..i]);
                start = i + 1;
            }
            b'\r' => {
                out.push(&text[start..i]);
                if bytes.get(i + 1) == Some(&b'\n') {
                    i += 1;
                }
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

/// Which map the option being read belongs to.
#[derive(Clone, PartialEq)]
enum Sect {
    Defaults,
    Named(String),
}

/// Parses `text` like `ConfigParser(strict=False, interpolation=None)` with case-sensitive keys.
/// `None` where the file is invalid (an option or text before any section, a line that is neither
/// section nor option, an empty option name).
pub fn parse_ini(text: &str) -> Option<Ini> {
    let mut ini = Ini::default();
    let mut cursect: Option<Sect> = None;
    let mut opt: Option<(String, Vec<String>)> = None; // option being read and its lines
    let mut indent_level = 0usize;
    let mut failed = false;

    // Values are kept as line lists while an option may still continue, then joined.
    fn flush(ini: &mut Ini, sect: &Option<Sect>, opt: &mut Option<(String, Vec<String>)>) {
        if let (Some(s), Some((k, lines))) = (sect, opt.take()) {
            let val = lines.join("\n").trim_end().to_owned();
            match s {
                Sect::Defaults => ini.defaults.insert(k, val),
                Sect::Named(n) => ini.sections.entry(n.clone()).or_default().insert(k, val),
            };
        }
    }

    for line in file_lines(text) {
        let value = strip_space(line);
        let comment = value.starts_with('#') || value.starts_with(';');
        if value.is_empty() || comment {
            if !comment && let Some((_, lines)) = opt.as_mut() {
                lines.push(String::new()); // empty_lines_in_values
            }
            continue;
        }
        let cur_indent = indent_of(line);
        if cursect.is_some() && opt.is_some() && cur_indent > indent_level {
            if let Some((_, lines)) = opt.as_mut() {
                lines.push(value.to_owned());
            }
            continue;
        }
        indent_level = cur_indent;
        // SECTCRE = \[(?P<header>.+)\] matched at the start of the stripped line
        if let Some(rest) = value.strip_prefix('[')
            && let Some(end) = rest.rfind(']')
            && end > 0
        {
            flush(&mut ini, &cursect, &mut opt);
            let name = &rest[..end];
            cursect = Some(if name == "DEFAULT" {
                Sect::Defaults
            } else {
                ini.sections.entry(name.to_owned()).or_default();
                Sect::Named(name.to_owned())
            });
            continue;
        }
        cursect.as_ref()?; // MissingSectionHeaderError
        match value.find(['=', ':']) {
            Some(d) if !strip_space(&value[..d]).is_empty() => {
                flush(&mut ini, &cursect, &mut opt);
                opt = Some((
                    value[..d].trim_end().to_owned(),
                    vec![strip_space(&value[d + 1..]).to_owned()],
                ));
            }
            _ => failed = true, // ParsingError, raised after the whole file is read
        }
    }
    flush(&mut ini, &cursect, &mut opt);
    (!failed).then_some(ini)
}

/// `int(float(text))` for a font point size; `None` where it does not parse or overflows.
fn point_size(text: &str) -> Option<i64> {
    let f: f64 = strip_space(text).parse().ok()?;
    (f.is_finite() && f.abs() < 9.2e18).then(|| f.trunc() as i64)
}

fn read_ini(path: &Path) -> Option<Ini> {
    parse_ini(&String::from_utf8(std::fs::read(path).ok()?).ok()?)
}

/// KDE Konsole's default profile: colour scheme and font. Without a readable custom default
/// profile the scheme is [`DEFAULT_COLOR_SCHEME`] and the font is unset.
pub fn default_appearance(xdg: &XdgDirs) -> KonsoleAppearance {
    let breeze = KonsoleAppearance {
        color_scheme: Some(DEFAULT_COLOR_SCHEME.to_owned()),
        ..KonsoleAppearance::default()
    };
    let cfg_home = match xdg.config_home.as_deref().filter(|d| !d.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => xdg.home.join(".config"),
    };
    let konsolerc = cfg_home.join("konsolerc");
    let profile_name = if konsolerc.is_file() {
        read_ini(&konsolerc)
            .and_then(|i| i.get("Desktop Entry", "DefaultProfile").map(str::to_owned))
            .map(|n| strip_space(&n).to_owned())
            .filter(|n| !n.is_empty())
    } else {
        None
    };
    let Some(profile_name) = profile_name else {
        return breeze;
    };
    let Some(path) = scheme_dirs(xdg)
        .into_iter()
        .map(|d| d.join(&profile_name))
        .find(|p| p.is_file())
    else {
        return breeze;
    };
    let Some(ini) = read_ini(&path) else {
        return breeze; // unreadable: fall back to Breeze
    };
    // QFont string form: "Family,pointSize,..."
    let mut font = ini.get("Appearance", "Font").map(|f| f.split(','));
    let font_family = font
        .as_mut()
        .and_then(Iterator::next)
        .map(|f| strip_space(f).to_owned())
        .filter(|f| !f.is_empty());
    let font_size = font.as_mut().and_then(Iterator::next).and_then(point_size);
    // A profile without an explicit ColorScheme uses the built-in Breeze scheme.
    let color_scheme = ini
        .get("Appearance", "ColorScheme")
        .map(|s| strip_space(s).to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_COLOR_SCHEME.to_owned());
    KonsoleAppearance {
        color_scheme: Some(color_scheme),
        font_family,
        font_size,
    }
}

/// The family to use: `family` if installed, else the first installed of
/// [`MONO_FONT_CANDIDATES`]. With no font list at all (`installed` empty) the first candidate
/// is taken on trust.
pub fn pick_terminal_font(family: Option<&str>, installed: &HashSet<String>) -> String {
    let fam = strip_space(family.unwrap_or_default());
    let wanted = (!fam.is_empty()).then_some(fam);
    wanted
        .into_iter()
        .chain(MONO_FONT_CANDIDATES)
        .find(|c| installed.is_empty() || installed.contains(*c))
        .or(wanted)
        .unwrap_or("Monospace")
        .to_owned()
}

/// The point size: the settings value if set, else the profile's; below 1 it is
/// [`DEFAULT_FONT_SIZE`].
pub fn effective_font_size(configured: Option<i64>, profile: Option<i64>) -> i64 {
    let size = match configured {
        Some(s) if s != 0 => s,
        _ => profile.unwrap_or(0),
    };
    if size < 1 { DEFAULT_FONT_SIZE } else { size }
}

/// The scheme to set among `available` (Qt's list; names may carry `.colorscheme`): the
/// `preferred` one if present, else the first present of the light or dark preferences by the
/// UI palette, then of the dark ones, then simply the first available.
pub fn choose_color_scheme(
    available: &[String],
    preferred: Option<&str>,
    dark: bool,
) -> Option<String> {
    let names: HashSet<&str> = available.iter().map(String::as_str).collect();
    let try_name = |name: &str| -> Option<String> {
        if names.contains(name) {
            return Some(name.to_owned());
        }
        let ext = format!("{name}.colorscheme");
        names.contains(ext.as_str()).then_some(ext)
    };
    let prefs: &[&str] = if dark {
        &COLOR_SCHEME_PREFS_DARK
    } else {
        &COLOR_SCHEME_PREFS_LIGHT
    };
    preferred
        .filter(|p| !p.is_empty())
        .and_then(try_name)
        .or_else(|| prefs.iter().find_map(|p| try_name(p)))
        .or_else(|| COLOR_SCHEME_PREFS_DARK.iter().find_map(|p| try_name(p)))
        .or_else(|| available.first().cloned())
}
