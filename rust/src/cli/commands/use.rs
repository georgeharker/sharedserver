use anyhow::{bail, Result};
use sharedserver::core::{
    get_server_state, read_clients_lock, read_server_lock, LaunchLock, ServerState,
};

use crate::output::{
    format_pid, format_refcount, format_server_name, print_success, print_warning,
};

/// Get the client PID: use provided PID, or default to parent process PID
fn get_client_pid(pid: Option<i32>) -> i32 {
    pid.unwrap_or_else(|| {
        // Default to parent process PID (the caller, e.g., Neovim)
        nix::unistd::getppid().as_raw()
    })
}

/// Use a server: start it if not running, then always increment refcount.
/// This is an atomic "start-or-attach" operation that combines start + incref.
pub fn execute(
    name: &str,
    grace_period: &str,
    metadata: Option<String>,
    pid: Option<i32>,
    env_vars: &[String],
    log_file: Option<&str>,
    command: &[String],
) -> Result<()> {
    // Determine the client PID (use provided or default to parent process)
    let client_pid = get_client_pid(pid);

    // Serialize the whole start-or-attach decision per server. Without this, two
    // concurrent `use` calls could both read Stopped and each start an instance
    // (and lose a reference). Held until we return: through the attach paths, or
    // (when starting) until the new instance has published its PIDs, so a
    // queued caller always sees a fully published Active server.
    let launch = LaunchLock::acquire(name)?;

    // Check current state
    let state = get_server_state(name)?;

    match state {
        ServerState::Stopped => {
            // Server not running - we need a command to start it
            if command.is_empty() {
                bail!(
                    "Server '{}' is not running and no command provided. \
                     Usage: sharedserver use [--grace-period DURATION] [--pid PID] <name> -- <command> [args...]",
                    name
                );
            }

            // Start the server atomically with this client as the initial client (refcount=1)
            // This avoids the refcount=0 window that would trigger immediate grace period
            let spec = super::start::StartSpec {
                name,
                grace_period,
                env_vars,
                command,
                log_file,
            };
            super::start::execute_with_client(launch, spec, client_pid, metadata.clone())?;

            // Read the server and clients info to get PID and refcount for output
            if let Ok(server_lock) = read_server_lock(name) {
                let refcount = read_clients_lock(name).map(|c| c.refcount).unwrap_or(1);
                print_success(&format!(
                    "Started server {} (PID: {}, refcount: {})",
                    format_server_name(name),
                    format_pid(server_lock.pid),
                    format_refcount(refcount)
                ));
            }

            Ok(())
        }
        ServerState::Active => {
            // Server exists - just increment refcount
            // Command is ignored in this case (server already running with its command)
            super::incref::execute(name, metadata, client_pid)?;

            // Read refcount after incref
            if let Ok(clients_lock) = read_clients_lock(name) {
                print_success(&format!(
                    "Attached to server {} (refcount: {})",
                    format_server_name(name),
                    format_refcount(clients_lock.refcount)
                ));
            }

            Ok(())
        }
        ServerState::Grace => {
            // Server in grace period - rescue it
            super::incref::execute(name, metadata, client_pid)?;

            // Read refcount after incref
            if let Ok(clients_lock) = read_clients_lock(name) {
                print_warning(&format!(
                    "Rescued server {} from grace period (refcount: {})",
                    format_server_name(name),
                    format_refcount(clients_lock.refcount)
                ));
            }

            Ok(())
        }
        ServerState::Defunct => {
            // Previous instance died and is still being torn down by its watcher.
            // Don't race the watcher's cleanup; ask the caller to retry.
            bail!(
                "Server '{}' is shutting down (defunct, cleanup pending). Retry shortly, \
                 or run 'sharedserver admin kill {}' if it is stuck.",
                name,
                name
            );
        }
    }
}
