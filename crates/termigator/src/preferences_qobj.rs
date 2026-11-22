//! Settings dialog : `PreferencesController`, exposed to QML as
//! `Termigator.PreferencesController`.
//!
//! The dialog (`qml/preferences/PreferencesDialog.qml`) keeps the edited draft; this object only
//! translates between the shared config and a flat JSON draft whose keys are the config keys of
//! the `window` / `launch` sections. It holds no
//! copy of the config; saving is `controller.persist()` in QML, which emits `configChanged`.

use std::collections::BTreeSet;
use std::path::Path;

use cxx_qt_lib::{QString, QStringList};
use serde_json::{Map, json};
use termigator_core::config::doc::Value;
use termigator_core::config::environments;
use termigator_core::config::model::{
    DEFAULT_SCROLLBACK_LINES, default_config, normalize_terminal_mode, section, strip_space,
    truthy, value_int, value_str,
};
use termigator_core::konsole::{XdgDirs, scheme_dirs};
use termigator_core::proto::rdp;

use crate::shell_qobj::with_app;

/// The style used when nothing else is chosen: Kirigami's own Breeze look.
pub const DEFAULT_QUICK_STYLE: &str = "org.kde.desktop";

/// The Qt Quick Controls style `cfg` asks for: `window.quick_style`, else the default. (The
/// legacy `window.modern_style` key is ignored; there is no fallback for it any more.)
pub fn configured_style(cfg: &Value) -> String {
    let chosen = text_of(section(cfg, "window"), "quick_style");
    let chosen = strip_space(&chosen);
    if !chosen.is_empty() {
        return chosen.to_owned();
    }
    DEFAULT_QUICK_STYLE.to_owned()
}

/// `window.ui_theme`: "auto" (default; the desktop's light/dark preference), "light" or "dark".
pub fn configured_ui_theme(cfg: &Value) -> String {
    let chosen = text_of(section(cfg, "window"), "ui_theme");
    match strip_space(&chosen).to_lowercase().as_str() {
        t @ ("light" | "dark") => t.to_owned(),
        _ => "auto".to_owned(),
    }
}

/// The style to start with: the configured one when it is installed, else the default.
pub fn startup_style(cfg: &Value, available: &[String]) -> String {
    let wanted = configured_style(cfg);
    available
        .iter()
        .find(|s| s.eq_ignore_ascii_case(&wanted))
        .cloned()
        .unwrap_or_else(|| DEFAULT_QUICK_STYLE.to_owned())
}

/// Stored when the log file field is left empty.
const LOG_TEMPLATE: &str = "~/termigator-logs/{label}/{date}_{time}.log";

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
    }

    #[namespace = "termigator::preferences"]
    unsafe extern "C++" {
        include!("termigator/src/preferences_native.h");
        #[rust_name = "monospace_families"]
        fn monospaceFamilies() -> QStringList;
        #[rust_name = "builtin_color_schemes"]
        fn builtinColorSchemes() -> QStringList;
        #[rust_name = "available_quick_styles"]
        fn availableQuickStyles() -> QStringList;
    }

    #[auto_cxx_name]
    extern "RustQt" {
        #[qobject]
        #[qml_element]
        type PreferencesController = super::PreferencesControllerRust;

        /// The page values of the current config as a JSON object.
        #[qinvokable]
        fn load(self: &PreferencesController) -> QString;

        /// The same for `default_config()` (Restore Defaults).
        #[qinvokable]
        fn defaults(self: &PreferencesController) -> QString;

        /// Writes a draft (JSON from `load` / `defaults`, edited) into the config's `window` and
        /// `launch` sections; not saved, call `persist` afterwards.
        #[qinvokable]
        fn apply(self: &PreferencesController, draft: &QString);

        /// Colour scheme names (compiled in + Konsole's directories), sorted ignoring case.
        #[qinvokable]
        fn color_schemes(self: &PreferencesController) -> QStringList;

        /// Fixed-pitch font families, for the Font combo.
        #[qinvokable]
        fn monospace_fonts(self: &PreferencesController) -> QStringList;

        /// Qt Quick Controls styles that are installed (the Style combo).
        #[qinvokable]
        fn quick_styles(self: &PreferencesController) -> QStringList;

        /// The style this process runs with (a changed choice applies after a restart).
        #[qinvokable]
        fn running_style(self: &PreferencesController) -> QString;

        /// The local shells of this machine, `[{id, label, available}]`, for the Windows choice
        /// of the default shell.
        #[qinvokable]
        fn local_shells(self: &PreferencesController) -> QString;
    }
}

