//! Core benchmarks.
//!
//! Every mode prints one `BENCH …` line with min/median/max wall time
//! over the requested iterations and the process peak RSS.
//!
//! This example exists for measurements only; the application never calls it.

use std::env;
use std::error::Error;
use std::fs;
use std::hint::black_box;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use termigator_core::config::doc::Value;
use termigator_core::config::save::{self, merge_defaults};
use termigator_core::import::moba::{merge_moba_connections, parse_moba_bookmarks};
use termigator_core::paths::Paths;
use termigator_core::remote::listing::parse_ls_output;
use termigator_core::remote::sync::{self, Direction};
use termigator_core::vault;

type Fallible = Result<(), Box<dyn Error>>;

fn main() -> Fallible {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some(mode) = args.first().map(String::as_str) else {
        return Err("usage: bench_core <config-load|config-save|yaml-load|merge-defaults|merge-defaults-owned|vault-derive|vault-seal|vault-unseal|import-moba|moba-parse|moba-merge|listing|sync|hosttree|ssh-build> [args]".into());
    };
    match mode {
        "config-load" => config_load(fixture(&args)?, iters(&args, 2, 5))?,
        "config-save" => config_save(fixture(&args)?, iters(&args, 2, 5))?,
        "vault-derive" => vault_derive(iters(&args, 1, 5))?,
        "vault-seal" => vault_seal(iters(&args, 1, 2000))?,
        "vault-unseal" => vault_unseal(iters(&args, 1, 2000))?,
        "import-moba" => import_moba(fixture(&args)?, iters(&args, 2, 3))?,
        "moba-parse" => moba_parse(fixture(&args)?, iters(&args, 2, 3))?,
        "moba-merge" => moba_merge(fixture(&args)?, iters(&args, 2, 3))?,
        "yaml-load" => yaml_load(fixture(&args)?, iters(&args, 2, 5))?,
        "merge-defaults" => merge_defaults_bench(fixture(&args)?, iters(&args, 2, 5))?,
        "merge-defaults-owned" => merge_defaults_owned_bench(fixture(&args)?, iters(&args, 2, 5))?,
        "listing" => listing(fixture(&args)?, iters(&args, 2, 5))?,
        "sync" => sync_trees(fixture(&args)?, iters(&args, 2, 5))?,
        "hosttree" => hosttree(fixture(&args)?, iters(&args, 2, 5))?,
        "ssh-build" => ssh_build(fixture(&args)?, iters(&args, 2, 2))?,
        other => return Err(format!("unknown mode: {other}").into()),
    }
    Ok(())
}

/// The fixture path most modes take (an input file for the mode, e.g. a `config.yaml` or an `ls -l` listing).
fn fixture(args: &[String]) -> Result<&str, String> {
    args.get(1).map(String::as_str).ok_or_else(|| {
        format!(
            "{}: needs a fixture path (an input file for this mode)",
            args[0]
        )
    })
}

fn iters(args: &[String], at: usize, default: usize) -> usize {
    args.get(at)
        .filter(|s| !s.is_empty())
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn paths(dir: &str) -> Paths {
    let dir = PathBuf::from(dir);
    Paths {
        repo_root: dir.clone(),
        config_path: dir.join("config.yaml"),
        session_path: dir.join("session.yaml"),
        config_example_path: dir.join("config.example.yaml"),
        session_example_path: dir.join("session.example.yaml"),
        config_dir: dir,
    }
}

fn rss_mb() -> f64 {
    fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines().find_map(|l| {
                l.strip_prefix("VmHWM:")?
                    .trim()
                    .strip_suffix(" kB")?
                    .trim()
                    .parse::<f64>()
                    .ok()
            })
        })
        .map_or(f64::NAN, |kb| kb / 1024.0)
}

fn report(name: &str, runs: &[Duration], extra: &str) {
    let mut ms: Vec<f64> = runs.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
    ms.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = ms.get(ms.len() / 2).copied().unwrap_or(f64::NAN);
    println!(
        "BENCH name={name} iters={} min_ms={:.3} median_ms={:.3} max_ms={:.3} total_ms={:.3} \
         rss_mb={:.1} {extra}",
        ms.len(),
        ms.first().copied().unwrap_or(f64::NAN),
        median,
        ms.last().copied().unwrap_or(f64::NAN),
        ms.iter().sum::<f64>(),
        rss_mb()
    );
}

