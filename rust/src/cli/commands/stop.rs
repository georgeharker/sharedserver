use anyhow::{bail, Context, Result};
use nix::sys::signal::{kill, killpg, Signal};
use nix::unistd::Pid;
use sharedserver::core::{
    clients_lock_exists, delete_locks_owned_by, get_server_state, parse_duration,
    process_liveness_checked, read_server_lock, server_lock_exists, Liveness, ServerLock,
    ServerState,
};
use std::thread;
use std::time::{Duration, Instant};

use crate::output::{
    format_duration, format_pid, format_server_name, print_error, print_info, print_success,
    print_warning,
};

/// Stop a server.
///
/// `stop` is a *signaller*: it asks the server to exit, then waits for the
/// watcher to reap it and remove the lockfiles. It does not delete the
/// lockfiles itself — the watcher is the single owner of teardown, so we never
/// race it. The operation only succeeds once the server is gone, the watcher
/// has exited, and both lockfiles are gone (or it times out).
///
/// - without `--force`: SIGTERM only. If the server hasn't torn down within
///   `timeout`, it errors and leaves state intact (use `--force`).
/// - with `--force`: SIGTERM, then escalate to SIGKILL if `timeout` elapses,
///   then wait again. Errors with a diagnostic if it still can't converge —
///   at which point `admin kill` is the watcher-independent escape hatch.
pub fn execute(name: &str, force: bool, timeout: &str) -> Result<()> {
    let timeout =
        parse_duration(timeout).with_context(|| format!("Invalid timeout: {}", timeout))?;

    let state = get_server_state(name)?;
    if state == ServerState::Stopped {
        bail!("Server '{}' is not running", name);
    }

    let server = read_server_lock(name)?;
    let pid = Pid::from_raw(server.pid);

    print_info(&format!(
        "Stopping server {} (PID: {})...",
        format_server_name(name),
        format_pid(server.pid)
    ));

    // Ask the server to exit. It runs in its own process group, so signal the
    // whole group; fall back to a single-PID kill for servers started before
    // the setpgid change.
    if killpg(pid, Signal::SIGTERM).is_err() {
        kill(pid, Signal::SIGTERM).context("Failed to send SIGTERM")?;
    }

    if wait_for_teardown(name, &server, timeout) {
        print_success(&format!(
            "Server {} stopped gracefully",
            format_server_name(name)
        ));
        log_stop(name);
        return Ok(());
    }

    if !force {
        print_error(&format!(
            "Server {} did not stop within {}",
            format_server_name(name),
            format_duration(timeout)
        ));
        bail!(
            "Server '{}' did not stop within {}. Use --force to send SIGKILL",
            name,
            format_duration(timeout)
        );
    }

    // --force: escalate to SIGKILL and wait for the watcher to converge again.
    print_warning("Server did not stop gracefully, sending SIGKILL...");
    if killpg(pid, Signal::SIGKILL).is_err() {
        kill(pid, Signal::SIGKILL).context("Failed to send SIGKILL")?;
    }

    if wait_for_teardown(name, &server, timeout) {
        print_success(&format!(
            "Server {} forcefully terminated",
            format_server_name(name)
        ));
        log_stop(name);
        return Ok(());
    }

    let diagnostic = teardown_failure_diagnostic(name, &server);
    print_error(&diagnostic);
    bail!("{}", diagnostic);
}

/// Wait until the server has been fully torn down: the watcher has exited and
/// both lockfiles are gone. Returns `false` on timeout.
///
/// While a live watcher exists we leave cleanup entirely to it. If there is no
/// live watcher (it already exited, or was never recorded) and the server is
/// dead, we remove the lockfiles ourselves — pid-guarded so a restarted
/// instance is never touched — because nothing else will.
fn wait_for_teardown(name: &str, server: &ServerLock, timeout: Duration) -> bool {
    let start = Instant::now();
    loop {
        let watcher_alive = sharedserver::core::watcher_alive(server);

        if !watcher_alive
            && process_liveness_checked(server.pid, server.start_time) != Liveness::Alive
        {
            // No watcher to reap/clean and the server is dead: clean up the
            // orphaned lockfiles ourselves (guarded against a newer instance).
            delete_locks_owned_by(name, server.pid);
        }

        if !watcher_alive && !server_lock_exists(name) && !clients_lock_exists(name) {
            return true;
        }

        if start.elapsed() >= timeout {
            return false;
        }

        thread::sleep(Duration::from_millis(100));
    }
}

/// Build a precise message describing what is still alive after a failed
/// `--force` stop, so the user knows whether to reach for `admin kill`.
fn teardown_failure_diagnostic(name: &str, server: &ServerLock) -> String {
    let mut parts = Vec::new();

    match process_liveness_checked(server.pid, server.start_time) {
        Liveness::Alive => parts.push(format!("server process {} still alive", server.pid)),
        Liveness::Zombie => {
            parts.push(format!("server process {} is defunct (awaiting reap)", server.pid))
        }
        Liveness::Gone => {}
    }

    if sharedserver::core::watcher_alive(server) {
        if let Some(watcher_pid) = server.watcher_pid {
            parts.push(format!("watcher process {} still alive", watcher_pid));
        }
    }

    if server_lock_exists(name) {
        parts.push("server lockfile remains".to_string());
    }
    if clients_lock_exists(name) {
        parts.push("clients lockfile remains".to_string());
    }

    if parts.is_empty() {
        format!("Server '{}' did not tear down cleanly", name)
    } else {
        format!(
            "Failed to fully stop server '{}': {}. Run 'sharedserver admin kill {}' to force cleanup.",
            name,
            parts.join(", "),
            name
        )
    }
}

fn log_stop(name: &str) {
    let _ = sharedserver::core::log::log_invocation(
        name,
        &sharedserver::core::log::InvocationLog::success("stop", &[name.to_string()], None),
    );
}
