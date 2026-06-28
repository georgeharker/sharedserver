use anyhow::{anyhow, bail, Context, Result};
use nix::sys::signal::{kill, killpg, Signal};
use nix::sys::wait::waitpid;
use nix::unistd::{fork, setpgid, setsid, ForkResult, Pid};
use sharedserver::core::{
    delete_clients_lock, delete_server_lock, get_server_state, is_process_alive, parse_duration,
    process_start_stamp, read_server_lock, server_lock_exists, write_clients_lock,
    write_server_lock, ClientInfo, ClientsLock, ServerLock, ServerState,
};
use std::collections::HashMap;

/// Start a server with no initial clients (refcount=0)
pub fn execute(
    name: &str,
    grace_period: &str,
    env_vars: &[String],
    command: &[String],
    log_file: Option<&str>,
) -> Result<()> {
    execute_internal(name, grace_period, env_vars, command, None, log_file)
}

/// Start a server with an initial client atomically (refcount=1)
/// This is used by the `use` command to avoid the refcount=0 window
pub fn execute_with_client(
    name: &str,
    grace_period: &str,
    env_vars: &[String],
    command: &[String],
    client_pid: i32,
    metadata: Option<String>,
    log_file: Option<&str>,
) -> Result<()> {
    execute_internal(
        name,
        grace_period,
        env_vars,
        command,
        Some((client_pid, metadata)),
        log_file,
    )
}