fn timed<T>(runs: &mut Vec<Duration>, f: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let out = f();
    runs.push(start.elapsed());
    black_box(out)
}

/// Set `window.width` in the config mapping (the value the save benchmark changes).
fn set_width(cfg: &mut Value, width: i64) {
    let Value::Map(pairs) = cfg else { return };
    for (k, v) in pairs {
        if matches!(k, Value::Str(s) if s == "window")
            && let Value::Map(window) = v
        {
            for (wk, wv) in &mut *window {
                if matches!(wk, Value::Str(s) if s == "width") {
                    *wv = Value::Int(width);
                    return;
                }
            }
            window.push((Value::Str("width".to_owned()), Value::Int(width)));
            return;
        }
    }
}

fn config_load(dir: &str, n: usize) -> Fallible {
    let p = paths(dir);
    let mut runs = Vec::with_capacity(n);
    for _ in 0..n {
        timed(&mut runs, || merge_defaults(&save::load(&p).expect("load")));
    }
    report("config-load", &runs, "unit=load+merge");
    Ok(())
}

fn config_save(dir: &str, n: usize) -> Fallible {
    let p = paths(dir);
    let mut cfg = merge_defaults(&save::load(&p).expect("load"));
    let mut runs = Vec::with_capacity(n);
    for i in 0..n {
        set_width(&mut cfg, 900 + i as i64);
        timed(&mut runs, || save::save(&p, &cfg, false).expect("save"));
    }
    report("config-save", &runs, "unit=save(width changed)");
    Ok(())
}

fn vault_params() -> Value {
    let (params, _) = vault::create_vault_with_salt(
        "bench-master",
        b"0123456789abcdef",
        b"abcdefghijkl",
        (1 << 15, 8, 1),
    )
    .expect("params");
    params
}

fn vault_derive(n: usize) -> Fallible {
    let params = vault_params();
    let mut runs = Vec::with_capacity(n);
    for _ in 0..n {
        timed(&mut runs, || {
            vault::derive_key("bench-master", &params).expect("derive")
        });
    }
    report("vault-derive", &runs, "unit=scrypt N=32768");
    Ok(())
}

fn vault_seal(n: usize) -> Fallible {
    let params = vault_params();
    let key = vault::derive_key("bench-master", &params).expect("key");
    let mut runs = Vec::with_capacity(n);
    for i in 0..n {
        timed(&mut runs, || {
            vault::seal(&key, &format!("secret-password-{i}")).expect("seal")
        });
    }
    report("vault-seal", &runs, "unit=seal one password");
    Ok(())
}

fn vault_unseal(n: usize) -> Fallible {
    let params = vault_params();
    let key = vault::derive_key("bench-master", &params).expect("key");
    let tokens: Vec<String> = (0..n)
        .map(|i| vault::seal(&key, &format!("secret-password-{i}")).expect("seal"))
        .collect();
    let mut runs = Vec::with_capacity(n);
    for token in &tokens {
        timed(&mut runs, || vault::unseal(&key, token).expect("unseal"));
    }
    report("vault-unseal", &runs, "unit=unseal one password");
    Ok(())
}

fn import_moba(file: &str, n: usize) -> Fallible {
    let text = fs::read_to_string(file)?;
    let mut runs = Vec::with_capacity(n);
    for _ in 0..n {
        timed(&mut runs, || {
            let rows = parse_moba_bookmarks(&text, "bench");
            let (merged, added) = merge_moba_connections(&[], &rows, "embedded");
            (merged.len(), added)
        });
    }
    report("import-moba", &runs, "unit=parse+merge");
    Ok(())
}

fn listing(file: &str, n: usize) -> Fallible {
    let text = fs::read(file)?;
    let mut runs = Vec::with_capacity(n);
    for _ in 0..n {
        timed(&mut runs, || parse_ls_output(&text).expect("parse"));
    }
    report("listing", &runs, "unit=parse ls -lAn");
    Ok(())
}

