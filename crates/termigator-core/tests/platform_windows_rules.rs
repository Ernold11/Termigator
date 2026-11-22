//! Windows rules that are plain functions, checked on any host: `PATHEXT`, config base,
//! the Windows network-tool argv and `ShellCommand::Argv`.

use std::ffi::OsString;

use termigator_core::paths::config_base_for;
use termigator_core::platform::pathext_names;
use termigator_core::proc::pty::ShellCommand;
use termigator_core::proto::nettools::tool_argv_windows;

#[test]
fn pathext_appends_extensions_unless_present() {
    let ext = ".COM;.EXE;.BAT;.CMD";
    assert_eq!(
        pathext_names("ssh", ext),
        ["ssh.COM", "ssh.EXE", "ssh.BAT", "ssh.CMD"]
    );
    assert_eq!(pathext_names("ssh.exe", ext), ["ssh.exe"]);
    assert_eq!(pathext_names("Plink.EXE", ext), ["Plink.EXE"]);
    assert_eq!(pathext_names("x", ""), Vec::<String>::new());
    assert_eq!(pathext_names("x", ";.EXE;"), ["x.EXE"]);
}

#[test]
fn config_base_windows_uses_appdata() {
    let env = |k: &str| match k {
        "APPDATA" => Some(OsString::from(r"C:\Users\me\AppData\Roaming")),
        "HOME" => Some(OsString::from("/home/me")),
        _ => None,
    };
    assert_eq!(
        config_base_for(true, &env).to_str(),
        Some(r"C:\Users\me\AppData\Roaming")
    );
    assert_eq!(
        config_base_for(false, &env),
        std::path::Path::new("/home/me").join(".config")
    );
    let xdg = |k: &str| (k == "XDG_CONFIG_HOME").then(|| OsString::from("/x"));
    assert_eq!(config_base_for(true, &xdg), std::path::Path::new("/x"));
}

#[test]
fn windows_tool_argv() {
    assert_eq!(
        tool_argv_windows("ping", "10.0.0.1", 22).unwrap(),
        ["ping", "-t", "10.0.0.1"]
    );
    assert_eq!(
        tool_argv_windows("trace", "h", 22).unwrap(),
        ["tracert", "h"]
    );
    assert_eq!(
        tool_argv_windows("dns", "h", 22).unwrap(),
        ["nslookup", "h"]
    );
    let port = tool_argv_windows("port", "h", 2222).unwrap();
    assert_eq!(
        port[port.len() - 4..],
        ["-ComputerName", "h", "-Port", "2222"]
    );
    assert!(tool_argv_windows("scan", "h", 22).is_err());
    assert!(tool_argv_windows("ping", "-evil", 22).is_err());
    assert!(tool_argv_windows("ping", "a; calc", 22).is_err());
}

#[test]
fn argv_command_is_run_as_given() {
    let c = ShellCommand::Argv(vec!["plink.exe".into(), "-telnet".into(), "h".into()]);
    assert_eq!(c, c.clone());
}
