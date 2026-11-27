use cxx_qt_build::{CxxQtBuilder, QmlModule};
use qt_build_utils::{QResource, QResourceFile, QResources};

/// QML files are compiled by qmlcachegen and embedded as qrc:/qt/qml/Termigator/qml/<area>/...
const QML_FILES: &[&str] = &[
    "qml/panes/PanesArea.qml",
    "qml/panes/PaneView.qml",
    "qml/panes/SplitGrip.qml",
    "qml/panes/PaneTab.qml",
    "qml/panes/DetachedWindow.qml",
    "qml/panes/DockOverlay.qml",
    "qml/panes/TabDrag.qml",
    "qml/panes/PanesSmoke.qml",
    "qml/shell/FollowSmoke.qml",
    "qml/shell/QuickConnectSmoke.qml",
    "qml/shell/LocalShellCheck.qml",
    "qml/shell/SshCheck.qml",
    "qml/shell/ShellMenuCheck.qml",
    "qml/import/ImportWinCheck.qml",
    "qml/shell/SoundCheck.qml",
    "qml/terminal/TerminalHost.qml",
    "qml/terminal/TabSession.qml",
    "qml/terminal/TerminalView.qml",
    "qml/terminal/TerminalFindBar.qml",
    "qml/terminal/TerminalSmoke.qml",
    "qml/terminal/GridCheck.qml",
    "qml/shell/ShellWindow.qml",
    "qml/shell/ShellActions.qml",
    "qml/shell/ShellToolBar.qml",
    "qml/shell/QuickConnectField.qml",
    "qml/shell/ShortcutsDialog.qml",
    "qml/shell/DialogSize.qml",
    "qml/shell/AppPalette.qml",
    "qml/shell/MessageBox.qml",
    "qml/shell/MultiExecBanner.qml",
    "qml/shell/UpdateBanner.qml",
    "qml/shell/UpdateCheck.qml",
    "qml/shell/EnvMark.qml",
    "qml/shell/KeyedMenuContent.qml",
    "qml/shell/SingleInstanceSmoke.qml",
    "qml/shell/SmokeCheck.qml",
    "qml/shell/StartupCheck.qml",
    "qml/shell/RestartCheck.qml",
    "qml/shell/CloseChoiceCheck.qml",
    "qml/shell/LookCheck.qml",
    "qml/shell/ShowcaseCheck.qml",
    "qml/shell/ThemeSwitchCheck.qml",
    "qml/preferences/PrefWindow.qml",
    "qml/preferences/PreferencesDialog.qml",
    "qml/preferences/PrefPage.qml",
    "qml/preferences/PrefHint.qml",
    "qml/preferences/PrefCheck.qml",
    "qml/preferences/PrefSpin.qml",
    "qml/preferences/PrefField.qml",
    "qml/preferences/PrefCombo.qml",
    "qml/preferences/AppearancePage.qml",
    "qml/preferences/TerminalPage.qml",
    "qml/preferences/ClipboardPage.qml",
    "qml/preferences/LoggingPage.qml",
    "qml/preferences/ConnectionsPage.qml",
    "qml/preferences/EnvironmentsPage.qml",
    "qml/preferences/ApplicationPage.qml",
    "qml/preferences/PreferencesSmoke.qml",
    "qml/editors/EditorHost.qml",
    "qml/editors/EditorDialog.qml",
    "qml/editors/EdField.qml",
    "qml/editors/EdCheck.qml",
    "qml/editors/EdArea.qml",
    "qml/editors/EdCombo.qml",
    "qml/editors/DataCombo.qml",
    "qml/editors/TerminalCombo.qml",
    "qml/editors/EnvironmentCombo.qml",
    "qml/editors/GatewayCombo.qml",
    "qml/editors/PortSpin.qml",
    "qml/editors/KeyPathField.qml",
    "qml/editors/CredentialSync.qml",
    "qml/editors/SshDialog.qml",
    "qml/editors/RdpDialog.qml",
    "qml/editors/VncDialog.qml",
    "qml/editors/TelnetDialog.qml",
    "qml/editors/SerialDialog.qml",
    "qml/editors/FavoriteDialog.qml",
    "qml/editors/QuickSshDialog.qml",
    "qml/editors/TestConnectionDialog.qml",
    "qml/editors/EditorsSmoke.qml",
    "qml/sidebar/HostsPanel.qml",
    "qml/sidebar/HostRow.qml",
    "qml/sidebar/PromptDialog.qml",
    "qml/sidebar/DeleteFolderDialog.qml",
    "qml/sidebar/ConnectionTestDialog.qml",
    "qml/sidebar/SidebarSmoke.qml",
    "qml/sidebar/HostsKeysSmoke.qml",
    "qml/tools/ToolDialog.qml",
    "qml/tools/ToolsHost.qml",
    "qml/tools/TunnelsDialog.qml",
    "qml/tools/TunnelEditDialog.qml",
    "qml/tools/SnippetsDialog.qml",
    "qml/tools/BulkEditDialog.qml",
    "qml/tools/DisconnectNotice.qml",
    "qml/tools/ToolsSmoke.qml",
    "qml/vault/VaultHost.qml",
    "qml/vault/VaultDialog.qml",
    "qml/vault/PasswordDialog.qml",
    "qml/vault/ChoiceDialog.qml",
    "qml/vault/MessageBoxDialog.qml",
    "qml/vault/MasterPasswordDialog.qml",
    "qml/vault/CredentialsDialog.qml",
    "qml/vault/GenerateKeyDialog.qml",
    "qml/vault/SshKeysDialog.qml",
    "qml/vault/VaultSmoke.qml",
    "qml/files/FilesHost.qml",
    "qml/files/FilesSession.qml",
    "qml/files/FilesSidePanel.qml",
    "qml/files/FilesPanel.qml",
    "qml/files/RemoteList.qml",
    "qml/files/ColumnGrip.qml",
    "qml/files/HeaderCell.qml",
    "qml/files/LocalPane.qml",
    "qml/files/TransfersView.qml",
    "qml/files/PermissionsDialog.qml",
    "qml/files/SyncDialog.qml",
    "qml/files/FilesSmoke.qml",
    "qml/files/FilesKeysSmoke.qml",
    "qml/files/FilesCheck.qml",
    "qml/files/FilesAuthCheck.qml",
    "qml/files/FilesFollowCheck.qml",
    "qml/files/LocalCwdCheck.qml",
    "qml/import/ImportHost.qml",
    "qml/import/ImportSmoke.qml",
];

