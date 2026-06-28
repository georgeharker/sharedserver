use anyhow::{bail, Context, Result};
use sharedserver::core::{get_server_state, ClientInfo, ClientsLock, ServerState};

use crate::output::{format_refcount, format_server_name, print_success};

pub fn execute(name: &str, metadata: Option<String>, client_pid: i32) -> Result<()> {
    let state = get_server_state(name)?;

    match state {
        ServerState::Stopped => {
            bail!(
                "Server '{}' is not running. Start it first with 'sharedserver use' or 'sharedserver admin start'",
                name
            );
        }
        ServerState::Defunct => {
            bail!(
                "Server '{}' is shutting down (defunct, cleanup pending). Retry shortly.",
                name
            );
        }
        ServerState::Active | ServerState::Grace => {
            let new_refcount = increment_refcount(name, metadata, client_pid)?;

            // Log success
            let _ = sharedserver::core::log::log_invocation(
                name,
                &sharedserver::core::log::InvocationLog::success(
                    "incref",
                    &[name.to_string()],
                    Some(serde_json::json!({
                        "new_refcount": new_refcount,
                        "state": state.as_str(),
                    })),
                ),
            );

            print_success(&format!(
                "Attached to server {} (refcount: {})",
                format_server_name(name),
                format_refcount(new_refcount)
            ));
            Ok(())
        }
    }
}

fn increment_refcount(name: &str, metadata: Option<String>, client_pid: i32) -> Result<u32> {
    let clients_path = sharedserver::core::lockfile::clients_lockfile_path(name)?;

    // Read-modify-write the whole clients lock under a single exclusive lock.
    // The clients lockfile is created at server start and kept for the server's
    // whole life (never deleted on grace), so the inode is stable and this lock
    // provides real mutual exclusion. The refcount is *derived* from the number
    // of distinct client PIDs, so a repeat attach from the same PID is
    // idempotent: a HashMap insert that replaces an existing key must not bump
    // the count.
    sharedserver::core::lockfile::with_lock(&clients_path, |file| {
        let mut clients: ClientsLock =
            sharedserver::core::lockfile::read_json(file).unwrap_or_else(|_| ClientsLock::new());
        clients
            .clients
            .insert(client_pid, ClientInfo::new(metadata));
        clients.refcount = clients.clients.len() as u32;
        sharedserver::core::lockfile::write_json(file, &clients)?;
        Ok(clients.refcount)
    })
    .context("Failed to increment refcount")
}
