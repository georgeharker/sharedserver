use anyhow::{bail, Context, Result};
use sharedserver::core::{get_server_state, ClientsLock, ServerState};

use crate::output::{format_refcount, format_server_name, print_success, print_warning};

pub fn execute(name: &str, client_pid: i32) -> Result<()> {
    let state = get_server_state(name)?;

    match state {
        ServerState::Stopped => {
            bail!("Server '{}' is not running", name);
        }
        ServerState::Active => {
            let new_refcount = decrement_refcount(name, client_pid)?;

            // Log success
            let _ = sharedserver::core::log::log_invocation(
                name,
                &sharedserver::core::log::InvocationLog::success(
                    "decref",
                    &[name.to_string()],
                    Some(serde_json::json!({
                        "new_refcount": new_refcount,
                        "client_pid": client_pid,
                    })),
                ),
            );

            if new_refcount == 0 {
                print_warning(&format!(
                    "Detached from server {} (refcount: {}, entering grace period)",
                    format_server_name(name),
                    format_refcount(new_refcount)
                ));
            } else {
                print_success(&format!(
                    "Detached from server {} (refcount: {})",
                    format_server_name(name),
                    format_refcount(new_refcount)
                ));
            }

            Ok(())
        }
        ServerState::Grace => {
            bail!("Server '{}' is in grace period (refcount already 0)", name);
        }
        ServerState::Defunct => {
            bail!(
                "Server '{}' is shutting down (defunct, cleanup pending)",
                name
            );
        }
    }
}

fn decrement_refcount(name: &str, client_pid: i32) -> Result<u32> {
    let clients_path = sharedserver::core::lockfile::clients_lockfile_path(name)?;

    // Read-modify-write under a single exclusive lock. The clients lockfile is
    // never deleted while the server lives (refcount 0 == grace, the file stays
    // with an empty client map), so the inode is stable and this lock gives real
    // mutual exclusion. The refcount is derived from the client map, so it can
    // never drift from the actual set of attached clients.
    sharedserver::core::lockfile::with_lock(&clients_path, |file| {
        let mut clients: ClientsLock =
            sharedserver::core::lockfile::read_json(file).unwrap_or_else(|_| ClientsLock::new());

        if clients.clients.remove(&client_pid).is_none() {
            bail!(
                "Client {} was not attached to server '{}'",
                client_pid,
                name
            );
        }

        clients.refcount = clients.clients.len() as u32;
        sharedserver::core::lockfile::write_json(file, &clients)?;
        Ok(clients.refcount)
    })
    .with_context(|| format!("Failed to decrement refcount for '{}'", name))
}
