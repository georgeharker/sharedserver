//! `config` — the self-define verbs. Thin CLI wrappers over `core::config_edit`
//! (which does the atomic JSON ops) plus read-side inspection. A plugin (or a
//! human) registers/unregisters scoped server defs and tags them into profiles;
//! `up`/`down` then resolve those defs.

use anyhow::{bail, Result};
use serde_json::{Map, Value};
use std::path::PathBuf;

use sharedserver::core::config_edit::RegisterOutcome;
use sharedserver::core::{config_edit, discover_config_path};

use crate::output::{print_success, print_warning};

/// The file writes target: explicit `--config`, else the discovered config, else
/// the global `~/.config/sharedserver/servers.json` (created on first write).
fn write_path(config: Option<&str>) -> PathBuf {
    if let Some(c) = config {
        return PathBuf::from(c);
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if let Some(p) = discover_config_path(None, &cwd) {
        return p;
    }
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home)
        .join(".config")
        .join("sharedserver")
        .join("servers.json")
}

/// The file reads target: explicit `--config`, else the discovered config.
fn read_path(config: Option<&str>) -> Option<PathBuf> {
    if let Some(c) = config {
        return Some(PathBuf::from(c));
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    discover_config_path(None, &cwd)
}

/// Build the server spec object from CLI flags (camelCase keys, matching the schema).
fn build_spec(
    command: &[String],
    grace_period: Option<&str>,
    env_vars: &[String],
    log_file: Option<&str>,
    metadata: Option<&str>,
    lazy: bool,
) -> Result<Value> {
    let mut spec = Map::new();
    if lazy {
        spec.insert("lazy".to_string(), Value::Bool(true));
    }
    if let Some((cmd, args)) = command.split_first() {
        spec.insert("command".to_string(), Value::String(cmd.clone()));
        if !args.is_empty() {
            spec.insert(
                "args".to_string(),
                Value::Array(args.iter().cloned().map(Value::String).collect()),
            );
        }
    } else if !lazy {
        bail!("register needs a command (`-- <cmd> [args...]`) unless --lazy");
    }
    if let Some(g) = grace_period {
        spec.insert("gracePeriod".to_string(), Value::String(g.to_string()));
    }
    if let Some(l) = log_file {
        spec.insert("logFile".to_string(), Value::String(l.to_string()));
    }
    if let Some(m) = metadata {
        spec.insert("metadata".to_string(), Value::String(m.to_string()));
    }
    if !env_vars.is_empty() {
        let mut env = Map::new();
        for kv in env_vars {
            let (k, v) = kv
                .split_once('=')
                .ok_or_else(|| anyhow::anyhow!("--env expects KEY=VALUE, got '{kv}'"))?;
            env.insert(k.to_string(), Value::String(v.to_string()));
        }
        spec.insert("env".to_string(), Value::Object(env));
    }
    Ok(Value::Object(spec))
}

#[allow(clippy::too_many_arguments)]
pub fn register(
    scope: &str,
    name: &str,
    command: &[String],
    grace_period: Option<&str>,
    env_vars: &[String],
    log_file: Option<&str>,
    metadata: Option<&str>,
    lazy: bool,
    profiles: &[String],
    if_absent: bool,
    config: Option<&str>,
) -> Result<()> {
    let path = write_path(config);
    let spec = build_spec(command, grace_period, env_vars, log_file, metadata, lazy)?;
    let outcome = config_edit::register(&path, scope, name, spec, profiles, if_absent)?;
    let where_ = path.display();
    match outcome {
        RegisterOutcome::Registered => {
            print_success(&format!("registered '{name}' (scope {scope}) in {where_}"))
        }
        RegisterOutcome::Overwritten => {
            print_success(&format!("updated '{name}' (scope {scope}) in {where_}"))
        }
        RegisterOutcome::Skipped => print_warning(&format!(
            "'{name}' already exists; left as-is (--if-absent)"
        )),
    }
    Ok(())
}

pub fn unregister(scope: &str, name: Option<&str>, config: Option<&str>) -> Result<()> {
    let path = write_path(config);
    let removed = config_edit::unregister(&path, scope, name)?;
    if removed.is_empty() {
        print_warning(&format!("nothing registered by scope '{scope}' to remove"));
    } else {
        print_success(&format!(
            "unregistered {} ({})",
            removed.join(", "),
            path.display()
        ));
    }
    Ok(())
}

pub fn lookup(name: &str, config: Option<&str>, json: bool) -> Result<()> {
    let Some(path) = read_path(config) else {
        if json {
            println!("null");
        } else {
            print_warning("no sharedserver config found");
        }
        return Ok(());
    };
    let hit = config_edit::lookup(&path, name)?;
    if json {
        println!("{}", serde_json::to_string(&lookup_json(&hit))?);
        return Ok(());
    }
    match hit {
        None => print_warning(&format!("'{name}' is not registered")),
        Some(h) => {
            let scope = h
                .spec
                .get("_scope")
                .and_then(|v| v.as_str())
                .unwrap_or("(hand-authored)");
            let profs = if h.profiles.is_empty() {
                "(none)".to_string()
            } else {
                h.profiles.join(", ")
            };
            print_success(&format!("'{name}' — scope {scope}, profiles: {profs}"));
            println!("{}", serde_json::to_string_pretty(&h.spec)?);
        }
    }
    Ok(())
}

fn lookup_json(hit: &Option<config_edit::LookupHit>) -> Value {
    match hit {
        None => Value::Null,
        Some(h) => serde_json::json!({ "name": h.name, "spec": h.spec, "profiles": h.profiles }),
    }
}

pub fn list(config: Option<&str>, json: bool) -> Result<()> {
    let Some(path) = read_path(config) else {
        if json {
            println!("{{\"servers\":[],\"profiles\":{{}}}}");
        } else {
            print_warning("no sharedserver config found");
        }
        return Ok(());
    };
    let root = config_edit::read_config_value(&path)?;
    if json {
        // Compact machine view: server names (+scope) and profile → members.
        let servers: Vec<Value> = root
            .get("servers")
            .and_then(|v| v.as_object())
            .map(|m| {
                m.iter()
                    .map(|(n, spec)| serde_json::json!({ "name": n, "scope": spec.get("_scope") }))
                    .collect()
            })
            .unwrap_or_default();
        let profiles = root
            .get("profiles")
            .cloned()
            .unwrap_or(Value::Object(Map::new()));
        println!(
            "{}",
            serde_json::to_string(
                &serde_json::json!({ "servers": servers, "profiles": profiles })
            )?
        );
        return Ok(());
    }
    let servers = root.get("servers").and_then(|v| v.as_object());
    match servers {
        None => print_warning("(no servers)"),
        Some(m) if m.is_empty() => print_warning("(no servers)"),
        Some(m) => {
            println!("servers:");
            for (n, spec) in m {
                let scope = spec.get("_scope").and_then(|v| v.as_str()).unwrap_or("-");
                println!("  {n}  (scope {scope})");
            }
        }
    }
    if let Some(profs) = root.get("profiles").and_then(|v| v.as_object()) {
        if !profs.is_empty() {
            println!("profiles:");
            for (p, list) in profs {
                let members: Vec<&str> = list
                    .as_array()
                    .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
                    .unwrap_or_default();
                println!("  {p}: {}", members.join(", "));
            }
        }
    }
    Ok(())
}

pub fn show(config: Option<&str>, json: bool) -> Result<()> {
    let Some(path) = read_path(config) else {
        if json {
            println!("{{}}");
        } else {
            print_warning("no sharedserver config found");
        }
        return Ok(());
    };
    let root = config_edit::read_config_value(&path)?;
    if json {
        println!("{}", serde_json::to_string(&root)?);
    } else {
        println!("{}", serde_json::to_string_pretty(&root)?);
    }
    Ok(())
}

pub fn validate(config: Option<&str>) -> Result<()> {
    let Some(path) = read_path(config) else {
        print_warning("no sharedserver config found");
        return Ok(());
    };
    let issues = config_edit::validate(&path)?;
    if issues.is_empty() {
        print_success(&format!("{}: no issues", path.display()));
    } else {
        for i in &issues {
            print_warning(i);
        }
        bail!("{} issue(s) found", issues.len());
    }
    Ok(())
}

pub fn profile_add(profile: &str, names: &[String], config: Option<&str>) -> Result<()> {
    let path = write_path(config);
    config_edit::profile_add(&path, profile, names)?;
    print_success(&format!(
        "added {} to profile '{profile}'",
        names.join(", ")
    ));
    Ok(())
}

pub fn profile_remove(profile: &str, names: &[String], config: Option<&str>) -> Result<()> {
    let path = write_path(config);
    config_edit::profile_remove(&path, profile, names)?;
    print_success(&format!(
        "removed {} from profile '{profile}'",
        names.join(", ")
    ));
    Ok(())
}
