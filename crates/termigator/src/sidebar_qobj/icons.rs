//! Distro badges and logos, row icon names and context-menu icons.

use super::*;

/// Distro styles: (id, badge colour, glyph, name).
const DISTRO_STYLE: [(&str, &str, &str, &str); 19] = [
    ("ubuntu", "#E95420", "U", "Ubuntu"),
    ("debian", "#A81D33", "D", "Debian"),
    ("fedora", "#3C6EB4", "f", "Fedora"),
    ("arch", "#1793D1", "A", "Arch Linux"),
    ("centos", "#9CCD2A", "C", "CentOS"),
    ("rocky", "#10B981", "R", "Rocky Linux"),
    ("almalinux", "#1A6CB5", "A", "AlmaLinux"),
    ("rhel", "#EE0000", "R", "Red Hat"),
    ("opensuse", "#73BA25", "S", "openSUSE"),
    ("alpine", "#0D597F", "A", "Alpine"),
    ("mint", "#86BE43", "M", "Linux Mint"),
    ("raspbian", "#C51A4A", "R", "Raspberry Pi OS"),
    ("manjaro", "#35BF5C", "M", "Manjaro"),
    ("kali", "#2776B6", "K", "Kali Linux"),
    ("gentoo", "#54487A", "G", "Gentoo"),
    ("void", "#478061", "V", "Void Linux"),
    ("pop", "#48B9C7", "P", "Pop!_OS"),
    ("amazon", "#FF9900", "A", "Amazon Linux"),
    ("oracle", "#C74634", "O", "Oracle Linux"),
];

/// `(colour, glyph)` of a distro badge; unknown ids get grey and their initial.
pub fn distro_badge(id: &str) -> (String, String) {
    match DISTRO_STYLE.iter().find(|s| s.0 == id) {
        Some(s) => (s.1.to_owned(), s.2.to_owned()),
        None => (
            "#6E7681".to_owned(),
            id.chars()
                .next()
                .map_or_else(|| "?".to_owned(), |c| c.to_uppercase().collect()),
        ),
    }
}

/// Display label of a distro id.
pub fn distro_label(id: &str) -> String {
    match DISTRO_STYLE.iter().find(|s| s.0 == id) {
        Some(s) => s.3.to_owned(),
        None if id.is_empty() => "Linux".to_owned(),
        None => id.to_owned(),
    }
}

/// The distro logos (white on the badge), embedded so an installed binary needs no source tree.
pub(super) const DISTRO_LOGOS: &[(&str, &str)] = &[
    (
        "almalinux",
        include_str!("../../../../assets/distros/almalinux.svg"),
    ),
    (
        "alpine",
        include_str!("../../../../assets/distros/alpine.svg"),
    ),
    ("arch", include_str!("../../../../assets/distros/arch.svg")),
    (
        "centos",
        include_str!("../../../../assets/distros/centos.svg"),
    ),
    (
        "debian",
        include_str!("../../../../assets/distros/debian.svg"),
    ),
    (
        "fedora",
        include_str!("../../../../assets/distros/fedora.svg"),
    ),
    (
        "gentoo",
        include_str!("../../../../assets/distros/gentoo.svg"),
    ),
    ("kali", include_str!("../../../../assets/distros/kali.svg")),
    (
        "manjaro",
        include_str!("../../../../assets/distros/manjaro.svg"),
    ),
    ("mint", include_str!("../../../../assets/distros/mint.svg")),
    (
        "opensuse",
        include_str!("../../../../assets/distros/opensuse.svg"),
    ),
    ("pop", include_str!("../../../../assets/distros/pop.svg")),
    (
        "raspbian",
        include_str!("../../../../assets/distros/raspbian.svg"),
    ),
    ("rhel", include_str!("../../../../assets/distros/rhel.svg")),
    (
        "rocky",
        include_str!("../../../../assets/distros/rocky.svg"),
    ),
    (
        "ubuntu",
        include_str!("../../../../assets/distros/ubuntu.svg"),
    ),
    ("void", include_str!("../../../../assets/distros/void.svg")),
];

