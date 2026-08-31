//! `down` — release every server in a profile. It re-runs the SAME deterministic
//! selection `up` used and calls `unuse` for each, so a session releases exactly
//! what it brought up without tracking any state. `skipIfEnv` is honored
//! symmetrically: a server another host owns was never attached, so it is not
//! detached here either.
//!
//! `--detach-all` switches the per-server release from "this PID's refs" to
//! "everyone's refs": the whole client map is cleared, the refcount hits 0,
//! and each server enters its grace period. No signals — the gentle tier
//! between a normal `down` and `admin stop`.
//!
//! `--json` mirrors `up --json`: one machine-readable object on stdout, nothing
//! else.

use anyhow::Result;
use serde::Serialize;

use sharedserver::core::ServerSpec;

use crate::output::{print_warning, set_quiet};

/// Did we skip attaching this server (so we must not detach it)? Same rule as
/// `up`: another host owns it when `skipIfEnv` is set.
fn skipped_by_env(spec: &ServerSpec) -> Option<&str> {
    let var = spec.skip_if_env.as_deref()?;
    match std::env::var(var) {
        Ok(v) if !v.is_empty() => Some(var),
        _ => None,
    }
}

#[derive(Serialize)]
struct ServerResult {
    name: String,
    /// "released" | "skipped"
    outcome: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

#[derive(Serialize)]
struct DownReport {
    profile: String,
    servers: Vec<ServerResult>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
}

/// Release a profile. Errors from individual `unuse` calls (e.g. a server that
/// already went away — "not running") are benign here and don't abort the rest.
pub fn execute(
    profile: &str,
    pid: Option<i32>,
    detach_all: bool,
    config: Option<&str>,
    cwd: Option<&str>,
    profile_optional: bool,
    json: bool,
) -> Result<()> {
    if json {
        set_quiet(true);
    }

    let Some(sel) = super::up::resolve_selection(profile, config, cwd, profile_optional)? else {
        if json {
            let report = DownReport {
                profile: profile.to_string(),
                servers: Vec::new(),
                warnings: vec!["no sharedserver config found".to_string()],
            };
            println!("{}", serde_json::to_string(&report)?);
        } else {
            print_warning("no sharedserver config found; nothing to release");
        }
        return Ok(());
    };

    let mut results: Vec<ServerResult> = Vec::with_capacity(sel.servers.len());
    for s in &sel.servers {
        if let Some(var) = skipped_by_env(&s.spec) {
            let reason = format!("{var} is set");
            print_warning(&format!("{}: skipped ({reason})", s.name));
            results.push(ServerResult {
                name: s.name.clone(),
                outcome: "skipped",
                reason: Some(reason),
            });
            continue;
        }
        // `--detach-all` releases every client's refs (refcount -> 0, grace);
        // the default releases just this PID's refs (the inverse of `up`).
        let release = if detach_all {
            super::unuse::execute_all(&s.name)
        } else {
            super::unuse::execute(&s.name, pid)
        };
        match release {
            Ok(()) => results.push(ServerResult {
                name: s.name.clone(),
                outcome: "released",
                reason: None,
            }),
            Err(e) => {
                // Typically "not running" — the server already went away. Report
                // at a low key and keep going.
                print_warning(&format!("{}: {e}", s.name));
                results.push(ServerResult {
                    name: s.name.clone(),
                    outcome: "skipped",
                    reason: Some(e.to_string()),
                });
            }
        }
    }

    if json {
        let report = DownReport {
            profile: profile.to_string(),
            servers: results,
            warnings: sel.warnings.clone(),
        };
        println!("{}", serde_json::to_string(&report)?);
        return Ok(());
    }

    let released = results.iter().filter(|r| r.outcome == "released").count();
    let skipped = results.len() - released;
    println!("profile '{profile}': released {released}, skipped {skipped}");
    Ok(())
}
