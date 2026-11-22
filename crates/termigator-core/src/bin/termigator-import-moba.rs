//! Import the SSH bookmarks of a MobaXterm export into Termigator's session file.
//!
//! It lives in `termigator-core` because it needs no Qt: the `termigator` package links
//! the cxx-qt glue into every binary it holds. The session file is written through the shared
//! config save path (atomic, `TERMIGATOR_CONFIG_DIR` honoured), which also avoids a non-atomic
//! write, lost anchors and an ignored config dir.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use termigator_core::config::doc::{Value, dump};
use termigator_core::config::save::{load, save};
use termigator_core::import::moba::{
    decode_moba_bytes, local_user, merge_moba_connections, parse_moba_bookmarks,
};
use termigator_core::paths::{Layout, Paths, ProcessEnv};
use termigator_core::vault;

const USAGE: &str =
    "usage: termigator-import-moba [-h] [--session SESSION] [--config CONFIG] [--dry-run] moba_ini";
const HEADER: &str = "# Termigator SSH connections — merged from MobaXterm import.\n";

struct Args {
    moba_ini: PathBuf,
    session: Option<PathBuf>,
    config: Option<PathBuf>,
    dry_run: bool,
}

enum Parsed {
    Run(Args),
    Help,
    Error(String),
}

fn parse_args(args: impl IntoIterator<Item = OsString>) -> Parsed {
    let mut it = args.into_iter();
    let (mut ini, mut session, mut config, mut dry_run) = (None, None, None, false);
    while let Some(a) = it.next() {
        let s = a.to_string_lossy().into_owned();
        let (name, inline) = match s.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n.to_owned(), Some(OsString::from(v))),
            _ => (s.clone(), None),
        };
        match name.as_str() {
            "-h" | "--help" => return Parsed::Help,
            "--dry-run" if inline.is_none() => dry_run = true,
            "--session" | "--config" => {
                let Some(v) = inline.or_else(|| it.next()) else {
                    return Parsed::Error(format!("argument {name}: expected one argument"));
                };
                if name == "--session" {
                    session = Some(PathBuf::from(v));
                } else {
                    config = Some(PathBuf::from(v));
                }
            }
            _ if s.starts_with('-') && s.len() > 1 => {
                return Parsed::Error(format!("unrecognized arguments: {s}"));
            }
            _ if ini.is_none() => ini = Some(PathBuf::from(a)),
            _ => return Parsed::Error(format!("unrecognized arguments: {s}")),
        }
    }
    match ini {
        Some(moba_ini) => Parsed::Run(Args {
            moba_ini,
            session,
            config,
            dry_run,
        }),
        None => Parsed::Error("the following arguments are required: moba_ini".to_owned()),
    }
}

fn run(args: Args) -> Result<ExitCode, String> {
    let bytes =
        std::fs::read(&args.moba_ini).map_err(|e| format!("{}: {e}", args.moba_ini.display()))?;
    let text =
        decode_moba_bytes(&bytes).map_err(|e| format!("{}: {e}", args.moba_ini.display()))?;
    let parsed = parse_moba_bookmarks(&text, &local_user());
    if parsed.is_empty() {
        eprintln!("No #109 SSH bookmarks found.");
        return Ok(ExitCode::FAILURE);
    }

    let mut paths = Paths::resolve(Layout::ACTIVE, &ProcessEnv, std::path::Path::new("/"));
    if let Some(p) = args.session {
        paths.session_path = p;
    }
    if let Some(p) = args.config {
        paths.config_path = p;
    }

    let mut cfg = load(&paths).map_err(|e| e.to_string())?;
    let terminal = cfg
        .get("window")
        .and_then(|w| w.get("default_terminal"))
        .filter(|v| termigator_core::config::model::truthy(v))
        .map_or_else(
            || "embedded".to_owned(),
            termigator_core::config::model::value_str,
        );
    let existing = match cfg.get("connections") {
        Some(Value::Seq(c)) => c.clone(),
        _ => Vec::new(),
    };
    let (merged, added) = merge_moba_connections(&existing, &parsed, &terminal);

    if args.dry_run {
        println!("{HEADER}{}", dump(&Value::Seq(merged)));
        eprintln!(
            "# Would add {added} new connections ({} parsed, rest duplicates).",
            parsed.len()
        );
        return Ok(ExitCode::SUCCESS);
    }

    if let Value::Map(pairs) = &mut cfg {
        let seq = Value::Seq(merged);
        match pairs
            .iter_mut()
            .find(|(k, _)| matches!(k, Value::Str(s) if s == "connections"))
        {
            Some(slot) => slot.1 = seq,
            None => pairs.push((Value::Str("connections".to_owned()), seq)),
        }
    }
    for dir in [paths.session_path.parent(), paths.config_path.parent()]
        .into_iter()
        .flatten()
        .filter(|d| !d.as_os_str().is_empty())
    {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let vault_configured = vault::is_configured(cfg.get("vault").unwrap_or(&Value::Null));
    save(&paths, &cfg, vault_configured).map_err(|e| e.to_string())?;
    println!(
        "Wrote {}: added {added} connections ({} from Moba, duplicates skipped).",
        paths.session_path.display(),
        parsed.len()
    );
    Ok(ExitCode::SUCCESS)
}

fn main() -> ExitCode {
    match parse_args(std::env::args_os().skip(1)) {
        Parsed::Help => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Parsed::Error(m) => {
            eprintln!("{USAGE}\ntermigator-import-moba: error: {m}");
            ExitCode::from(2)
        }
        Parsed::Run(args) => run(args).unwrap_or_else(|m| {
            eprintln!("termigator-import-moba: {m}");
            ExitCode::FAILURE
        }),
    }
}
