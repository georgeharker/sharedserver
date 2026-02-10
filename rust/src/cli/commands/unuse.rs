use crate::output::{format_server_name, print_warning};
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
            super::decref::execute(name, Some(client_pid))
        }
        ServerState::Active => {
            // Normal case: decrement reference count
            super::decref::execute(name, Some(client_pid))
        }
    }
}