fn main() {
    #[cfg(windows)]
    embed_windows_icon();
    let builder = CxxQtBuilder::new_qml_module(QmlModule::new("Termigator").qml_files(QML_FILES))
        .files([
            "src/shell_qobj.rs",
            "src/preferences_qobj.rs",
            "src/editors_qobj.rs",
            "src/panes_qobj.rs",
            "src/sidebar_qobj.rs",
            "src/terminal_qobj.rs",
            "src/tools_qobj.rs",
            "src/vault_qobj.rs",
            "src/files_qobj.rs",
            "src/files_sync_qobj.rs",
            "src/files_helper_qobj.rs",
            "src/import_qobj.rs",
            "src/transfers_qobj.rs",
        ])
        .cpp_file("src/shell_native.cpp")
        .cpp_file("src/preferences_native.cpp")
        .cpp_file("src/terminal_native.cpp")
        .cpp_file("src/message_native.h")
        .cpp_file("src/message_native.cpp")
        .cpp_file("src/session_native.h")
        .cpp_file("src/session_native.cpp")
        .qt_module("Quick")
        .qt_module("Widgets")
        // The app icon (an installed binary has no source tree), for the empty-pane placeholder:
        // qrc:/qt/qml/Termigator/qml/assets/termigator-icon.svg
        .qrc_resources(
            QResources::new().resource(
                [16, 24, 32, 48, 64, 128, 256, 512].into_iter().fold(
                    QResource::new().prefix("/qt/qml/Termigator").file(
                        QResourceFile::new("../../assets/termigator-icon.svg")
                            .alias("qml/assets/termigator-icon.svg"),
                    ),
                    // The window icon of an installed build without the hicolor theme icon
                    // (Windows): setAppIcon reads qml/assets/icons from here.
                    |r, size| {
                        r.file(
                            QResourceFile::new(format!("../../assets/icons/termigator-{size}.png"))
                                .alias(format!("qml/assets/icons/termigator-{size}.png")),
                        )
                    },
                ),
            ),
        );
    // applyFixedLook (shell_native.cpp): KF6 colour schemes and the Breeze icon theme.
    // Windows: KF6 comes from the same KDE Craft root as qmake (`<root>\\bin\\qmake.exe`), headers
    // in `<root>\\include\\KF6`, import libraries in `<root>\\lib`. With it the Breeze icon theme
    // compiled into KF6BreezeIcons.dll is the app's icon theme there (no system theme on Windows).
    let craft_root = (std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows"))
        .then(|| std::env::var_os("QMAKE"))
        .flatten()
        .map(std::path::PathBuf::from)
        .and_then(|q| Some(q.parent()?.parent()?.to_path_buf()))
        .filter(|root| root.join("include").join("KF6").is_dir());
    if let Some(root) = &craft_root {
        println!(
            "cargo:rustc-link-search=native={}",
            root.join("lib").display()
        );
    }
    // MessageBeep (message_native.cpp): the message box and bell sounds on Windows.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-lib=user32");
        println!("cargo:rustc-link-lib=advapi32"); // RegGetValueW
    }
    let kf6 = std::env::var("KF6_INCLUDE_DIR").unwrap_or_else(|_| match &craft_root {
        Some(root) => root.join("include").join("KF6").display().to_string(),
        None => "/usr/include/KF6".to_owned(),
    });
    // Optional KF6 features: link only when the development headers are installed. Without them
    // the guarded code paths in shell_native.cpp/message_native.cpp are compiled out, so a plain
    // Qt-only build still works (no fixed Breeze palette, no KDE notification sounds).
    let header = |path: &str| std::path::Path::new(path).exists();
    let have_kcolor = header(&format!("{kf6}/KColorScheme/KColorScheme"))
        && header(&format!("{kf6}/KConfigCore/KSharedConfig"));
    let have_icons = header(&format!("{kf6}/KIconThemes/KIconTheme"));
    let have_breeze = header(&format!("{kf6}/BreezeIcons/BreezeIcons"));
    let have_notifications = header(&format!("{kf6}/KNotifications/KNotification"));
    // The Breeze icons do not need the -dev headers: they are compiled into libKF6BreezeIcons, and
    // the only entry point (`BreezeIcons::initIcons`) is declared in shell_native.cpp when this
    // build links the runtime library from a symlink here (its soname keeps the runtime name).
    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let breeze_runtime_link = if have_breeze {
        None
    } else {
        find_runtime_lib("KF6BreezeIcons").and_then(|lib| {
            let link = out_dir.join("libKF6BreezeIcons.so");
            std::fs::remove_file(&link).ok();
            symlink(&lib, &link).ok().map(|()| out_dir.clone())
        })
    };
    // SAFETY: the closure only adds include directories and one define.
    let builder = unsafe {
        builder.cc_builder(|cc| {
            cc.include(&kf6);
            for module in [
                "KColorScheme",
                "KConfig",
                "KConfigCore",
                "KConfigGui",
                "KCoreAddons",
                "KGuiAddons",
                "KWidgetsAddons",
                "BreezeIcons",
                "KIconThemes",
                "KNotifications",
            ] {
                cc.include(format!("{kf6}/{module}"));
            }
            if breeze_runtime_link.is_some() {
                cc.define("TERMIGATOR_BREEZE_ICONS_RUNTIME", None::<&str>);
            }
        })
    };
    if let Some(dir) = &breeze_runtime_link {
        println!("cargo:rustc-link-search=native={}", dir.display());
    }
    for lib in [
        have_kcolor.then_some("KF6ColorScheme"),
        have_kcolor.then_some("KF6ConfigCore"),
        (have_breeze || breeze_runtime_link.is_some()).then_some("KF6BreezeIcons"),
        have_icons.then_some("KF6IconThemes"),
        have_notifications.then_some("KF6Notifications"),
    ]
    .into_iter()
    .flatten()
    {
        println!("cargo:rustc-link-lib={lib}");
    }
    remove_stale_qmltypes(&out_dir);
    builder.build();
}