#[derive(Default)]
pub struct PreferencesControllerRust;

impl qobject::PreferencesController {
    fn load(&self) -> QString {
        let json = with_app(|app| page_values(&app.cfg));
        QString::from(json.to_string().as_str())
    }

    fn defaults(&self) -> QString {
        QString::from(page_values(&default_config()).to_string().as_str())
    }

    fn apply(&self, draft: &QString) {
        // The draft comes from our own QML; a text that is not a JSON object changes nothing.
        let Some(draft) = crate::qml_json::value("apply", &draft.to_string()) else {
            return;
        };
        let theme = with_app(|app| {
            let before = configured_ui_theme(&app.cfg);
            apply_values(&mut app.cfg, &draft);
            let after = configured_ui_theme(&app.cfg);
            (after != before).then_some(after)
        });
        // Light / Dark take effect at once (palette, Breeze style, icon theme). Automatic inside a
        // Plasma session keeps the look this process already has until a restart.
        if let Some(theme) = theme {
            crate::shell_qobj::qobject::apply_fixed_look(&QString::from(theme.as_str()));
        }
    }

    fn color_schemes(&self) -> QStringList {
        let builtin: Vec<String> = qobject::builtin_color_schemes()
            .iter()
            .map(|s| s.to_string())
            .collect();
        let dirs = scheme_dirs(&XdgDirs::from_env());
        scheme_names(builtin, &dirs)
            .iter()
            .map(|s| QString::from(s.as_str()))
            .collect()
    }

    fn monospace_fonts(&self) -> QStringList {
        qobject::monospace_families()
    }

    fn quick_styles(&self) -> QStringList {
        qobject::available_quick_styles()
    }

    fn running_style(&self) -> QString {
        cxx_qt_lib::QQuickStyle::name()
    }

    fn local_shells(&self) -> QString {
        let path = std::env::var("PATH").ok();
        let which = |n: &str| termigator_core::proto::ssh::which(n, path.as_deref());
        let shells = termigator_core::proto::local_shell::detect(&which);
        QString::from(crate::panes_qobj::local_shells_json(&shells).as_str())
    }
}

// ---- Pure logic (unit-tested below) ------------------------------------------------------

/// The text of `v`, empty when it is missing or falsy.
fn text_of(sec: &Value, name: &str) -> String {
    sec.get(name)
        .filter(|v| truthy(v))
        .map(value_str)
        .unwrap_or_default()
}

/// `launch.rdp_resolution`: "screen" until chosen; "" is a choice too (the client's own default).
fn rdp_resolution_of(sec: &Value) -> String {
    match sec.get("rdp_resolution") {
        None | Some(Value::Null) => rdp::RDP_RESOLUTION_DEFAULT.to_owned(),
        Some(v) => strip_space(&value_str(v)).to_owned(),
    }
}

/// The truthiness of `sec[name]`, `default` when the key is missing.
fn flag_of(sec: &Value, name: &str, default: bool) -> bool {
    sec.get(name).map_or(default, truthy)
}

/// `sec[name]` as an integer (`default` when missing, falsy or not a number), then a spin box's
/// clamp to `lo..=hi`.
fn number_of(sec: &Value, name: &str, default: i64, lo: i64, hi: i64) -> i64 {
    sec.get(name)
        .filter(|v| truthy(v))
        .and_then(value_int)
        .unwrap_or(default)
        .clamp(lo, hi)
}

