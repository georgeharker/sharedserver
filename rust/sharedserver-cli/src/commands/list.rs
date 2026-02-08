use anyhow::Result;
use colored::*;
use sharedserver_core::{get_server_state, read_clients_lock, read_server_lock};
use std::fs;

use crate::output::{
    format_clients, format_pid, format_refcount, format_server_name, format_server_state,
};

pub fn execute() -> Result<()> {
    let lockdir = sharedserver_core::lockfile::lockfile_dir()?;

    if !lockdir.exists() {
        println!("{}", "No servers found".dimmed());
        return Ok(());
    }

    let entries = fs::read_dir(&lockdir)?;

    let mut servers = Vec::new();

    for entry in entries {
        let entry = entry?;
        let path = entry.path();

        if let Some(filename) = path.file_name() {
            let filename = filename.to_string_lossy();

            if filename.ends_with(".server.json") {
                let name = filename
                    .strip_suffix(".server.json")
                    .unwrap_or(&filename)
                    .to_string();

                if let Ok(state) = get_server_state(&name) {
                    let server_info = if state != sharedserver_core::ServerState::Stopped {
                        read_server_lock(&name).ok()
                    } else {
                        None
                    };

                    servers.push((name, state, server_info));
                }
            }
        }
    }

    if servers.is_empty() {
        println!("{}", "No servers found".dimmed());
        return Ok(());
    }

    // Sort by name
    servers.sort_by(|a, b| a.0.cmp(&b.0));

    // Print header
    println!(
        "{:<20} {:<15} {:<10} {:<10} {}",
        "NAME".bold(),
        "STATE".bold(),
        "PID".bold(),
        "REFCOUNT".bold(),
        "CLIENTS".bold()
    );
    println!("{}", "─".repeat(80).dimmed());

    for (name, state, server_info) in servers {
        let pid_str = server_info
            .as_ref()
            .map(|s| format_pid(s.pid).to_string())
            .unwrap_or_else(|| "-".dimmed().to_string());

        // Read refcount and clients from ClientsLock if the server is active
        let (refcount, clients) = if state == sharedserver_core::ServerState::Active {
            if let Ok(clients_lock) = read_clients_lock(&name) {
                let client_list: Vec<String> =
                    clients_lock.clients.keys().map(|k| k.to_string()).collect();
                (clients_lock.refcount, client_list)
            } else {
                (0, vec![])
            }
        } else {
            (0, vec![])
        };

        println!(
            "{:<20} {:<24} {:<10} {:<10} {}",
            format_server_name(&name),
            format_server_state(&state),
            pid_str,
            format_refcount(refcount),
            format_clients(&clients, 3)
        );
    }

    Ok(())
}
