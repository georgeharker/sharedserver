use anyhow::Result;
use colored::*;
use serde_json::json;
use sharedserver::core::{get_server_state, read_clients_lock, read_server_lock, ServerState};

use crate::output::{
    format_duration, format_pid, format_refcount, format_server_name, format_server_state,
    format_timestamp,
};

pub fn execute(name: &str, json_output: bool) -> Result<()> {
    let state = get_server_state(name)?;

    if state == ServerState::Stopped {
        if json_output {
            println!(
                "{}",
                json!({
                    "state": "stopped",
                    "name": name,
                })
            );
        } else {
            println!(
                "Server: {}\nStatus: {}",
                format_server_name(name),
                format_server_state(&state)
            );
        }
        return Ok(());
    }

    let server_lock = read_server_lock(name)?;

    let (refcount, clients_info) = if state == ServerState::Active {
        match read_clients_lock(name) {
            Ok(clients) => {
                let clients_info: Vec<_> = clients
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
                (clients.refcount, Some(clients_info))
            }
            Err(_) => (0, None),
        }
    } else {
        (0, None)
    };

    if json_output {
        let info = json!({
            "state": state.as_str(),
            "name": name,
            "pid": server_lock.pid,
            "command": server_lock.command,
            "grace_period": server_lock.grace_period,
            "watcher_pid": server_lock.watcher_pid,
            "started_at": server_lock.started_at.timestamp(),
            "refcount": refcount,
            "clients": clients_info,
        });

        println!("{}", serde_json::to_string_pretty(&info)?);
    } else {
        // Formatted output
        println!(
            "Server: {} (PID: {})",
            format_server_name(name),
            format_pid(server_lock.pid)
        );
        println!(
            "Status: {} (refcount: {})",
            format_server_state(&state),
            format_refcount(refcount)
        );
        println!("Command: {}", server_lock.command.join(" ").bright_white());

        // Parse grace period string and format duration
        if let Ok(grace_duration) = sharedserver::core::parse_duration(&server_lock.grace_period) {
            println!("Grace Period: {}", format_duration(grace_duration));
        } else {
            println!("Grace Period: {}", server_lock.grace_period);
        }

        // Convert chrono::DateTime to SystemTime for formatting
        let started_system_time = std::time::SystemTime::UNIX_EPOCH
            + std::time::Duration::from_secs(server_lock.started_at.timestamp() as u64);
        println!(
            "Started: {}",
            format_timestamp(started_system_time).dimmed()
        );

        if let Some(watcher_pid) = server_lock.watcher_pid {
            println!("Watcher: {}", format_pid(watcher_pid));
        }

        // Print clients
        if let Some(clients) = clients_info {
            println!("\n{}:", "Clients".bold());
            if clients.is_empty() {
                println!("  {}", "(none)".dimmed());
            } else {
                for client in clients {
                    let pid = client["pid"].as_i64().unwrap_or(0) as i32;
                    let metadata = client["metadata"]
                        .as_str()
                        .map(|m| format!(" ({})", m))
                        .unwrap_or_default();

                    if let Some(attached_at_str) = client["attached_at"].as_str() {
                        // Parse chrono DateTime from JSON string
                        if let Ok(attached_at) =
                            chrono::DateTime::parse_from_rfc3339(attached_at_str)
                        {
                            let attached_system_time = std::time::SystemTime::UNIX_EPOCH
                                + std::time::Duration::from_secs(attached_at.timestamp() as u64);
                            println!(
                                "  {} PID: {}{} - attached {}",
                                "•".cyan(),
                                format_pid(pid),
                                metadata,
                                format_timestamp(attached_system_time).dimmed()
                            );
                        } else {
                            println!("  {} PID: {}{}", "•".cyan(), format_pid(pid), metadata);
                        }
                    } else {
                        println!("  {} PID: {}{}", "•".cyan(), format_pid(pid), metadata);
                    }
                }
            }
        }
    }

    Ok(())
}
