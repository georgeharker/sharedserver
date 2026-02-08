use anyhow::Result;
use colored::*;
use sharedserver_core::{get_server_state, read_server_lock, ServerState};

use crate::output::{format_pid, format_server_name};

pub fn execute(name: &str) -> Result<()> {
    let state = get_server_state(name)?;

    match state {
        ServerState::Active => {
            if let Ok(server_lock) = read_server_lock(name) {
                println!(
                    "{} {} is running (PID: {}, state: {})",
                    "✓".green().bold(),
                    format_server_name(name),
                    format_pid(server_lock.pid),
                    "active".green()
                );
            } else {
                println!(
                    "{} {} is {}",
                    "✓".green().bold(),
                    format_server_name(name),
                    "active".green()
                );
            }
        }
        ServerState::Grace => {
            if let Ok(server_lock) = read_server_lock(name) {
                println!(
                    "{} {} is in grace period (PID: {}, shutting down soon)",
                    "⚠".yellow().bold(),
                    format_server_name(name),
                    format_pid(server_lock.pid)
                );
            } else {
                println!(
                    "{} {} is in {}",
                    "⚠".yellow().bold(),
                    format_server_name(name),
                    "grace period".yellow()
                );
            }
        }
        ServerState::Stopped => {
            println!(
                "{} {} is {}",
                "✗".red().bold(),
                format_server_name(name),
                "not running".red()
            );
        }
    }

    std::process::exit(state.exit_code());
}
