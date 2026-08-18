//! `up` — bring up every server in a profile, resolving each def from the
//! config (Tier 2: "resolve-and-run"). This fans out to the same start-or-attach
//! logic as `use`, so a profile is just a named set of `use` calls the binary
//! makes for you. Selection is deterministic (see `core::config::select`) so
//! `down` releases exactly what `up` brought up.
//!
//! With `--json` the command emits a single machine-readable object on stdout
//! (profile, per-server outcome, warnings) and stays silent otherwise — for
//! callers (the editor plugins) that want to health-check exactly what came up.

use anyhow::Result;
use serde::Serialize;
use std::path::{Path, PathBuf};

use sharedserver::core::{
    discover_config_path, get_server_state, load_config, read_server_lock, select, Selection,
    ServerSpec, ServerState,
};

use crate::output::{print_warning, set_quiet};

/// Resolve `--cwd` (config discovery root) to a concrete path, defaulting to the
/// process cwd. This governs the per-project config walk, NOT where servers run.
fn resolve_cwd(cwd: Option<&str>) -> PathBuf {
    cwd.map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

/// Shared config resolution for `up`/`down`: discover the file, load it, and
/// select the profile. `Ok(None)` means nothing is configured (a normal state).
/// Warnings are printed here so both verbs report typos/dangling members alike
/// (suppressed under `set_quiet`, i.e. in `--json` mode, where they instead ride
/// in the JSON via `Selection::warnings`).
pub(crate) fn resolve_selection(
    profile: &str,
    config: Option<&str>,
    cwd: Option<&str>,
    profile_optional: bool,
) -> Result<Option<Selection>> {
    let cwd = resolve_cwd(cwd);
    let path = match discover_config_path(config.map(Path::new), &cwd) {
        Some(p) => p,
        None => return Ok(None),
    };
    let cfg = load_config(&path)?;
    // `--profile-optional` (the plugin path) suppresses the "unknown profile"
    // warning: a host asking for its own, user-undefined profile is normal.
    let sel = select(&cfg, profile, !profile_optional);
    for w in &sel.warnings {
        print_warning(w);
    }
    Ok(Some(sel))
}

/// Should this server be skipped because another host already launched it?
/// Mirrors the `skipIfEnv` handling every client does.
fn skipped_by_env(spec: &ServerSpec) -> Option<&str> {
    let var = spec.skip_if_env.as_deref()?;
    match std::env::var(var) {
        Ok(v) if !v.is_empty() => Some(var),
        _ => None,
    }
}

/// One server's outcome, serialized in `--json` mode and counted for the summary.
#[derive(Serialize)]
struct ServerResult {
    name: String,
    /// "started" | "attached" | "skipped" | "failed"
    outcome: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pid: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

#[derive(Serialize)]
struct UpReport {
    profile: String,
    servers: Vec<ServerResult>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
}

fn bring_up_one(
    name: &str,
    spec: &ServerSpec,
    pid: Option<i32>,
    grace_default: &str,
) -> ServerResult {
    let result = |outcome, pid, reason| ServerResult {
        name: name.to_string(),
        outcome,
        pid,
        reason,
    };

    if let Some(var) = skipped_by_env(spec) {
        let reason = format!("{var} is set");
        print_warning(&format!("{name}: skipped ({reason})"));
        return result("skipped", None, Some(reason));
    }

    let grace = spec.grace_period.as_deref().unwrap_or(grace_default);
    let env_vars: Vec<String> = spec.env.iter().map(|(k, v)| format!("{k}={v}")).collect();
    let log_file = spec.log_file.as_deref();
    let metadata = spec.metadata.clone();

    // `lazy`: attach-only, never start. If the server isn't already up there is
    // nothing to attach to — skip rather than error, matching the shell hook.
    if spec.lazy {
        match get_server_state(name) {
            Ok(ServerState::Active) | Ok(ServerState::Grace) => {
                match super::r#use::execute(name, grace, metadata, pid, &env_vars, log_file, &[]) {
                    Ok(()) => result("attached", server_pid(name), None),
                    Err(e) => {
                        print_warning(&format!("{name}: {e}"));
                        result("failed", None, Some(e.to_string()))
                    }
                }
            }
            _ => {
                let reason = "lazy and not running".to_string();
                print_warning(&format!("{name}: {reason}; skipped"));
                result("skipped", None, Some(reason))
            }
        }
    } else {
        let Some(command) = spec.command.as_deref() else {
            let reason = "no command and not lazy".to_string();
            print_warning(&format!("{name}: {reason}; skipped"));
            return result("skipped", None, Some(reason));
        };
        let mut cmd_vec = Vec::with_capacity(1 + spec.args.len());
        cmd_vec.push(command.to_string());
        cmd_vec.extend(spec.args.iter().cloned());

        // Pre-state distinguishes a fresh start from an attach for the summary;
        // `use::execute` prints its own per-server line either way.
        let pre = get_server_state(name).unwrap_or(ServerState::Stopped);
        match super::r#use::execute(name, grace, metadata, pid, &env_vars, log_file, &cmd_vec) {
            Ok(()) => {
                let outcome = if matches!(pre, ServerState::Stopped) {
                    "started"
                } else {
                    "attached"
                };
                result(outcome, server_pid(name), None)
            }
            Err(e) => {
                print_warning(&format!("{name}: {e}"));
                result("failed", None, Some(e.to_string()))
            }
        }
    }
}