/// What the dialog's widgets show for `cfg` .
pub fn page_values(cfg: &Value) -> serde_json::Value {
    let (w, l) = (section(cfg, "window"), section(cfg, "launch"));
    let mut v = Map::new();
    let mut put = |key: &str, value: serde_json::Value| {
        v.insert(key.to_owned(), value);
    };
    put(
        "font_family",
        json!(strip_space(&text_of(l, "font_family"))),
    );
    put("font_size", json!(number_of(l, "font_size", 0, 0, 72)));
    put(
        "color_scheme",
        json!(strip_space(&text_of(l, "color_scheme"))),
    );
    put(
        "blinking_cursor",
        json!(flag_of(l, "blinking_cursor", false)),
    );
    put("quick_style", json!(configured_style(cfg)));
    put("ui_theme", json!(configured_ui_theme(cfg)));
    put(
        "embedded_pyte_history",
        json!(number_of(
            l,
            "embedded_pyte_history",
            DEFAULT_SCROLLBACK_LINES,
            1_000,
            2_000_000
        )),
    );
    put(
        "scrollback_unlimited",
        json!(flag_of(l, "scrollback_unlimited", false)),
    );
    put(
        "bracketed_paste",
        json!(flag_of(l, "bracketed_paste", true)),
    );
    put(
        "tab_attention_marks",
        json!(flag_of(l, "tab_attention_marks", true)),
    );
    put("bell_sound", json!(flag_of(l, "bell_sound", true)));
    put(
        "track_remote_folder",
        json!(flag_of(l, "track_remote_folder", true)),
    );
    put(
        "default_terminal",
        json!(normalize_terminal_mode(w.get("default_terminal"))),
    );
    put("shell", json!(text_of(l, "shell")));
    put("template", json!(text_of(l, "template")));
    put(
        "confirm_multiline_paste",
        json!(flag_of(l, "confirm_multiline_paste", true)),
    );
    put("copy_on_select", json!(flag_of(l, "copy_on_select", false)));
    put(
        "paste_on_right_click",
        json!(flag_of(l, "paste_on_right_click", false)),
    );
    put(
        "paste_on_middle_click",
        json!(flag_of(l, "paste_on_middle_click", true)),
    );
    put(
        "session_log_auto",
        json!(flag_of(l, "session_log_auto", false)),
    );
    put(
        "session_log_template",
        json!(text_of(l, "session_log_template")),
    );
    put(
        "session_log_strip_ansi",
        json!(flag_of(l, "session_log_strip_ansi", true)),
    );
    put(
        "session_log_timestamps",
        json!(flag_of(l, "session_log_timestamps", false)),
    );
    put("ssh_keepalive", json!(flag_of(l, "ssh_keepalive", true)));
    put(
        "ssh_keepalive_seconds",
        json!(number_of(l, "ssh_keepalive_seconds", 15, 3, 300)),
    );
    put(
        "probe_timeout",
        json!(number_of(l, "probe_timeout", 5, 1, 60)),
    );
    put("rdp_client", json!(text_of(l, "rdp_client")));
    put("rdp_resolution", json!(rdp_resolution_of(l)));
    put("vnc_client", json!(text_of(l, "vnc_client")));
    put("editor", json!(text_of(l, "editor")));
    put("tray_icon", json!(flag_of(w, "tray_icon", true)));
    put("show_recent", json!(flag_of(w, "show_recent", true)));
    put(
        "host_list_detailed",
        json!(flag_of(w, "host_list_detailed", false)),
    );
    put("confirm_close", json!(flag_of(w, "confirm_close", true)));
    put(
        "confirm_close_tab",
        json!(flag_of(w, "confirm_close_tab", true)),
    );
    put(
        "notify_on_disconnect",
        json!(flag_of(w, "notify_on_disconnect", false)),
    );
    put("check_updates", json!(flag_of(w, "check_updates", true)));
    put("always_on_top", json!(flag_of(w, "always_on_top", false)));
    put(
        "session_restore",
        json!(flag_of(w, "session_restore", false)),
    );
    put(
        "environment_frame",
        json!(flag_of(w, "environment_frame", false)),
    );
    put(
        "environments",
        environments::to_json(&environments::list(cfg)),
    );
    serde_json::Value::Object(v)
}

/// `cfg[section][name] = value`; the section becomes a mapping if it is not one,
/// the key is appended when new.
fn store(cfg: &mut Value, sec: &str, name: &str, value: Value) {
    let key = |t: &str| Value::Str(t.to_owned());
    if !matches!(cfg, Value::Map(_)) {
        *cfg = Value::Map(Vec::new());
    }
    let Value::Map(top) = cfg else { return };
    let at = top
        .iter()
        .position(|(k, _)| *k == key(sec))
        .unwrap_or_else(|| {
            top.push((key(sec), Value::Null));
            top.len() - 1
        });
    if !matches!(top[at].1, Value::Map(_)) {
        top[at].1 = Value::Map(Vec::new());
    }
    let Value::Map(pairs) = &mut top[at].1 else {
        return;
    };
    match pairs.iter_mut().find(|(k, _)| *k == key(name)) {
        Some((_, slot)) => *slot = value,
        None => pairs.push((key(name), value)),
    }
}

