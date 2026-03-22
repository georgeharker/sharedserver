use anyhow::Result;
use colored::*;
use serde_json::json;
use sharedserver::core::{get_server_state, read_clients_lock, read_server_lock};
use std::fs;

use crate::output::{
    format_clients, format_pid, format_refcount, format_server_name, format_server_state,
};

pub fn execute(json_output: bool) -> Result<()> {
    let lockdir = sharedserver::core::lockfile::lockfile_dir()?;

    if !lockdir.exists() {
        if json_output {
            println!("[]");
        } else {
            println!("{}", "No servers found".dimmed());
        }
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
                    let server_info = if state != sharedserver::core::ServerState::Stopped {
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
        if json_output {
            println!("[]");
        } else {
            println!("{}", "No servers found".dimmed());
        }
        return Ok(());
    }

    // Sort by name
    servers.sort_by(|a, b| a.0.cmp(&b.0));

    if json_output {
        let items: Vec<_> = servers
            .iter()
            .map(|(name, state, server_info)| {
                let (refcount, clients_info) = if state == &sharedserver::core::ServerState::Active
                {
                    if let Ok(clients_lock) = read_clients_lock(name) {
                        let clients_info: Vec<_> = clients_lock
                            .clients
                            .iter()
                            .map(|(pid, info)| {
                                json!({
                                    "pid": pid,
                                    "attached_at": info.attached_at,
                                    "metadata": info.metadata,
                                })
                            })
                            .collect();
                        (clients_lock.refcount, Some(clients_info))
                    } else {
                        (0, None)
                    }
                } else {
                    (0, None)
                };

                if let Some(srv) = server_info {
                    json!({
                        "name": name,
                        "state": state.as_str(),
                        "pid": srv.pid,
                        "command": srv.command,
                        "grace_period": srv.grace_period,
                        "watcher_pid": srv.watcher_pid,
                        "started_at": srv.started_at.timestamp(),
                        "refcount": refcount,
                        "clients": clients_info,
                    })
                } else {
                    json!({
                        "name": name,
                        "state": state.as_str(),
                        "pid": null,
                        "refcount": 0,
                        "clients": null,
                    })
                }
            })
            .collect();

        println!("{}", serde_json::to_string_pretty(&items)?);
        return Ok(());
    }

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
        let (refcount, clients) = if state == sharedserver::core::ServerState::Active {
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
