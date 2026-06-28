use super::health::{process_liveness_checked, Liveness};
use super::lockfile::{read_clients_lock, read_server_lock, server_lock_exists, ServerLock};
use anyhow::Result;

/// Whether the lock's watcher process is alive, guarded against PID reuse via
/// its recorded start stamp. `false` if there is no recorded watcher.
pub fn watcher_alive(lock: &ServerLock) -> bool {
    match lock.watcher_pid {
        Some(wp) => process_liveness_checked(wp, lock.watcher_start_time) == Liveness::Alive,
        None => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerState {
    Stopped,
    Active,
    Grace,
    /// Server process has died but its lockfile still exists and the process
    /// has not yet been reaped (zombie). Transient: the watcher reaps the
    /// process and removes the lockfile, after which the state becomes Stopped.
    Defunct,
}

impl ServerState {
    pub fn as_str(&self) -> &'static str {
        match self {
            ServerState::Stopped => "stopped",
            ServerState::Active => "active",
            ServerState::Grace => "grace",
            ServerState::Defunct => "defunct",
        }
    }

    pub fn exit_code(&self) -> i32 {
        match self {
            ServerState::Active => 0,
            ServerState::Grace => 1,
            ServerState::Stopped => 2,
            ServerState::Defunct => 3,
        }
    }
}

/// Get current server state
pub fn get_server_state(name: &str) -> Result<ServerState> {
    if !server_lock_exists(name) {
        return Ok(ServerState::Stopped);
    }

    // Verify server process is actually alive. If the lock was deleted between
    // the existence check and here (normal teardown race) or is corrupt/empty,
    // treat the server as Stopped rather than surfacing a hard error to every
    // caller — doctor/start can then clean up any leftover file.
    let server_lock = match read_server_lock(name) {
        Ok(lock) => lock,
        Err(_) => return Ok(ServerState::Stopped),
    };

    // Identity-checked so a recycled PID (some unrelated process now owning the
    // old server's PID) reads as Gone rather than masquerading as the server.
    match process_liveness_checked(server_lock.pid, server_lock.start_time) {
        // Server is dead but lockfile exists - stale lock
        Liveness::Gone => Ok(ServerState::Stopped),
        // Server died but hasn't been reaped yet - lockfile cleanup pending
        Liveness::Zombie => Ok(ServerState::Defunct),
        Liveness::Alive => {
            // Active iff at least one client holds a reference. The clients
            // lockfile is kept for the whole life of the server (it is no longer
            // deleted when the refcount hits zero), so Grace is signalled by
            // refcount == 0, not by the file's absence. A missing/unreadable
            // clients lock is treated as zero references (Grace).
            let refcount = read_clients_lock(name).map(|c| c.refcount).unwrap_or(0);
            if refcount > 0 {
                Ok(ServerState::Active)
            } else {
                Ok(ServerState::Grace)
            }
        }
    }
}