fn sync_trees(file: &str, n: usize) -> Fallible {
    let text = fs::read(file)?;
    let other = fs::read(format!("{file}.remote"))?;
    let mut runs = Vec::with_capacity(n);
    for _ in 0..n {
        timed(&mut runs, || {
            let (local, _) = sync::parse_remote_tree(&text);
            let (remote, rejected) = sync::parse_remote_tree(&other);
            let plan = sync::plan_sync(&local, &remote, Direction::Both);
            (local.len(), rejected.len(), plan.len())
        });
    }
    report("sync", &runs, "unit=parse two trees + plan");
    Ok(())
}

fn hosttree(dir: &str, n: usize) -> Fallible {
    use termigator_core::hosttree::{self, TreeState};
    let p = paths(dir);
    let cfg = save::load(&p).expect("load");
    let state = TreeState::default();
    let mut runs = Vec::with_capacity(n);
    for _ in 0..n {
        timed(&mut runs, || {
            hosttree::build(&cfg, &state, "", &()).rows.len()
        });
    }
    report("hosttree", &runs, "unit=build tree");
    Ok(())
}

fn ssh_build(dir: &str, n: usize) -> Fallible {
    use termigator_core::config::model::section;
    use termigator_core::proto::ssh;
    let p = paths(dir);
    let cfg = save::load(&p).expect("load");
    let launch = section(&cfg, "launch");
    let conns: Vec<Value> = match cfg.get("connections") {
        Some(Value::Seq(items)) => items.clone(),
        _ => Vec::new(),
    };
    let mut runs = Vec::with_capacity(n);
    for _ in 0..n {
        timed(&mut runs, || {
            conns
                .iter()
                .filter(|c| ssh::build_ssh_command(c, launch).is_ok())
                .count()
        });
    }
    report("ssh-build", &runs, "unit=build command for every entry");
    Ok(())
}

fn moba_parse(file: &str, n: usize) -> Fallible {
    let text = fs::read_to_string(file)?;
    let mut runs = Vec::with_capacity(n);
    for _ in 0..n {
        timed(&mut runs, || parse_moba_bookmarks(&text, "bench").len());
    }
    report("moba-parse", &runs, "unit=parse only");
    Ok(())
}

fn moba_merge(file: &str, n: usize) -> Fallible {
    let text = fs::read_to_string(file)?;
    let rows = parse_moba_bookmarks(&text, "bench");
    let mut runs = Vec::with_capacity(n);
    for _ in 0..n {
        timed(&mut runs, || {
            merge_moba_connections(&[], &rows, "embedded").1
        });
    }
    report("moba-merge", &runs, "unit=merge only");
    Ok(())
}

fn yaml_load(file: &str, n: usize) -> Fallible {
    let text = fs::read_to_string(file)?;
    let mut runs = Vec::with_capacity(n);
    for _ in 0..n {
        timed(&mut runs, || {
            termigator_core::config::doc::load(&text).expect("yaml")
        });
    }
    report("yaml-load", &runs, "unit=parse YAML only");
    Ok(())
}

fn merge_defaults_bench(dir: &str, n: usize) -> Fallible {
    let p = paths(dir);
    let cfg = save::load(&p).expect("load");
    let mut runs = Vec::with_capacity(n);
    for _ in 0..n {
        timed(&mut runs, || merge_defaults(&cfg));
    }
    report("merge-defaults", &runs, "unit=merge defaults only");
    Ok(())
}

fn merge_defaults_owned_bench(dir: &str, n: usize) -> Fallible {
    let p = paths(dir);
    let cfg = save::load(&p).expect("load");
    // Clones are prepared outside the timed region; each merge moves its lists.
    let owned: Vec<Value> = (0..n).map(|_| cfg.clone()).collect();
    let mut runs = Vec::with_capacity(n);
    for cfg in owned {
        timed(&mut runs, || save::merge_defaults_owned(cfg));
    }
    report("merge-defaults-owned", &runs, "unit=merge defaults, owned");
    Ok(())
}
