use anyhow::{bail, Context, Result};
use sharedserver::core::{
    clients_lock_exists, get_server_state, write_clients_lock, ClientInfo, ClientsLock, ServerState,
};

use crate::output::{format_refcount, format_server_name, print_success};

pub fn execute(name: &str, metadata: Option<String>, pid: Option<i32>) -> Result<()> {
    let state = get_server_state(name)?;

    match state {
        ServerState::Stopped => {
            bail!(
                "Server '{}' is not running. Start it first with 'serverctl start'",
                name
            );
        }
        ServerState::Active | ServerState::Grace => {
            // Increment refcount
            let client_pid = pid.unwrap_or_else(|| std::process::id() as i32);
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

    if !clients_lock_exists(name) {
        // Grace period: recreate clients.json
        let mut clients = ClientsLock::new();
        clients.refcount = 1;
        clients
            .clients
            .insert(client_pid, ClientInfo::new(metadata));

        write_clients_lock(name, &clients)?;
        return Ok(1);
    }

    // Active state: increment existing refcount
    sharedserver::core::lockfile::with_lock(&clients_path, |file| {
        let mut clients: ClientsLock = sharedserver::core::lockfile::read_json(file)?;
        clients.refcount += 1;
        clients
            .clients
            .insert(client_pid, ClientInfo::new(metadata));

        sharedserver::core::lockfile::write_json(file, &clients)?;
        Ok(clients.refcount)
    })
    .context("Failed to increment refcount")
}
