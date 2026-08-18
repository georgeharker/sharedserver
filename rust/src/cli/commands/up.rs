//! `up` — bring up every server in a profile, resolving each def from the
//! config (Tier 2: "resolve-and-run"). This fans out to the same start-or-attach
//! logic as `use`, so a profile is just a named set of `use` calls the binary
//! makes for you. Selection is deterministic (see `core::config::select`) so
//! `down` releases exactly what `up` brought up.

use anyhow::Result;
use std::path::{Path, PathBuf};

use sharedserver::core::{
    discover_config_path, get_server_state, load_config, select, Selection, ServerSpec, ServerState,
};

use crate::output::print_warning;

/// Resolve `--cwd` (config discovery root) to a concrete path, defaulting to the
/// process cwd. This governs the per-project config walk, NOT where servers run.
fn resolve_cwd(cwd: Option<&str>) -> PathBuf {
    cwd.map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

/// Shared config resolution for `up`/`down`: discover the file, load it, and
/// select the profile. `Ok(None)` means nothing is configured (a normal state).
/// Warnings are printed here so both verbs report typos/dangling members alike.
pub(crate) fn resolve_selection(
    profile: &str,
    config: Option<&str>,
    cwd: Option<&str>,
) -> Result<Option<Selection>> {
    let cwd = resolve_cwd(cwd);
    let path = match discover_config_path(config.map(Path::new), &cwd) {
        Some(p) => p,
        None => return Ok(None),
    };
    let cfg = load_config(&path)?;
    let sel = select(&cfg, profile);
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

enum Outcome {
    Started,
    Attached,
    Skipped,
    Failed,
}

fn bring_up_one(name: &str, spec: &ServerSpec, pid: Option<i32>, grace_default: &str) -> Outcome {
    if let Some(var) = skipped_by_env(spec) {
        print_warning(&format!("{name}: skipped ({var} is set)"));
        return Outcome::Skipped;
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
                    Ok(()) => Outcome::Attached,
                    Err(e) => {
                        print_warning(&format!("{name}: {e}"));
                        Outcome::Failed
                    }
                }
            }
            _ => {
                print_warning(&format!("{name}: lazy and not running; skipped"));
                Outcome::Skipped
            }
        }
    } else {
        let Some(command) = spec.command.as_deref() else {
            print_warning(&format!("{name}: no command and not lazy; skipped"));
            return Outcome::Skipped;
        };
        let mut cmd_vec = Vec::with_capacity(1 + spec.args.len());
        cmd_vec.push(command.to_string());
        cmd_vec.extend(spec.args.iter().cloned());

        // Pre-state distinguishes a fresh start from an attach for the summary;
        // `use::execute` prints its own per-server line either way.
        let pre = get_server_state(name).unwrap_or(ServerState::Stopped);
        match super::r#use::execute(name, grace, metadata, pid, &env_vars, log_file, &cmd_vec) {
            Ok(()) => {
                if matches!(pre, ServerState::Stopped) {
                    Outcome::Started
                } else {
                    Outcome::Attached
                }
            }
            Err(e) => {
                print_warning(&format!("{name}: {e}"));
                Outcome::Failed
            }
        }
    }
}

/// Bring up a profile. Partial failure is tolerated: one server failing does not
/// abort the rest, and the run ends with a one-line summary.
pub fn execute(
    profile: &str,
    pid: Option<i32>,
    grace_default: &str,
    config: Option<&str>,
    cwd: Option<&str>,
) -> Result<()> {
    let Some(sel) = resolve_selection(profile, config, cwd)? else {
        print_warning("no sharedserver config found; nothing to bring up");
        return Ok(());
    };
    if sel.servers.is_empty() {
        print_warning(&format!("profile '{profile}': no servers to bring up"));
        return Ok(());
    }

    let (mut started, mut attached, mut skipped, mut failed) = (0u32, 0u32, 0u32, 0u32);
    for s in &sel.servers {
        match bring_up_one(&s.name, &s.spec, pid, grace_default) {
            Outcome::Started => started += 1,
            Outcome::Attached => attached += 1,
            Outcome::Skipped => skipped += 1,
            Outcome::Failed => failed += 1,
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
