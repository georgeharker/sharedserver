use anyhow::Result;
use colored::*;
use sharedserver::core::{
    clients_lock_exists, delete_clients_lock, delete_server_lock, get_server_state,
    is_process_alive, process_liveness_checked, read_clients_lock, read_server_lock,
    server_lock_exists, Liveness, ServerState,
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
                    }
                    Err(e) => print_error(&format!("    Failed to remove server lockfile: {}", e)),
                }
            }

            if has_clients_lock {
                match delete_clients_lock(name) {
                    Ok(_) => {
                        print_success("    Removed stale clients lockfile");
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

    // Check 2: Validate server process is actually alive.
    //
    // The watcher owns reaping the server and removing the lockfiles, so when
    // the process is dead (gone or zombie) we only clean up ourselves if there
    // is no live watcher to do it. Deleting locks out from under a live watcher
    // would race its cleanup and could clobber a freshly-restarted instance.
    let server_liveness = process_liveness_checked(server_lock.pid, server_lock.start_time);
    if server_liveness != Liveness::Alive {
        let watcher_alive = sharedserver::core::watcher_alive(&server_lock);
        let descr = match server_liveness {
            Liveness::Zombie => "has died (zombie, awaiting reap)",
            _ => "is not running",
        };

        issues_found += 1;

        if watcher_alive {
            // Defer to the watcher; it will reap and remove the lockfiles.
            print_warning(&format!(
                "  Server process {} {} — watcher is alive, cleanup pending",
                format_pid(server_lock.pid),
                descr
            ));
            println!(
                "    {}",
                "Note: the watcher will reap it and remove the lockfiles shortly".dimmed()
            );
        } else {
            // No live watcher to clean up: this state is genuinely stale.
            print_warning(&format!(
                "  Server process {} {} and no watcher is running, but lockfile exists",
                format_pid(server_lock.pid),
                descr
            ));

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
        if !sharedserver::core::watcher_alive(&server_lock) {
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
        let mut server_names = std::collections::BTreeSet::new();

        // Discover by EITHER lockfile, so an orphaned `<name>.clients.json` with
        // no matching `<name>.server.json` (e.g. from a partial teardown) is
        // still found and cleaned up rather than lingering invisibly.
        for entry in entries {
            let entry = entry?;
            let filename = entry.file_name();
            let filename = filename.to_string_lossy();

            if let Some(name) = filename
                .strip_suffix(".server.json")
                .or_else(|| filename.strip_suffix(".clients.json"))
            {
                server_names.insert(name.to_string());
            }
        }

        if server_names.is_empty() {
            println!("{}", "No servers found".dimmed());
            return Ok(());
        }

        // One bad server must not abort the whole sweep — doctor exists to clean
        // up messes, so keep going and report any per-server failure.
        for name in server_names {
            if let Err(e) = check_server(&name) {
                print_error(&format!("  Failed to check '{}': {:#}", name, e));
            }
        }

        println!("\n{}", "Health check complete".bold());
    }

    Ok(())
}