/// Writes a draft into `cfg["window"]` / `cfg["launch"]` A missing or
/// mistyped draft field falls back to the value the dialog would show for an empty config.
pub fn apply_values(cfg: &mut Value, draft: &serde_json::Value) {
    let base = page_values(&Value::Map(Vec::new()));
    let field = |name: &str| draft.get(name).or_else(|| base.get(name));
    let flag = |name: &str| field(name).and_then(|v| v.as_bool()).unwrap_or(false);
    let text = |name: &str| {
        field(name)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_owned()
    };
    let stripped = |name: &str| strip_space(&text(name)).to_owned();
    let num = |name: &str, lo: i64, hi: i64| {
        field(name)
            .and_then(|v| v.as_i64())
            .unwrap_or(lo)
            .clamp(lo, hi)
    };
    let optional = |name: &str| match stripped(name) {
        t if t.is_empty() => Value::Null,
        t => Value::Str(t),
    };

    // Only a changed choice is stored, so a config that never chose one stays as it was written.
    let style = stripped("quick_style");
    if !style.is_empty() && style != configured_style(cfg) {
        store(cfg, "window", "quick_style", Value::Str(style));
    }
    let ui_theme = stripped("ui_theme");
    if !ui_theme.is_empty() && ui_theme != configured_ui_theme(cfg) {
        store(cfg, "window", "ui_theme", Value::Str(ui_theme));
    }
    let mut win = |name: &str, value: Value| store(cfg, "window", name, value);
    win(
        "default_terminal",
        Value::Str(normalize_terminal_mode(Some(&Value::Str(text("default_terminal")))).to_owned()),
    );
    for name in [
        "tray_icon",
        "show_recent",
        "host_list_detailed",
        "confirm_close",
        "confirm_close_tab",
        "notify_on_disconnect",
        "check_updates",
        "always_on_top",
        "session_restore",
        "environment_frame",
    ] {
        win(name, Value::Bool(flag(name)));
    }

    let resolution_chosen_before = cfg
        .get("launch")
        .is_some_and(|l| l.get("rdp_resolution").is_some());
    let mut launch = |name: &str, value: Value| store(cfg, "launch", name, value);
    launch("font_family", Value::Str(stripped("font_family")));
    launch("font_size", Value::Int(num("font_size", 0, 72)));
    launch("color_scheme", Value::Str(stripped("color_scheme")));
    launch(
        "embedded_pyte_history",
        Value::Int(num("embedded_pyte_history", 1_000, 2_000_000)),
    );
    for name in [
        "scrollback_unlimited",
        "blinking_cursor",
        "bracketed_paste",
        "confirm_multiline_paste",
        "copy_on_select",
        "paste_on_right_click",
        "paste_on_middle_click",
        "tab_attention_marks",
        "bell_sound",
        "track_remote_folder",
        "session_log_auto",
    ] {
        launch(name, Value::Bool(flag(name)));
    }
    let log_template = stripped("session_log_template");
    launch(
        "session_log_template",
        Value::Str(if log_template.is_empty() {
            LOG_TEMPLATE.to_owned()
        } else {
            log_template
        }),
    );
    for name in [
        "session_log_strip_ansi",
        "session_log_timestamps",
        "ssh_keepalive",
    ] {
        launch(name, Value::Bool(flag(name)));
    }
    launch(
        "ssh_keepalive_seconds",
        Value::Int(num("ssh_keepalive_seconds", 3, 300)),
    );
    let template = stripped("template");
    if !template.is_empty() {
        launch("template", Value::Str(template));
    }
    // Empty is meaningful (= $SHELL), so it is stored too: a shell once set can be cleared again.
    launch("shell", Value::Str(stripped("shell")));
    launch("editor", Value::Str(stripped("editor")));
    launch("rdp_client", optional("rdp_client"));
    // Not stored while it is the default and was never chosen, so an untouched config stays as is.
    let resolution = {
        let t = stripped("rdp_resolution").to_lowercase();
        if rdp::resolution_setting_is_valid(&t) {
            t
        } else {
            rdp::RDP_RESOLUTION_DEFAULT.to_owned()
        }
    };
    if resolution_chosen_before || resolution != rdp::RDP_RESOLUTION_DEFAULT {
        launch("rdp_resolution", Value::Str(resolution));
    }
    launch("vnc_client", optional("vnc_client"));
    launch("probe_timeout", Value::Int(num("probe_timeout", 1, 60)));

    if let Some(list) = draft.get("environments") {
        environments::apply(cfg, list);
    }
}

/// The names of the compiled-in schemes and of the
/// `*.colorscheme` files in `dirs`, once each, sorted ignoring case.
pub fn scheme_names(builtin: Vec<String>, dirs: &[std::path::PathBuf]) -> Vec<String> {
    let mut names: BTreeSet<String> = builtin.into_iter().collect();
    for dir in dirs {
        names.extend(scheme_files(dir));
    }
    let mut names: Vec<String> = names.into_iter().collect();
    names.sort_by_key(|n| n.to_lowercase());
    names
}

fn scheme_files(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".colorscheme").map(str::to_owned)
        })
        .collect()
}

#[cfg(test)]
mod tests;
