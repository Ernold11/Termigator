//! `termigator_core::terminal_background`: the terminal's background opacity and blur from the
//! settings, else from the Konsole colour scheme.
#![cfg(unix)]
#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use termigator_core::config::doc;
use termigator_core::konsole::XdgDirs;
use termigator_core::terminal_background::{Background, MIN_OPACITY, configured, scheme_effects};

static SEQ: AtomicU32 = AtomicU32::new(0);

/// A private XDG tree whose Konsole data dir holds `schemes` (name, file text).
fn xdg_with(schemes: &[(&str, &str)]) -> XdgDirs {
    let root = std::env::temp_dir().join(format!(
        "tg-bg-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::SeqCst)
    ));
    let data = root.join("data");
    fs::create_dir_all(data.join("konsole")).unwrap();
    for (name, text) in schemes {
        fs::write(
            data.join("konsole").join(format!("{name}.colorscheme")),
            text,
        )
        .unwrap();
    }
    XdgDirs {
        home: root.clone(),
        config_home: Some(root.join("config").display().to_string()),
        data_home: Some(data.display().to_string()),
        data_dirs: Some(PathBuf::from("/nonexistent").display().to_string()),
    }
}

const GLASS: &str =
    "[General]\nDescription=Glass\nOpacity=0.75\nBlur=true\n\n[Background]\nColor=0,0,0\n";
const PLAIN: &str = "[General]\nDescription=Plain\n\n[Background]\nColor=0,0,0\n";

fn launch(yaml: &str) -> termigator_core::config::doc::Value {
    doc::load(yaml).expect("yaml")
}

#[test]
fn scheme_effects_read_opacity_and_blur() {
    let xdg = xdg_with(&[("Glass", GLASS), ("Plain", PLAIN)]);
    assert_eq!(scheme_effects("Glass", &xdg), (Some(75), Some(true)));
    assert_eq!(
        scheme_effects("Glass.colorscheme", &xdg),
        (Some(75), Some(true))
    );
    assert_eq!(scheme_effects("Plain", &xdg), (None, None));
    assert_eq!(scheme_effects("Missing", &xdg), (None, None));
}

#[test]
fn defaults_come_from_the_scheme_then_full() {
    let xdg = xdg_with(&[("Glass", GLASS), ("Plain", PLAIN)]);
    let full = Background {
        opacity: 100,
        blur: false,
    };
    assert_eq!(configured(&launch("{}"), &xdg), full);
    assert_eq!(configured(&launch("{color_scheme: Plain}"), &xdg), full);
    assert_eq!(
        configured(&launch("{color_scheme: Glass}"), &xdg),
        Background {
            opacity: 75,
            blur: true
        }
    );
}

#[test]
fn settings_override_the_scheme_and_are_clamped() {
    let xdg = xdg_with(&[("Glass", GLASS)]);
    let bg = |yaml: &str| configured(&launch(yaml), &xdg);
    assert_eq!(
        bg("{color_scheme: Glass, background_opacity: 90, background_blur: false}"),
        Background {
            opacity: 90,
            blur: false
        }
    );
    assert_eq!(bg("{background_opacity: 5}").opacity, MIN_OPACITY);
    assert_eq!(bg("{background_opacity: 250}").opacity, 100);
    // A null / non-number key means "not set": the scheme's value.
    assert_eq!(
        bg("{color_scheme: Glass, background_opacity: null}").opacity,
        75
    );
    assert_eq!(
        bg("{color_scheme: Glass, background_opacity: lots}").opacity,
        75
    );
}

#[test]
fn the_konsole_profile_scheme_counts_when_none_is_set() {
    let xdg = xdg_with(&[("Glass", GLASS)]);
    let cfg = PathBuf::from(xdg.config_home.clone().unwrap());
    fs::create_dir_all(&cfg).unwrap();
    fs::write(
        cfg.join("konsolerc"),
        "[Desktop Entry]\nDefaultProfile=Mine.profile\n",
    )
    .unwrap();
    let data = PathBuf::from(xdg.data_home.clone().unwrap()).join("konsole");
    fs::write(
        data.join("Mine.profile"),
        "[Appearance]\nColorScheme=Glass\n",
    )
    .unwrap();
    assert_eq!(configured(&launch("{}"), &xdg).opacity, 75);
}