/// Best-effort read of a running server's PID for the report; `None` if the lock
/// isn't readable (e.g. a teardown race), which is non-fatal here.
fn server_pid(name: &str) -> Option<i32> {
    read_server_lock(name).ok().map(|l| l.pid)
}

/// Bring up a profile. Partial failure is tolerated: one server failing does not
/// abort the rest. Without `--json` the run ends with a one-line summary; with it
/// a single JSON object is printed and nothing else touches stdout.
pub fn execute(
    profile: &str,
    pid: Option<i32>,
    grace_default: &str,
    config: Option<&str>,
    cwd: Option<&str>,
    profile_optional: bool,
    json: bool,
) -> Result<()> {
    // In JSON mode, silence the human printers up front so sub-operations can't
    // leak onto the JSON stdout channel; selection warnings ride in the report.
    if json {
        set_quiet(true);
    }

    let Some(sel) = resolve_selection(profile, config, cwd, profile_optional)? else {
        if json {
            let report = UpReport {
                profile: profile.to_string(),
                servers: Vec::new(),
                warnings: vec!["no sharedserver config found".to_string()],
            };
            println!("{}", serde_json::to_string(&report)?);
        } else {
            print_warning("no sharedserver config found; nothing to bring up");
        }
        return Ok(());
    };

    let results: Vec<ServerResult> = sel
        .servers
        .iter()
        .map(|s| bring_up_one(&s.name, &s.spec, pid, grace_default))
        .collect();

    if json {
        let report = UpReport {
            profile: profile.to_string(),
            servers: results,
            warnings: sel.warnings.clone(),
        };
        println!("{}", serde_json::to_string(&report)?);
        return Ok(());
    }

    let (mut started, mut attached, mut skipped, mut failed) = (0u32, 0u32, 0u32, 0u32);
    for r in &results {
        match r.outcome {
            "started" => started += 1,
            "attached" => attached += 1,
            "skipped" => skipped += 1,
            _ => failed += 1,
        }
    }
    let mut parts = Vec::new();
    if started > 0 {
        parts.push(format!("started {started}"));
    }
    if attached > 0 {
        parts.push(format!("attached {attached}"));
    }
    if skipped > 0 {
        parts.push(format!("skipped {skipped}"));
    }
    if failed > 0 {
        parts.push(format!("failed {failed}"));
    }
    println!(
        "profile '{profile}': {}",
        if parts.is_empty() {
            "nothing to do".to_string()
        } else {
            parts.join(", ")
        }
    );
    Ok(())
}
