use anyhow::{Context, Result};
use nix::errno::Errno;
use nix::sys::signal::{kill, killpg, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::Pid;
use sharedserver::core::{
    delete_clients_lock, delete_locks_owned_by, delete_server_lock, is_process_alive,
    parse_duration, read_server_lock, ClientsLock,
};
use std::thread;
use std::time::{Duration, Instant};

/// How often the watcher polls liveness, clients, and the grace timer.
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// How long the watcher waits for the server to exit after SIGTERM (on grace
/// expiry) before escalating to SIGKILL.
const GRACE_KILL_TIMEOUT: Duration = Duration::from_secs(5);

/// Try to reap the server child without blocking.
///
/// The watcher is the server's parent, so it is the process responsible for
/// reaping it — otherwise the server lingers as a zombie. Returns `true` once
/// the server has exited (and been reaped here) or is no longer our child.
fn try_reap_server(server_pid: i32) -> bool {
    match waitpid(Pid::from_raw(server_pid), Some(WaitPidFlag::WNOHANG)) {
        Ok(WaitStatus::StillAlive) => false,
        Ok(WaitStatus::Exited(_, _)) | Ok(WaitStatus::Signaled(_, _, _)) => true,
        // Stopped/Continued (job control): still alive, not gone.
        Ok(_) => false,
        // No such child: already reaped, or never ours.
        Err(Errno::ECHILD) => true,
        // Unexpected error: fall back to a liveness probe.
        Err(_) => !is_process_alive(server_pid),
    }
}

/// Block (polling) until the server has exited and been reaped, or `timeout`
/// elapses. Returns `true` if it is gone.
fn wait_for_server_exit(server_pid: i32, timeout: Duration) -> bool {
    let start = Instant::now();
    loop {
        if try_reap_server(server_pid) {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        thread::sleep(Duration::from_millis(100));
    }
}


pub fn run_watcher(name: &str, grace_period: &str) -> Result<()> {
    let grace_duration = parse_duration(grace_period)
        .with_context(|| format!("Invalid grace period: {}", grace_period))?;

    // Try to read server lock, but if it fails (e.g., empty/corrupted), clean up and exit
    let server = match read_server_lock(name) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Watcher: Failed to read server lock ({}), cleaning up", e);
            let _ = delete_server_lock(name);
            let _ = delete_clients_lock(name);
            return Err(e.context("Failed to read server lock in watcher"));
        }
    };
    let server_pid = server.pid;

    let mut grace_timer: Option<Instant> = None;

    loop {
        // Reap the server if it has exited (we are its parent). This both
        // detects death and prevents it lingering as a zombie.
        if try_reap_server(server_pid) {
            // Server died, clean up both lock files and exit.
            delete_locks_owned_by(name, server_pid);
            break;
        }

        // Check and clean up dead clients
        let has_clients = check_and_cleanup_dead_clients(name);

        if has_clients {
            // Active state: cancel grace timer if it was set
            if grace_timer.is_some() {
                grace_timer = None;
            }
        } else if grace_timer.is_none() {
            // Grace state: start timer
            grace_timer = Some(Instant::now());
        } else if let Some(start_time) = grace_timer {
            // Check if grace period expired
            if start_time.elapsed() >= grace_duration {
                // Grace period expired, kill server process group.
                // The server runs in its own process group (setpgid) so
                // killpg takes down the entire tree (e.g. uv + python child).
                let pid = Pid::from_raw(server_pid);

                // Try SIGTERM on the whole process group first.
                // Fall back to single-PID kill for servers started before
                // the setpgid change.
                if killpg(pid, Signal::SIGTERM).is_err() {
                    let _ = kill(pid, Signal::SIGTERM);
                }

                // Wait for graceful exit, reaping the server if it goes.
                if !wait_for_server_exit(server_pid, GRACE_KILL_TIMEOUT) {
                    // Force kill the whole process group with SIGKILL.
                    if killpg(pid, Signal::SIGKILL).is_err() {
                        let _ = kill(pid, Signal::SIGKILL);
                    }
                    // Reap the SIGKILLed server so it doesn't linger as a zombie.
                    wait_for_server_exit(server_pid, GRACE_KILL_TIMEOUT);
                }

                // Clean up and exit
                delete_locks_owned_by(name, server_pid);
                break;
            }
        }

        // Sleep before next poll
        thread::sleep(POLL_INTERVAL);
    }

    Ok(())
}

/// Remove dead client PIDs from the clients lockfile and report whether any
/// live clients remain (`true` == still has references).
///
/// The clients lockfile is never deleted while the server lives: when the last
/// client leaves, the file simply holds an empty client map with refcount 0
/// (which signals grace). The whole read-modify-write happens under one
/// exclusive lock on a stable inode, so it can't race incref/decref. Liveness
/// probes are cheap (`/proc` reads), so holding the lock across them is fine.
fn check_and_cleanup_dead_clients(name: &str) -> bool {
    let clients_path = match sharedserver::core::lockfile::clients_lockfile_path(name) {
        Ok(p) => p,
        Err(_) => return false,
    };

    // No clients lockfile yet (e.g. the brief window during start) -> no clients.
    if !clients_path.exists() {
        return false;
    }

    sharedserver::core::lockfile::with_lock(&clients_path, |file| {
        let mut clients: ClientsLock =
            sharedserver::core::lockfile::read_json(file).unwrap_or_else(|_| ClientsLock::new());

        clients.clients.retain(|pid, _| is_process_alive(*pid));
        clients.refcount = clients.clients.len() as u32;

        sharedserver::core::lockfile::write_json(file, &clients)?;
        Ok(clients.refcount > 0)
    })
    .unwrap_or(false)
}