fn execute_internal(
    name: &str,
    grace_period: &str,
    env_vars: &[String],
    command: &[String],
    initial_client: Option<(i32, Option<String>)>,
    log_file: Option<&str>,
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
        ServerState::Defunct => {
            // Previous instance died but its watcher hasn't finished reaping and
            // removing the lockfiles yet. Don't race the watcher's cleanup.
            bail!(
                "Server '{}' is shutting down (defunct, cleanup pending). Retry shortly, \
                 or run 'sharedserver admin kill {}' if it is stuck.",
                name,
                name
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
        // Filled in by the watcher once it knows the real server PID.
        start_time: None,
        watcher_start_time: None,
    };

    write_server_lock(name, &server_lock).context("Failed to create server lockfile")?;

    // Always create the clients lockfile. It lives for the whole life of the
    // server and is the single mutual-exclusion point for refcount changes; it
    // is no longer deleted when the refcount hits zero (refcount 0 == grace).
    // `use` seeds it with one client (Active); a bare `admin start` seeds it
    // empty (refcount 0 == grace immediately, as before).
    let mut clients = ClientsLock::new();
    if let Some((client_pid, metadata)) = initial_client {
        clients
            .clients
            .insert(client_pid, ClientInfo::new(metadata));
    }
    clients.refcount = clients.clients.len() as u32;
    write_clients_lock(name, &clients).context("Failed to create clients lockfile")?;

    // Double fork strategy:
    // 1. First fork: Parent = sharedserver (returns), Child = watcher
    // 2. Second fork (in watcher): Parent = watcher (monitors), Child = server (execs)
    //
    // SAFETY: the children below call non-async-signal-safe code (allocation,
    // serde, file I/O) between fork and exec. That is sound ONLY because this
    // CLI is single-threaded — there is no async runtime or background thread,
    // so no other thread can hold a lock (e.g. the allocator's) across the fork.
    // If a background thread is ever introduced, this must move to a
    // post-fork-exec re-invocation or async-signal-safe-only code.
    match unsafe { fork() } {
        Ok(ForkResult::Child) => {
            // First child: become the watcher process
            setsid().context("Failed to create new session for watcher")?;

            // CRITICAL: Redirect watcher's stdout/stderr immediately to prevent blocking
            // on inherited pipes from parent process when writing errors/logs
            use std::fs::OpenOptions;
            use std::os::unix::io::IntoRawFd;
            if let Ok(devnull) = OpenOptions::new().write(true).open("/dev/null") {
                // into_raw_fd() takes ownership of the descriptor away from the
                // File so it isn't *also* closed when `devnull` drops. The
                // explicit libc::close below is then the single, correct close;
                // closing it twice aborts the process under std's debug-mode
                // I/O-safety guard (release silently tolerates the double close).
                let fd = devnull.into_raw_fd();
                unsafe {
                    libc::dup2(fd, 1); // stdout
                    libc::dup2(fd, 2); // stderr
                    libc::close(fd);
                }
            }

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
                    // Capture start stamps now so later liveness checks can
                    // detect PID reuse (see process_liveness_checked).
                    server_lock.start_time = process_start_stamp(server_child.as_raw());
                    server_lock.watcher_start_time = process_start_stamp(watcher_pid);

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

                    // Put the server in its own process group so we can kill the
                    // entire tree (including children like uv→python) with killpg().
                    // The watcher is in a separate session (setsid above) so it
                    // won't be affected.
                    let _ = setpgid(Pid::from_raw(0), Pid::from_raw(0));

                    // Redirect stdin to /dev/null (required for servers like workspace-mcp)
                    // stdout/stderr go to log_file if provided, otherwise /dev/null
                    use std::fs::OpenOptions;
                    use std::os::unix::io::IntoRawFd;

                    // stdin always goes to /dev/null. into_raw_fd() transfers
                    // ownership out of the File so the explicit libc::close is the
                    // only close — a double close aborts under std's debug-mode
                    // I/O-safety guard (release tolerates it).
                    if let Ok(devnull) = OpenOptions::new().read(true).open("/dev/null") {
                        let fd = devnull.into_raw_fd();
                        unsafe {
                            let flags = libc::fcntl(fd, libc::F_GETFD);
                            libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
                            libc::dup2(fd, 0); // stdin
                            libc::close(fd);
                        }
                    }

                    // stdout/stderr: log_file or /dev/null
                    if let Some(log_path) = log_file {
                        // Redirect to log file
                        if let Ok(logfile) =
                            OpenOptions::new().create(true).append(true).open(log_path)
                        {
                            let fd = logfile.into_raw_fd();
                            unsafe {
                                let flags = libc::fcntl(fd, libc::F_GETFD);
                                libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
                                libc::dup2(fd, 1); // stdout
                                libc::dup2(fd, 2); // stderr
                                libc::close(fd);
                            }
                        }
                    } else {
                        // Redirect to /dev/null
                        if let Ok(devnull) = OpenOptions::new().write(true).open("/dev/null") {
                            let fd = devnull.into_raw_fd();
                            unsafe {
                                let flags = libc::fcntl(fd, libc::F_GETFD);
                                libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
                                libc::dup2(fd, 1); // stdout
                                libc::dup2(fd, 2); // stderr
                                libc::close(fd);
                            }
                        }
                    }

                    // Exec into server command (never returns)
                    if let Err(e) = exec_server(command, env_vars) {
                        // Log error to server-specific log file if available
                        if let Some(error_log) = log_file {
                            if let Ok(mut log) = std::fs::OpenOptions::new()
                                .create(true)
                                .append(true)
                                .open(error_log)
                            {
                                use std::io::Write;
                                let _ = writeln!(
                                    log,
                                    "[{}] ERROR: Failed to exec server '{}': {:#}",
                                    chrono::Utc::now().format("%Y-%m-%d %H:%M:%S"),
                                    name,
                                    e
                                );
                            }
                        }
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
            // Original sharedserver process: wait briefly for watcher to set up,
            // then return to caller

            // Wait for the watcher to publish the real PIDs. "Published" means
            // it recorded its own PID and the server PID (written together) —
            // i.e. the process was *launched*, NOT that the server is ready or
            // listening. A slow-to-initialize server still publishes
            // immediately, so it never trips this.
            //
            // We distinguish "just slow" from "actually failed" by the watcher's
            // liveness, not a flat deadline: a live watcher will publish within
            // milliseconds of finishing its (possibly slow, fsync-bound) lock
            // write, so while it's alive we keep waiting up to a generous cap.
            // We only give up early when the watcher has *died* without
            // publishing — a real launch failure.
            let self_pid = std::process::id() as i32;
            let start = std::time::Instant::now();
            let hard_cap = std::time::Duration::from_secs(10);

            let mut published: Option<ServerLock> = None;
            loop {
                if let Ok(lock) = read_server_lock(name) {
                    if lock.watcher_pid.is_some() && lock.pid != self_pid {
                        published = Some(lock);
                        break;
                    }
                }
                // Watcher gone without publishing -> it never will. (A zombie
                // watcher reads as not-alive here, which is exactly right.)
                // Re-check the lock once in case it published as it exited.
                if !is_process_alive(watcher_child.as_raw()) {
                    if let Ok(lock) = read_server_lock(name) {
                        if lock.watcher_pid.is_some() && lock.pid != self_pid {
                            published = Some(lock);
                        }
                    }
                    break;
                }
                // Safety net: watcher alive but wedged far too long.
                if start.elapsed() > hard_cap {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }

            if let Some(lock) = published {
                let _ = sharedserver::core::log::log_invocation(
                    name,
                    &sharedserver::core::log::InvocationLog::success(
                        "start",
                        &[name.to_string()],
                        Some(serde_json::json!({
                            "server_pid": lock.pid,
                            "watcher_pid": watcher_child.as_raw(),
                            "command": command,
                            "grace_period": grace_period,
                        })),
                    ),
                );
                return Ok(());
            }

            // Genuine timeout. Tear down deterministically so we never leave an
            // orphaned watcher/server running (which would also let a retry
            // start a second instance): kill the watcher so it can't publish
            // late, kill any server it already forked, reap the watcher, and
            // remove the lockfiles. After this, `start` returning Err means the
            // state is fully clean.
            let _ = kill(watcher_child, Signal::SIGKILL);
            if let Ok(lock) = read_server_lock(name) {
                if lock.pid != self_pid {
                    let server_pid = Pid::from_raw(lock.pid);
                    if killpg(server_pid, Signal::SIGKILL).is_err() {
                        let _ = kill(server_pid, Signal::SIGKILL);
                    }
                }
            }
            let _ = waitpid(watcher_child, None);
            let _ = delete_server_lock(name);
            let _ = delete_clients_lock(name);
            bail!("Timeout waiting for server to start (cleaned up partial state)");
        }
        Err(e) => {
            // Fork failed, clean up
            let _ = delete_server_lock(name);
            let _ = delete_clients_lock(name);
            bail!("Failed to fork watcher: {}", e);
        }
    }
}

fn parse_env_vars(env_vars: &[String]) -> Result<HashMap<String, String>> {
    let mut map = HashMap::new();
    for env_str in env_vars {
        let parts: Vec<&str> = env_str.splitn(2, '=').collect();
        if parts.len() != 2 {
            bail!(
                "Invalid environment variable format: '{}'. Expected KEY=VALUE",
                env_str
            );
        }
        map.insert(parts[0].to_string(), parts[1].to_string());
    }
    Ok(map)
}

fn exec_server(command: &[String], env_vars: &[String]) -> Result<()> {
    if command.is_empty() {
        bail!("Server command cannot be empty");
    }

    // Parse environment variables
    let env_map = parse_env_vars(env_vars)?;

    // Use bash -c to execute the command (handles scripts and PATH lookup)
    let cmd_string = command.join(" ");

    use std::os::unix::process::CommandExt;
    let mut cmd = std::process::Command::new("/bin/bash");
    cmd.arg("-c");
    cmd.arg(&cmd_string);

    // Add custom environment variables on top of inherited ones
    if !env_map.is_empty() {
        cmd.envs(&env_map);
    }

    // exec replaces current process - this never returns on success
    let err = cmd.exec();

    // If we get here, exec failed
    Err(anyhow!("Failed to exec into server: {}", err))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_env_vars_valid() {
        let env_vars = vec![
            "KEY1=value1".to_string(),
            "PATH=/usr/bin".to_string(),
            "WORKSPACE_MCP_PORT=8002".to_string(),
        ];

        let result = parse_env_vars(&env_vars).unwrap();
        assert_eq!(result.get("KEY1"), Some(&"value1".to_string()));
        assert_eq!(result.get("PATH"), Some(&"/usr/bin".to_string()));
        assert_eq!(result.get("WORKSPACE_MCP_PORT"), Some(&"8002".to_string()));
    }

    #[test]
    fn test_parse_env_vars_with_equals_in_value() {
        let env_vars = vec!["URL=https://example.com?key=value".to_string()];

        let result = parse_env_vars(&env_vars).unwrap();
        assert_eq!(
            result.get("URL"),
            Some(&"https://example.com?key=value".to_string())
        );
    }

    #[test]
    fn test_parse_env_vars_empty_value() {
        let env_vars = vec!["EMPTY=".to_string()];

        let result = parse_env_vars(&env_vars).unwrap();
        assert_eq!(result.get("EMPTY"), Some(&"".to_string()));
    }

    #[test]
    fn test_parse_env_vars_invalid_no_equals() {
        let env_vars = vec!["INVALID_NO_EQUALS".to_string()];

        let result = parse_env_vars(&env_vars);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Invalid environment variable format"));
    }

    #[test]
    fn test_parse_env_vars_invalid_empty_key() {
        let env_vars = vec!["=value".to_string()];

        let result = parse_env_vars(&env_vars).unwrap();
        // Empty key is technically allowed by splitn, but should have empty string key
        assert_eq!(result.get(""), Some(&"value".to_string()));
    }
}
