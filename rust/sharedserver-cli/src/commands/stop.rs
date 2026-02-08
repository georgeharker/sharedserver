use anyhow::{bail, Context, Result};
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use sharedserver_core::{
    delete_clients_lock, delete_server_lock, get_server_state, read_server_lock, ServerState,
};
use std::thread;
use std::time::Duration;

use crate::output::{
    format_pid, format_server_name, print_error, print_info, print_success, print_warning,
};

pub fn execute(name: &str, force: bool) -> Result<()> {
    let state = get_server_state(name)?;

    if state == ServerState::Stopped {
        bail!("Server '{}' is not running", name);
    }

    let server = read_server_lock(name)?;
    let pid = Pid::from_raw(server.pid);

    print_info(&format!(
        "Stopping server {} (PID: {})...",
        format_server_name(name),
        format_pid(server.pid)
    ));

    // Try SIGTERM first
    kill(pid, Signal::SIGTERM).context("Failed to send SIGTERM")?;

    // Wait up to 5 seconds for graceful shutdown
    let mut attempts = 0;
    while attempts < 50 {
        if !sharedserver_core::is_process_alive(server.pid) {
            print_success(&format!(
                "Server {} stopped gracefully",
                format_server_name(name)
            ));
            cleanup(name)?;
            return Ok(());
        }

        thread::sleep(Duration::from_millis(100));
        attempts += 1;
    }

    if force {
        print_warning("Server did not stop gracefully, sending SIGKILL...");
        kill(pid, Signal::SIGKILL).context("Failed to send SIGKILL")?;

        thread::sleep(Duration::from_millis(500));

        if !sharedserver_core::is_process_alive(server.pid) {
            print_success(&format!(
                "Server {} forcefully terminated",
                format_server_name(name)
            ));
            cleanup(name)?;
            return Ok(());
        } else {
            print_error("Failed to kill server process");
            bail!("Failed to kill server process");
        }
    } else {
        print_error("Server did not stop within 5 seconds");
        bail!("Server did not stop within 5 seconds. Use --force to send SIGKILL");
    }
}

fn cleanup(name: &str) -> Result<()> {
    let _ = delete_clients_lock(name);
    let _ = delete_server_lock(name);

    let _ = sharedserver_core::log::log_invocation(
        name,
        &sharedserver_core::log::InvocationLog::success("stop", &[name.to_string()], None),
    );

    Ok(())
}
