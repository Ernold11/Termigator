//! Builds the bridge's C++ glue with cxx + moc + rcc, and embeds this project's colour schemes.
//!
//! Terminal emulation is Rust (`ghostty-vt`); the C++ here is only Qt plumbing: the QML
//! view (`terminal_item`), the session object (`ghostty_session`), MultiExec, the probe and KDE
//! integration. The vendored QMLTermWidget is gone.

use std::{
    env,
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use qt_build_utils::{MocArguments, QResource, QResourceFile, QResources, QtBuild};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// Qt modules of the glue: QML/Quick for the view, Widgets for dialogs, DBus for KGlobalAccel.
const QT_MODULES: [&str; 7] = ["Core", "Gui", "Widgets", "Network", "Qml", "Quick", "DBus"];

fn main() -> Result<()> {
    let manifest =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").ok_or("CARGO_MANIFEST_DIR unset")?);
    let out = PathBuf::from(env::var_os("OUT_DIR").ok_or("OUT_DIR unset")?);
    for path in ["src", "color-schemes"] {
        println!("cargo::rerun-if-changed={path}");
    }

    let mut qt = QtBuild::new(QT_MODULES.map(String::from).to_vec())?;
    let moc = qt.moc();
    let resources = qt.rcc().compile(write_qrc(&manifest, &out)?);

    let mut glue = cxx_build::bridge("src/lib.rs");
    qt.cargo_link_libraries(&mut glue);
    for name in ["multiexec", "probe", "terminal_item", "ghostty_session"] {
        let header = manifest.join(format!("src/{name}.h"));
        let header_moc = moc.compile(
            header,
            MocArguments::default().include_path(out.join("cxxbridge").join("include")),
        );
        glue.file(format!("src/{name}.cpp")).file(header_moc.cpp);
    }
    // KDE integration (shortcut, window raising) exists on Linux only; kde.cpp builds as stubs without it.
    let kf6 = if env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows") {
        Vec::new()
    } else {
        kf6_paths()?
    };
    // Qt needs a correct `__cplusplus` from MSVC.
    if glue.get_compiler().is_like_msvc() {
        glue.flag("/Zc:__cplusplus").flag("/permissive-");
    }
    glue.file("src/register.cpp")
        .file("src/kde.cpp")
        .files(resources.file)
        .std("c++17")
        .includes(qt.include_paths())
        .includes(&kf6)
        .compile("termigator_term_bridge");
    Ok(())
}

/// Our built-in colour schemes, served as `:/termigator/color-schemes` (the same `.colorscheme`
/// format Konsole uses). Written for this project, so the binary stays MIT.
fn write_qrc(manifest: &Path, out: &Path) -> Result<PathBuf> {
    let dir = manifest.join("color-schemes");
    let mut resource = QResource::new().prefix("/termigator/color-schemes");
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "colorscheme"))
        .collect();
    files.sort();
    for path in &files {
        let name = path
            .file_name()
            .ok_or("resource without a name")?
            .to_string_lossy();
        resource = resource.file(QResourceFile::new(path).alias(name.as_ref()));
    }
    // The name decides Q_INIT_RESOURCE(name) in register.cpp.
    let qrc = out.join("termigator.qrc");
    QResources::new()
        .resource(resource)
        .write(&mut fs::File::create(&qrc)?)?;
    Ok(qrc)
}

/// Include directories of KF6 GlobalAccel and WindowSystem (src/kde.cpp), and their link flags.
/// Only WindowSystem ships a pkg-config file; GlobalAccel's headers sit next to it under `KF6/`.
fn kf6_paths() -> Result<Vec<PathBuf>> {
    let variable = |name: &str| -> Result<String> {
        let out = Command::new("pkg-config")
            .args(["--variable", name, "KF6WindowSystem"])
            .output()?;
        if !out.status.success() {
            return Err("pkg-config cannot find KF6WindowSystem (install KDE Frameworks 6)".into());
        }
        Ok(String::from_utf8(out.stdout)?.trim().to_owned())
    };
    let window_system = PathBuf::from(variable("includedir")?);
    let global_accel = window_system
        .parent()
        .ok_or("KF6WindowSystem includedir has no parent")?
        .join("KGlobalAccel");
    println!("cargo::rustc-link-search=native={}", variable("libdir")?);
    for lib in ["KF6GlobalAccel", "KF6WindowSystem"] {
        println!("cargo::rustc-link-lib={lib}");
    }
    Ok(vec![window_system, global_accel])
}