/// The bundled logo of a distro (drawn white on its badge) as a file URL, or "". Qt needs a file,
/// so the embedded SVG is written once to `$XDG_CACHE_HOME/termigator/distros/<id>.svg`.
pub(super) fn distro_logo_url(id: &str) -> String {
    let Some((_, svg)) = DISTRO_LOGOS.iter().find(|(name, _)| *name == id) else {
        return String::new();
    };
    let base = std::env::var_os("XDG_CACHE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::home_dir().map(|h| h.join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join("termigator").join("distros");
    let file = dir.join(format!("{id}.svg"));
    let current = std::fs::read_to_string(&file).is_ok_and(|t| t == *svg);
    if !current && (std::fs::create_dir_all(&dir).is_err() || std::fs::write(&file, svg).is_err()) {
        return String::new();
    }
    QUrl::from_local_file(&qs(&file.to_string_lossy()))
        .to_qstring()
        .to_string()
}

pub(super) fn icon_name(icon: &Icon) -> String {
    match icon {
        Icon::Group => "folder",
        Icon::Recent => "document-open-recent",
        Icon::Connection => "network-server",
        Icon::ConnectionError => "dialog-warning",
        Icon::Remote => "preferences-desktop-remote-desktop",
        Icon::Serial => "network-modem",
        Icon::Favorite => "utilities-terminal",
        Icon::Distro(_) => "",
        Icon::Custom(name) => name,
    }
    .to_owned()
}

/// Theme icons of context-menu entries by text prefix.
const MENU_ICONS: &[(&str, &str)] = &[
    ("Connect", "media-playback-start"),
    ("Test connection", "network-connect"),
    ("Copy", "edit-copy"),
    ("Cut", "edit-cut"),
    ("Paste", "edit-paste"),
    ("Open files", "folder-remote"),
    ("Upload", "cloud-upload|go-up"),
    ("Download", "download|go-down"),
    ("Edit", "document-edit"),
    ("Duplicate entry", "edit-duplicate"),
    ("Duplicate tab", "tab-duplicate"),
    ("Move to", "go-jump"),
    ("Move up", "go-up"),
    ("Move down", "go-down"),
    ("Move", "go-jump"),
    ("Delete", "edit-delete"),
    ("Remove", "edit-delete"),
    ("Rename", "edit-rename"),
    ("Add SSH", "utilities-terminal"),
    ("SSH connection", "utilities-terminal"),
    ("Add RDP", "krdc|computer"),
    ("RDP connection", "krdc|computer"),
    ("Add VNC", "krdc|computer"),
    ("VNC connection", "krdc|computer"),
    ("Add Telnet", "network-wired"),
    ("Telnet connection", "network-wired"),
    ("Add serial", "network-modem"),
    ("Serial connection", "network-modem"),
    ("Add favorite", "bookmark-new"),
    ("Favorite command", "bookmark-new"),
    ("New folder", "folder-new"),
    ("New subfolder", "folder-new"),
    ("Folder", "folder-new"),
    ("Collapse", "collapse-all"),
    ("Expand", "expand-all"),
    ("Import", "document-import"),
    ("Reconnect", "view-refresh"),
    ("Log output", "document-save"),
    ("Stop session log", "process-stop"),
    ("Split right", "view-split-left-right"),
    ("Split down", "view-split-top-bottom"),
    ("Close pane", "view-close"),
    ("Close tab", "tab-close"),
    ("Set icon", "preferences-desktop-icons"),
    ("Local shell", "utilities-terminal"),
    ("Exclude from MultiExec", "system-run"),
    ("Start", "media-playback-start"),
    ("Stop", "media-playback-stop"),
    ("Detach", "window-new"),
    ("Attach", "window-restore|go-previous"),
    ("Record", "media-record"),
    ("Stop recording", "media-playback-stop"),
    ("Add", "list-add"),
    ("Network tools", "network-wired"),
    ("Ping", "network-wired"),
    ("Trace route", "network-workgroup|network-wired"),
    ("DNS lookup", "edit-find"),
    ("Check port", "network-connect"),
    ("Scan common ports", "security-high|network-wired"),
];

/// The icon names (`a|b`) for a menu text: the longest matching prefix, mnemonics ignored.
pub fn menu_icon(text: &str) -> Option<&'static str> {
    let plain = text.replace('&', "");
    MENU_ICONS
        .iter()
        .filter(|(prefix, _)| plain.starts_with(prefix))
        .max_by_key(|(prefix, _)| prefix.len())
        .map(|(_, icon)| *icon)
}
