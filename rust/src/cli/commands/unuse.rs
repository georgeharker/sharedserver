use crate::output::{format_refcount, format_server_name, print_warning};
use anyhow::{bail, Result};
use sharedserver::core::{get_server_state, ServerState};

/// Get the client PID: use provided PID, or default to parent process PID
fn get_client_pid(pid: Option<i32>) -> i32 {
    pid.unwrap_or_else(|| {
        // Default to parent process PID (the caller, e.g., Neovim)
        nix::unistd::getppid().as_raw()
    })
}

/// Detach from a server (decrement reference count)
///
/// This is a user-friendly wrapper around the 'admin decref' command.
/// It checks the server state and provides clear feedback about what's happening.
pub fn execute(name: &str, pid: Option<i32>) -> Result<()> {
    let client_pid = get_client_pid(pid);

    // Check current server state
    let state = get_server_state(name)?;

    match state {
        ServerState::Stopped => {
            bail!("Server {} is not running", format_server_name(name));
        }
        ServerState::Grace => {
            // Server is already in grace period, but we can still decref
            // This handles the case where a client might be trying to clean up
            print_warning(&format!(
                "Server {} is already in grace period, proceeding with detachment",
                format_server_name(name)
            ));
            super::decref::execute(name, client_pid)
        }
        ServerState::Active => {
            // Normal case: decrement reference count
            super::decref::execute(name, client_pid)
        }
        ServerState::Defunct => {
            // Server already died and is being torn down; nothing to detach from.
            bail!(
                "Server {} is shutting down (defunct, cleanup pending)",
                format_server_name(name)
            );
        }
    }
}

/// Detach from a server on behalf of EVERY client (`down --detach-all`): the
/// whole client map is cleared, so the refcount hits 0 and the server enters
/// its grace period. No signals are sent — the watcher's grace countdown owns
/// the actual shutdown, which makes this the gentle tier between a single
/// `unuse` and `admin stop`'s immediate teardown.
pub fn execute_all(name: &str) -> Result<()> {
    let state = get_server_state(name)?;

    match state {
        ServerState::Stopped => {
            bail!("Server {} is not running", format_server_name(name));
        }
        ServerState::Grace => {
            // Refcount is already 0 — nothing left to detach. Report success so
            // `down` counts the server as released (its grace countdown is
            // already running).
            print_warning(&format!(
                "Server {} is already in grace period (no clients attached)",
                format_server_name(name)
            ));
            Ok(())
        }
        ServerState::Active => {
            let detached = super::decref::clear_all_clients(name)?;

            let _ = sharedserver::core::log::log_invocation(
                name,
                &sharedserver::core::log::InvocationLog::success(
                    "detach-all",
                    &[name.to_string()],
                    Some(serde_json::json!({ "detached": detached })),
                ),
            );

            print_warning(&format!(
                "Detached all {detached} client(s) from server {} (refcount: {}, entering grace period)",
                format_server_name(name),
                format_refcount(0)
            ));
            Ok(())
        }
        ServerState::Defunct => {
            bail!(
                "Server {} is shutting down (defunct, cleanup pending)",
                format_server_name(name)
            );
        }
    }
}
