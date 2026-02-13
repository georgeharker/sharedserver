use anyhow::Result;
use colored::*;
use sharedserver::core::{
    clients_lock_exists, delete_clients_lock, delete_server_lock, get_server_state,
    is_process_alive, read_clients_lock, read_server_lock, server_lock_exists, ServerState,
};
use std::fs;

use crate::output::{format_pid, format_server_name, print_error, print_success, print_warning};

/// Validate a single server's state and fix issues
fn check_server(name: &str) -> Result<()> {
    println!("\n{} {}...", "Checking".cyan(), format_server_name(name));

    let state = get_server_state(name)?;
    let mut issues_found = 0;
    let mut issues_fixed = 0;

    // Check 1: If server is stopped but lockfiles exist
    if state == ServerState::Stopped {
        let has_server_lock = server_lock_exists(name);
        let has_clients_lock = clients_lock_exists(name);

        if has_server_lock || has_clients_lock {
            issues_found += 1;
            print_warning(&format!(
                "  Server is stopped but lockfiles exist (server: {}, clients: {})",
                has_server_lock, has_clients_lock
            ));

            // Clean up lockfiles
            if has_server_lock {
                match delete_server_lock(name) {
                    Ok(_) => {
                        print_success("    Removed stale server lockfile");
                        issues_fixed += 1;
                    }
                    Err(e) => print_error(&format!("    Failed to remove server lockfile: {}", e)),
                }
            }

            if has_clients_lock {
                match delete_clients_lock(name) {
                    Ok(_) => {
                        print_success("    Removed stale clients lockfile");
                        issues_fixed += 1;
                    }
                    Err(e) => print_error(&format!("    Failed to remove clients lockfile: {}", e)),
                }
            }
        } else {
            println!(
                "  {} No lockfiles (expected for stopped server)",
                "✓".green()
            );
        }

        if issues_found == 0 {
            println!("  {} Server state is clean", "✓".green());
        }

        return Ok(());
    }

    // Server is running (Active or Grace) - perform deeper checks
    // Read both locks once at the start to minimize lock contention
    let server_lock = match read_server_lock(name) {
        Ok(lock) => lock,
        Err(e) => {
            print_error(&format!("  Failed to read server lock: {}", e));
            return Ok(());
        }
    };
    
    // Read clients lock early if server is running (avoid multiple reads later)
    let clients_lock_snapshot = if state == ServerState::Active {
        read_clients_lock(name).ok()
    } else {
        None
    };

    // Check 2: Validate server process is actually alive
    if !is_process_alive(server_lock.pid) {
        issues_found += 1;
        print_warning(&format!(
            "  Server process {} is not running but lockfile exists",
            format_pid(server_lock.pid)
        ));

        // Clean up stale lockfiles
        match delete_server_lock(name) {
            Ok(_) => {
                print_success("    Removed stale server lockfile");
                issues_fixed += 1;
            }
            Err(e) => print_error(&format!("    Failed to remove server lockfile: {}", e)),
        }

        match delete_clients_lock(name) {
            Ok(_) => {
                print_success("    Removed stale clients lockfile");
                issues_fixed += 1;
            }
            Err(e) => print_error(&format!("    Failed to remove clients lockfile: {}", e)),
        }
    } else {
        println!(
            "  {} Server process {} is alive",
            "✓".green(),
            format_pid(server_lock.pid)
        );
    }

    // Check 3: Validate watcher process if it exists
    if let Some(watcher_pid) = server_lock.watcher_pid {
        if !is_process_alive(watcher_pid) {
            issues_found += 1;
            print_warning(&format!(
                "  Watcher process {} is not running",
                format_pid(watcher_pid)
            ));
            // Note: We don't fix this - watcher may have exited normally
        } else {
            println!(
                "  {} Watcher process {} is alive",
                "✓".green(),
                format_pid(watcher_pid)
            );
        }
    }

    // Check 4: Validate clients if server is Active
    if state == ServerState::Active {
        if clients_lock_snapshot.is_none() {
            issues_found += 1;
            print_warning("  Server is Active but no clients lockfile exists");
        } else if let Some(clients_lock) = clients_lock_snapshot {
            let mut dead_clients = Vec::new();

            // Check each client PID
            for (pid, _info) in &clients_lock.clients {
                if !is_process_alive(*pid) {
                    dead_clients.push(*pid);
                }
            }

            if !dead_clients.is_empty() {
                issues_found += 1;
                print_warning(&format!(
                    "  Found {} dead client(s): {}",
                    dead_clients.len(),
                    dead_clients
                        .iter()
                        .map(|p| format_pid(*p).to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                println!(
                    "    {}",
                    "Note: Dead clients should be removed via 'admin decref' or will timeout naturally".dimmed()
                );
            } else if !clients_lock.clients.is_empty() {
                println!(
                    "  {} All {} client(s) are alive",
                    "✓".green(),
                    clients_lock.clients.len()
                );
            }

            // Validate refcount matches client count
            if clients_lock.refcount != clients_lock.clients.len() as u32 {
                issues_found += 1;
                print_warning(&format!(
                    "  Refcount mismatch: refcount={}, actual clients={}",
                    clients_lock.refcount,
                    clients_lock.clients.len()
                ));
            } else {
                println!(
                    "  {} Refcount ({}) matches client count",
                    "✓".green(),
                    clients_lock.refcount
                );
            }

            // Check if server is Active with no clients
            if clients_lock.refcount == 0 && clients_lock.clients.is_empty() {
                issues_found += 1;
                print_warning("  Server is Active but has no clients (should be in Grace)");
            }
        }
    }

    // Check 5: If server is in Grace, ensure no clients
    // Note: We already checked for clients.json existence earlier, so this should be rare
    if state == ServerState::Grace {
        // Grace state means clients.json shouldn't exist, but double-check
        if clients_lock_exists(name) {
            if let Ok(clients_lock) = read_clients_lock(name) {
                if clients_lock.refcount > 0 || !clients_lock.clients.is_empty() {
                    issues_found += 1;
                    print_warning(&format!(
                        "  Server in Grace period but has clients (refcount={}, clients={})",
                        clients_lock.refcount,
                        clients_lock.clients.len()
                    ));
                }
            }
        } else {
            println!("  {} No clients (expected for Grace state)", "✓".green());
        }
    }

    // Summary
    println!();
    if issues_found == 0 {
        println!("  {} No issues found", "✓".green().bold());
    } else if issues_fixed > 0 {
        println!(
            "  {} Found {} issue(s), fixed {}",
            "⚠".yellow().bold(),
            issues_found,
            issues_fixed
        );
    } else {
        println!("  {} Found {} issue(s)", "⚠".yellow().bold(), issues_found);
    }

    Ok(())
}

/// Execute doctor command for one or all servers
pub fn execute(server_name: Option<String>) -> Result<()> {
    if let Some(name) = server_name {
        // Check single server
        check_server(&name)?;
    } else {
        // Check all servers
        println!("{}", "Running health check on all servers...".bold());

        let lockdir = sharedserver::core::lockfile::lockfile_dir()?;

        if !lockdir.exists() {
            println!("{}", "No servers found".dimmed());
            return Ok(());
        }

        let entries = fs::read_dir(&lockdir)?;
        let mut server_names = Vec::new();

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
                    server_names.push(name);
                }
            }
        }

        if server_names.is_empty() {
            println!("{}", "No servers found".dimmed());
            return Ok(());
        }

        server_names.sort();

        for name in server_names {
            check_server(&name)?;
        }

        println!("\n{}", "Health check complete".bold());
    }

    Ok(())
}
