//! `down` — release every server in a profile. It re-runs the SAME deterministic
//! selection `up` used and calls `unuse` for each, so a session releases exactly
//! what it brought up without tracking any state. `skipIfEnv` is honored
//! symmetrically: a server another host owns was never attached, so it is not
//! detached here either.

use anyhow::Result;

use sharedserver::core::ServerSpec;

use crate::output::print_warning;

/// Did we skip attaching this server (so we must not detach it)? Same rule as
/// `up`: another host owns it when `skipIfEnv` is set.
fn skipped_by_env(spec: &ServerSpec) -> Option<&str> {
    let var = spec.skip_if_env.as_deref()?;
    match std::env::var(var) {
        Ok(v) if !v.is_empty() => Some(var),
        _ => None,
    }
}

/// Release a profile. Errors from individual `unuse` calls (e.g. a server that
/// already went away — "not running") are benign here and don't abort the rest.
pub fn execute(
    profile: &str,
    pid: Option<i32>,
    config: Option<&str>,
    cwd: Option<&str>,
    profile_optional: bool,
) -> Result<()> {
    let Some(sel) = super::up::resolve_selection(profile, config, cwd, profile_optional)? else {
        print_warning("no sharedserver config found; nothing to release");
        return Ok(());
    };
    if sel.servers.is_empty() {
        print_warning(&format!("profile '{profile}': no servers to release"));
        return Ok(());
    }

    let (mut released, mut skipped) = (0u32, 0u32);
    for s in &sel.servers {
        if let Some(var) = skipped_by_env(&s.spec) {
            print_warning(&format!("{}: skipped ({var} is set)", s.name));
            skipped += 1;
            continue;
        }
        match super::unuse::execute(&s.name, pid) {
            Ok(()) => released += 1,
            Err(e) => {
                // Typically "not running" — the server already went away. Report
                // at a low key and keep going.
                print_warning(&format!("{}: {e}", s.name));
                skipped += 1;
            }
        }
    }

    println!("profile '{profile}': released {released}, skipped {skipped}");
    Ok(())
}
