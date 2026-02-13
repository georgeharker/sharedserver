use anyhow::{bail, Result};
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use sharedserver::core::{
    delete_clients_lock, delete_server_lock, get_server_state, is_process_alive, read_server_lock,
    ServerState,
};
use std::thread;
use std::time::Duration;

use crate::output::{format_pid, format_server_name, print_error, print_success, print_warning};

/// Forcibly kill a server process and clean up state
pub fn execute(name: &str) -> Result<()> {
    let state = get_server_state(name)?;

    if state == ServerState::Stopped {
        bail!("Server '{}' is not running", name);
    }

    let server = read_server_lock(name)?;
    let pid = Pid::from_raw(server.pid);

    print_warning(&format!(
        "Force killing server {} (PID: {})...",
        format_server_name(name),
        format_pid(server.pid)
    ));

    // Send SIGKILL immediately (no grace period)
    match kill(pid, Signal::SIGKILL) {
        Ok(_) => {
            print_success("SIGKILL sent");
        }
        Err(e) => {
            // Check if process already dead
            if !is_process_alive(server.pid) {
                print_warning("Process already dead");
            } else {
                print_error(&format!("Failed to send SIGKILL: {}", e));
                bail!("Failed to send SIGKILL: {}", e);
            }
        }
    }

    // Wait briefly to confirm termination
    thread::sleep(Duration::from_millis(500));

    if !is_process_alive(server.pid) {
        print_success(&format!(
            "Server {} forcefully terminated",
            format_server_name(name)
        ));
    } else {
        print_error("Process may still be alive (zombie state?)");
    }

    // Also kill watcher if it exists
    if let Some(watcher_pid) = server.watcher_pid {
        if is_process_alive(watcher_pid) {
            print_warning(&format!(
                "Killing watcher process {}...",
                format_pid(watcher_pid)
            ));

            let watcher_pid_nix = Pid::from_raw(watcher_pid);
            match kill(watcher_pid_nix, Signal::SIGKILL) {
                Ok(_) => print_success("Watcher killed"),
                Err(e) => print_warning(&format!("Failed to kill watcher: {}", e)),
            }
        }
    }

    // Clean up all lockfiles
    print_warning("Cleaning up lockfiles...");

    if let Err(e) = delete_clients_lock(name) {
        print_warning(&format!("Failed to delete clients lockfile: {}", e));
    } else {
        print_success("Removed clients lockfile");
    }

    if let Err(e) = delete_server_lock(name) {
        print_warning(&format!("Failed to delete server lockfile: {}", e));
    } else {
        print_success("Removed server lockfile");
    }

    // Log the kill operation
    let _ = sharedserver::core::log::log_invocation(
        name,
        &sharedserver::core::log::InvocationLog::success("kill", &[name.to_string()], None),
    );

    print_success(&format!(
        "Server {} forcefully terminated and cleaned up",
        format_server_name(name)
    ));

    Ok(())
}