/// qt-build-utils 0.10 runs qmlcachegen before qmltyperegistrar rewrites `plugin.qmltypes`, so an
/// incremental build compiles the QML ahead-of-time against the previous build's QObjects: after
/// an invokable is added or moved, the compiled QML calls methods by their old index and crashes
/// (a timer's `backend.poll()` in ToolsHost.qml). Without the old file it compiles like a clean
/// build.
fn remove_stale_qmltypes(out_dir: &std::path::Path) {
    let qmltypes = out_dir.join("qt-build-utils/qml_modules/Termigator/plugin.qmltypes");
    match std::fs::remove_file(&qmltypes) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => println!(
            "cargo:warning=cannot remove {} ({e}); the QML may be compiled against old types",
            qmltypes.display()
        ),
    }
}

/// The app icon as resource `IDI_ICON1` of termigator.exe: Explorer and the shortcuts show it, and
/// Qt's Windows platform takes it as the default window icon.
#[cfg(windows)]
fn embed_windows_icon() {
    let ico = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/termigator.ico");
    println!("cargo:rerun-if-changed={}", ico.display());
    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let rc = out_dir.join("termigator.rc");
    // rc.exe reads `\\` in a string as one backslash.
    let line = format!(
        "IDI_ICON1 ICON \"{}\"\n",
        ico.display().to_string().replace('\\', "\\\\")
    );
    if let Err(e) = std::fs::write(&rc, line) {
        println!("cargo:warning=app icon not embedded: {e}");
        return;
    }
    if let Err(e) = embed_resource::compile(&rc, embed_resource::NONE).manifest_optional() {
        println!("cargo:warning=app icon not embedded: {e:?}");
    }
}

#[cfg(unix)]
fn symlink(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(from, to)
}

/// No KF6 runtime library to link on other platforms (Windows builds without KDE).
#[cfg(not(unix))]
fn symlink(_from: &std::path::Path, _to: &std::path::Path) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}

/// The runtime library (`libX.so.6`…) of a KF6 module, for builds without its `-dev` symlink.
fn find_runtime_lib(name: &str) -> Option<std::path::PathBuf> {
    let so = format!("lib{name}.so");
    for dir in [
        "/usr/lib/x86_64-linux-gnu",
        "/usr/lib64",
        "/usr/lib",
        "/usr/local/lib",
    ] {
        let dir = std::path::Path::new(dir);
        if dir.join(&so).exists() {
            return Some(dir.join(&so));
        }
        let mut versions: Vec<_> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(&format!("{so}.")))
            })
            .collect();
        versions.sort();
        if let Some(newest) = versions.pop() {
            return Some(newest);
        }
    }
    None
}
