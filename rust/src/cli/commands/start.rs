use anyhow::{bail, Context, Result};
use nix::unistd::{fork, setsid, ForkResult};
use sharedserver::core::{
    delete_clients_lock, delete_server_lock, get_server_state, is_process_alive, parse_duration,
    read_server_lock, server_lock_exists, write_clients_lock, write_server_lock, ClientInfo,
    ClientsLock, ServerLock, ServerState,
};
use std::ffi::CString;

/// Start a server with no initial clients (refcount=0)
pub fn execute(name: &str, grace_period: &str, command: &[String]) -> Result<()> {
    execute_internal(name, grace_period, command, None)
}

/// Start a server with an initial client atomically (refcount=1)
/// This is used by the `use` command to avoid the refcount=0 window
pub fn execute_with_client(
    name: &str,
    grace_period: &str,
    command: &[String],
    client_pid: i32,
    metadata: Option<String>,
) -> Result<()> {
    execute_internal(name, grace_period, command, Some((client_pid, metadata)))
}

fn execute_internal(
    name: &str,
    grace_period: &str,
    command: &[String],
    initial_client: Option<(i32, Option<String>)>,
) -> Result<()> {
    // Validate grace period
    let _grace_duration = parse_duration(grace_period)
        .with_context(|| format!("Invalid grace period: {}", grace_period))?;

    // Check current state
    let state = get_server_state(name)?;

    match state {
        ServerState::Active | ServerState::Grace => {
            let server = read_server_lock(name)?;
            bail!(
                "Server '{}' is already running (PID: {}, state: {})",
                name,
                server.pid,
                state.as_str()
            );
        }
        ServerState::Stopped => {
            // Clean up any stale locks
            if server_lock_exists(name) {
                let server = read_server_lock(name)?;
                if !is_process_alive(server.pid) {
                    eprintln!("Warning: Cleaning up stale lock for server '{}'", name);
                    let _ = delete_server_lock(name);
                    let _ = delete_clients_lock(name);
                }
            }
        }
    }

    // Create initial lockfiles (with placeholder PID)
    let server_lock = ServerLock {
        pid: std::process::id() as i32,
        command: command.to_vec(),
        grace_period: grace_period.to_string(),
        watcher_pid: None,
        started_at: chrono::Utc::now(),
    };

    write_server_lock(name, &server_lock).context("Failed to create server lockfile")?;

    // Create clients lockfile if starting with an initial client (atomic operation)
    // Otherwise, server starts with no clients (refcount=0)
    if let Some((client_pid, metadata)) = initial_client {
        let mut clients = ClientsLock::new();
        clients.refcount = 1;
        clients
            .clients
            .insert(client_pid, ClientInfo::new(metadata));
        write_clients_lock(name, &clients).context("Failed to create clients lockfile")?;
    }

    // Double fork strategy:
    // 1. First fork: Parent = serverctl (returns), Child = watcher
    // 2. Second fork (in watcher): Parent = watcher (monitors), Child = server (execs)

    match unsafe { fork() } {
        Ok(ForkResult::Child) => {
            // First child: become the watcher process
            setsid().context("Failed to create new session for watcher")?;
            let watcher_pid = std::process::id() as i32;

            // Fork again to create the actual server process
            match unsafe { fork() } {
                Ok(ForkResult::Parent {
                    child: server_child,
                }) => {
                    // Watcher process: update locks with real PIDs
                    let mut server_lock = match read_server_lock(name) {
                        Ok(lock) => lock,
                        Err(e) => {
                            eprintln!("Watcher: Failed to read server lock ({}), cleaning up", e);
                            let _ = delete_server_lock(name);
                            let _ = delete_clients_lock(name);
                            std::process::exit(1);
                        }
                    };
                    server_lock.pid = server_child.as_raw();
                    server_lock.watcher_pid = Some(watcher_pid);

                    if let Err(e) = write_server_lock(name, &server_lock) {
                        eprintln!("Watcher: Failed to update server lock ({}), cleaning up", e);
                        let _ = delete_server_lock(name);
                        let _ = delete_clients_lock(name);
                        std::process::exit(1);
                    }

                    // Run watcher (never returns unless server dies)
                    if let Err(e) = crate::watcher::run_watcher(name, grace_period) {
                        eprintln!("Watcher error: {:#}", e);
                        std::process::exit(1);
                    }

                    std::process::exit(0);
                }
                Ok(ForkResult::Child) => {
                    // Grandchild: become the actual server process
                    // Exec into server command (never returns)
                    if let Err(e) = exec_server(command) {
                        eprintln!("Failed to exec server: {:#}", e);
                        std::process::exit(1);
                    }
                    unreachable!("exec should never return");
                }
                Err(e) => {
                    eprintln!("Failed to fork server: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Ok(ForkResult::Parent {
            child: watcher_child,
        }) => {
            // Original serverctl process: wait briefly for watcher to set up,
            // then return to caller

            // Give watcher time to fork server and update locks
            // We need a more reliable way than just sleeping - poll for the lock update
            let start = std::time::Instant::now();
            let timeout = std::time::Duration::from_secs(2);

            loop {
                if let Ok(server_lock) = read_server_lock(name) {
                    // Check if watcher has updated the PIDs
                    if server_lock.watcher_pid.is_some()
                        && server_lock.pid != std::process::id() as i32
                    {
                        // Successfully started
                        let _ = sharedserver::core::log::log_invocation(
                            name,
                            &sharedserver::core::log::InvocationLog::success(
                                "start",
                                &[name.to_string()],
                                Some(serde_json::json!({
                                    "server_pid": server_lock.pid,
                                    "watcher_pid": watcher_child.as_raw(),
                                    "command": command,
                                    "grace_period": grace_period,
                                })),
                            ),
                        );
                        return Ok(());
                    }
                }

                if start.elapsed() > timeout {
                    // Clean up lock files before bailing
                    let _ = delete_server_lock(name);
                    let _ = delete_clients_lock(name);
                    bail!("Timeout waiting for server to start");
                }

                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
        Err(e) => {
            // Fork failed, clean up
            let _ = delete_server_lock(name);
            let _ = delete_clients_lock(name);
            bail!("Failed to fork watcher: {}", e);
        }
    }
}

fn exec_server(command: &[String]) -> Result<()> {
    if command.is_empty() {
        bail!("Server command cannot be empty");
    }

    let program = CString::new(command[0].as_str()).context("Invalid program name")?;

    let args: Result<Vec<CString>> = command
        .iter()
        .map(|s| CString::new(s.as_str()).context("Invalid argument"))
        .collect();
    let args = args?;

    // exec replaces current process
    nix::unistd::execvp(&program, &args).context("Failed to exec into server")?;

    unreachable!("exec never returns on success");
}
