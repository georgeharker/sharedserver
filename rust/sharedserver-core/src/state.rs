use crate::health::is_process_alive;
use crate::lockfile::{clients_lock_exists, read_server_lock, server_lock_exists};
use anyhow::{Context, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerState {
    Stopped,
    Active,
    Grace,
}

impl ServerState {
    pub fn as_str(&self) -> &'static str {
        match self {
            ServerState::Stopped => "stopped",
            ServerState::Active => "active",
            ServerState::Grace => "grace",
        }
    }

    pub fn exit_code(&self) -> i32 {
        match self {
            ServerState::Active => 0,
            ServerState::Grace => 1,
            ServerState::Stopped => 2,
        }
    }
}

/// Get current server state
pub fn get_server_state(name: &str) -> Result<ServerState> {
    let has_server = server_lock_exists(name);
    let has_clients = clients_lock_exists(name);

    if !has_server {
        return Ok(ServerState::Stopped);
    }

    // Verify server process is actually alive
    let server_lock = read_server_lock(name).context("Failed to read server lock")?;

    if !is_process_alive(server_lock.pid) {
        // Server is dead but lockfile exists - stale lock
        return Ok(ServerState::Stopped);
    }

    if has_clients {
        Ok(ServerState::Active)
    } else {
        Ok(ServerState::Grace)
    }
}
