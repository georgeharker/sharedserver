use anyhow::{bail, Result};
use nix::sys::signal::{kill, killpg, Signal};
use nix::unistd::Pid;
use sharedserver::core::{
    delete_locks_owned_by, get_server_state, process_liveness_checked, read_server_lock, Liveness,
    ServerState,
};
use std::thread;
use std::time::{Duration, Instant};

use crate::output::{format_pid, format_server_name, print_error, print_success, print_warning};

/// Forcibly kill a server and clean up its state.
///
/// `kill` is the *floor*: unlike `stop`, it never depends on the watcher, so it
/// works even when the watcher is wedged. It SIGKILLs the watcher first (so it
/// can't reap/clean concurrently or linger), then SIGKILLs the server's process
/// group, then removes the lockfiles itself — the only command that self-cleans.
/// With the watcher dead, the server zombie is reparented to init, which reaps it.
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

    // 1. Kill the watcher first so it can't race our lockfile cleanup or linger
    //    after we've removed them. kill is watcher-independent by design.
    if let Some(watcher_pid) = server.watcher_pid {
        // Identity-checked so we never SIGKILL an unrelated process that reused
        // the watcher's PID after it died.
        if sharedserver::core::watcher_alive(&server) {
            print_warning(&format!(
                "Killing watcher process {}...",
                format_pid(watcher_pid)
            ));
            match kill(Pid::from_raw(watcher_pid), Signal::SIGKILL) {
                Ok(_) => print_success("Watcher killed"),
                Err(e) => print_warning(&format!("Failed to kill watcher: {}", e)),
            }
        }
    }

    // 2. SIGKILL the server's whole process group (server + children like
    //    uv→python). Fall back to a single-PID kill if it isn't a group leader.
    match killpg(pid, Signal::SIGKILL) {
        Ok(_) => print_success("SIGKILL sent to process group"),
        Err(_) => match kill(pid, Signal::SIGKILL) {
            Ok(_) => print_success("SIGKILL sent"),
            Err(e) => {
                if process_liveness_checked(server.pid, server.start_time) == Liveness::Gone {
                    print_warning("Process already dead");
                } else {
                    print_error(&format!("Failed to send SIGKILL: {}", e));
                    bail!("Failed to send SIGKILL: {}", e);
                }
            }
        },
    }

    // 3. Confirm termination. With the watcher dead, init reaps the zombie;
    //    poll briefly for it to fully disappear.
    wait_until_not_alive(server.pid, server.start_time, Duration::from_secs(2));
    match process_liveness_checked(server.pid, server.start_time) {
        Liveness::Gone => print_success(&format!(
            "Server {} forcefully terminated",
            format_server_name(name)
        )),
        Liveness::Zombie => print_warning(&format!(
            "Server {} terminated (defunct, awaiting reap by init)",
            format_server_name(name)
        )),
        Liveness::Alive => print_error(&format!(
            "Server process {} may still be alive (SIGKILL not deliverable — \
             possibly stuck in uninterruptible sleep)",
            format_pid(server.pid)
        )),
    }

    // 4. Clean up lockfiles. kill is the only command that deletes them itself
    //    (the watcher it would otherwise rely on is now dead). Pid-guarded so a
    //    concurrently-restarted instance is never clobbered.
    print_warning("Cleaning up lockfiles...");
    delete_locks_owned_by(name, server.pid);
    print_success("Removed lockfiles");

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

/// Poll until the process is no longer alive (gone or zombie, identity-checked
/// against `stamp` to ignore a recycled PID), or `timeout` elapses.
fn wait_until_not_alive(pid: i32, stamp: Option<u64>, timeout: Duration) {
    let start = Instant::now();
    while process_liveness_checked(pid, stamp) == Liveness::Alive {
        if start.elapsed() >= timeout {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
}
