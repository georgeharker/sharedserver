use anyhow::{bail, Context, Result};
use sharedserver_core::{delete_clients_lock, get_server_state, ServerState};

use crate::output::{format_refcount, format_server_name, print_success, print_warning};

pub fn execute(name: &str, pid: Option<i32>) -> Result<()> {
    let state = get_server_state(name)?;

    match state {
        ServerState::Stopped => {
            bail!("Server '{}' is not running", name);
        }
        ServerState::Active => {
            let client_pid = pid.unwrap_or_else(|| std::process::id() as i32);
            let new_refcount = decrement_refcount(name, client_pid)?;

            // Log success
            let _ = sharedserver_core::log::log_invocation(
                name,
                &sharedserver_core::log::InvocationLog::success(
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
    }
}

fn decrement_refcount(name: &str, client_pid: i32) -> Result<u32> {
    use nix::fcntl::{flock, FlockArg};
    use std::fs::OpenOptions;
    use std::os::unix::io::AsRawFd;

    let clients_path = sharedserver_core::lockfile::clients_lockfile_path(name)?;

    // Open and lock the file manually so we can control when it's closed
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(&clients_path)
        .with_context(|| format!("Failed to open lockfile: {:?}", clients_path))?;

    // Acquire exclusive lock
    flock(file.as_raw_fd(), FlockArg::LockExclusive)
        .with_context(|| format!("Failed to acquire lock on: {:?}", clients_path))?;

    let mut clients: sharedserver_core::ClientsLock =
        sharedserver_core::lockfile::read_json(&mut file)?;

    // Remove client - only decrement refcount if client was actually in the map
    let client_existed = clients.clients.remove(&client_pid).is_some();

    if !client_existed {
        // Drop the file to release the lock before returning error
        drop(file);
        bail!(
            "Client {} was not attached to server '{}'",
            client_pid,
            name
        );
    }

    if clients.refcount > 0 {
        clients.refcount -= 1;
    }

    // Sanity check: refcount should equal number of clients
    if clients.refcount != clients.clients.len() as u32 {
        eprintln!(
            "WARNING: refcount mismatch in {}: refcount={}, clients.len()={}",
            name,
            clients.refcount,
            clients.clients.len()
        );
        // Fix the mismatch by using the actual client count
        clients.refcount = clients.clients.len() as u32;
    }

    if clients.refcount == 0 {
        // Drop the file to release the lock before deleting
        drop(file);
        delete_clients_lock(name)?;
        Ok(0)
    } else {
        // Update clients.json with new refcount
        sharedserver_core::lockfile::write_json(&mut file, &clients)?;
        Ok(clients.refcount)
    }
}
