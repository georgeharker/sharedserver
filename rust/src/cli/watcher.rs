use anyhow::{Context, Result};
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use sharedserver::core::{
    clients_lock_exists, delete_clients_lock, delete_server_lock, is_process_alive, parse_duration,
    read_server_lock, ClientsLock,
};
use std::fs::OpenOptions;
use std::os::unix::io::AsRawFd;
use std::thread;
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_secs(5);

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
        // Check if server is alive
        if !is_process_alive(server_pid) {
            // Server died, clean up both lock files and exit
            let _ = delete_server_lock(name);
            let _ = delete_clients_lock(name);
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
                // Grace period expired, kill server
                let pid = Pid::from_raw(server_pid);

                // Try SIGTERM first
                let _ = kill(pid, Signal::SIGTERM);

                // Wait 5 seconds
                thread::sleep(Duration::from_secs(5));

                // Check if still alive
                if is_process_alive(server_pid) {
                    // Force kill with SIGKILL
                    let _ = kill(pid, Signal::SIGKILL);
                }

                // Clean up and exit
                let _ = delete_server_lock(name);
                break;
            }
        }

        // Sleep before next poll
        thread::sleep(POLL_INTERVAL);
    }

    Ok(())
}

/// Check clients.json and remove dead client PIDs
/// Returns true if there are still active clients, false otherwise
fn check_and_cleanup_dead_clients(name: &str) -> bool {
    use nix::fcntl::{flock, FlockArg};

    // Check if clients lockfile exists
    if !clients_lock_exists(name) {
        return false;
    }

    let clients_path = match sharedserver::core::lockfile::clients_lockfile_path(name) {
        Ok(p) => p,
        Err(_) => return false,
    };

    // Step 1: Read clients data and release lock immediately
    // This minimizes lock hold time and prevents blocking other operations
    let clients_snapshot = {
        let mut file = match OpenOptions::new()
            .read(true)
            .open(&clients_path)
        {
            Ok(f) => f,
            Err(_) => return false,
        };

        // Acquire shared lock for reading (allows concurrent reads)
        if flock(file.as_raw_fd(), FlockArg::LockShared).is_err() {
            return false;
        }

        // Read clients
        let clients: ClientsLock = match sharedserver::core::lockfile::read_json(&mut file) {
            Ok(c) => c,
            Err(_) => return false,
        };

        // Lock automatically released when file is dropped
        clients
    };

    // Step 2: Check which clients are alive WITHOUT holding any lock
    // This prevents blocking other operations during potentially slow is_process_alive() calls
    let mut dead_pids = Vec::new();
    for (pid, _client_info) in &clients_snapshot.clients {
        if !is_process_alive(*pid) {
            dead_pids.push(*pid);
        }
    }

    // If no dead clients, return early without acquiring write lock
    if dead_pids.is_empty() {
        return clients_snapshot.refcount > 0;
    }

    // Step 3: Re-acquire exclusive lock only for the update
    let mut file = match OpenOptions::new()
        .read(true)
        .write(true)
        .open(&clients_path)
    {
        Ok(f) => f,
        Err(_) => return false,
    };

    if flock(file.as_raw_fd(), FlockArg::LockExclusive).is_err() {
        return false;
    }

    // Re-read clients in case they changed between our read and write lock acquisition
    let mut clients: ClientsLock = match sharedserver::core::lockfile::read_json(&mut file) {
        Ok(c) => c,
        Err(_) => {
            drop(file);
            return false;
        }
    };

    // Remove dead clients that are still in the current client list
    let mut changed = false;
    for pid in dead_pids {
        if clients.clients.remove(&pid).is_some() {
            changed = true;
        }
    }

    // Recalculate refcount based on remaining clients
    if changed {
        clients.refcount = clients.clients.len() as u32;
    }

    // If no changes (dead clients were already removed by another process), just return
    if !changed {
        drop(file);
        return clients.refcount > 0;
    }

    // If refcount is now 0, delete the clients file to trigger grace period
    if clients.refcount == 0 {
        drop(file);
        let _ = delete_clients_lock(name);
        return false;
    }

    // Otherwise, update the clients file with cleaned-up data
    if sharedserver::core::lockfile::write_json(&mut file, &clients).is_err() {
        drop(file);
        return false;
    }

    drop(file);
    true
}
