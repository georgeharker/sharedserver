use serial_test::serial;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::Duration;

/// Dedicated, isolated lockfile directory for the integration tests.
///
/// Every spawned command is pointed here via SHAREDSERVER_LOCKDIR so the tests
/// never touch the user's real lockdir (XDG_RUNTIME_DIR/sharedserver or
/// /tmp/sharedserver), and so lock-existence assertions look in the same place
/// the binary actually writes.
fn test_lockdir() -> PathBuf {
    env::temp_dir().join("sharedserver-inttest")
}

/// Get the path to the sharedserver binary
fn get_binary_path() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("target");
    // Match the profile this test binary was built with, so the daemon we exec
    // is always the one `cargo test` / `cargo test --release` just rebuilt —
    // never a stale binary left over from the other profile.
    path.push(if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    });
    path.push("sharedserver");
    path
}

/// Get the path to test helper scripts
fn get_test_helper_path(script_name: &str) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push(".."); // Go up from rust/ to project root
    path.push("tests");
    path.push("test_helpers");
    path.push(script_name);
    path
}

/// Clean up lock files for a given server name
fn cleanup_lock_files(server_name: &str) {
    let temp_dir = test_lockdir();

    let server_lock = temp_dir.join(format!("{}.server.json", server_name));
    let clients_lock = temp_dir.join(format!("{}.clients.json", server_name));
    let invocations_log = temp_dir.join(format!("{}.invocations.log", server_name));

    let _ = fs::remove_file(server_lock);
    let _ = fs::remove_file(clients_lock);
    let _ = fs::remove_file(invocations_log);
}

/// Run a command with a timeout and return its output
fn run_command(args: &[&str]) -> std::process::Output {
    run_command_with_timeout(args, Duration::from_secs(30))
}

/// Run a command with a specified timeout
fn run_command_with_timeout(args: &[&str], timeout: Duration) -> std::process::Output {
    let binary = get_binary_path();
    let lockdir = test_lockdir();
    let _ = fs::create_dir_all(&lockdir);
    let child = Command::new(&binary)
        .args(args)
        .env("SHAREDSERVER_LOCKDIR", &lockdir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("Failed to spawn command");

    // Use a thread with timeout to wait for the child
    let child_id = child.id();
    let handle = thread::spawn(move || child.wait_with_output());

    let start = std::time::Instant::now();
    loop {
        if handle.is_finished() {
            return handle
                .join()
                .expect("Thread panicked")
                .expect("Failed to wait for child");
        }

        if start.elapsed() > timeout {
            // Timeout - kill the process
            eprintln!("Command timed out after {:?}: {:?}", timeout, args);
            unsafe {
                libc::kill(child_id as i32, libc::SIGKILL);
            }
            panic!(
                "Command timed out after {:?}: sharedserver {}",
                timeout,
                args.join(" ")
            );
        }

        thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn test_quick_death_cleanup() {
    // Regression test for: When a server dies immediately, both .server.json and .clients.json
    // lock files must be cleaned up by the watcher (watcher.rs:36-38)
    let server_name = "test_quick_death";

    cleanup_lock_files(server_name);

    // Start a server that exits immediately with error code
    let immediate_exit_script = get_test_helper_path("immediate_exit.sh");
    let _output = run_command(&[
        "admin",
        "start",
        server_name,
        "--",
        immediate_exit_script.to_str().unwrap(),
    ]);

    // Wait for the watcher to detect the death and clean up. The watcher polls
    // every 500ms and reaps the server, so a couple of seconds is plenty.
    thread::sleep(Duration::from_secs(3));

    // CRITICAL: Both lock files must be deleted
    let temp_dir = test_lockdir();
    let server_lock = temp_dir.join(format!("{}.server.json", server_name));
    let clients_lock = temp_dir.join(format!("{}.clients.json", server_name));

    assert!(
        !server_lock.exists(),
        "BUG: Server lock file not cleaned up after quick death"
    );
    assert!(
        !clients_lock.exists(),
        "BUG: Clients lock file not cleaned up after quick death (watcher.rs bug)"
    );

    cleanup_lock_files(server_name);
}

#[test]
fn test_environment_variables() {
    // REGRESSION TEST: Verifies that parse_env_vars() correctly parses environment variables.
    //
    // Coverage:
    // - Unit tests (start.rs:267-319): Thorough testing of parse_env_vars() function
    // - This integration test: Verifies CLI command doesn't error with --env flags
    // - Bash suite (test-monitoring-and-recovery.sh Category 5): End-to-end verification
    //   that env vars actually reach the server process

    let server_name = "test_env_vars";
    cleanup_lock_files(server_name);

    let echo_env_script = get_test_helper_path("echo_env.sh");

    // Start with env vars including one with equals in the value
    let output = run_command(&[
        "admin",
        "start",
        server_name,
        "--env",
        "TEST_VAR=hello_world",
        "--env",
        "ANOTHER_VAR=value_with=equals",
        "--",
        echo_env_script.to_str().unwrap(),
    ]);

    // The key test: start command should succeed (parse_env_vars() didn't error)
    assert!(
        output.status.success(),
        "Start command with env vars (including equals in value) should succeed"
    );

    // Cleanup
    thread::sleep(Duration::from_secs(1));
    run_command(&["admin", "stop", server_name]);
    cleanup_lock_files(server_name);
}

#[test]
#[serial]
fn test_server_lifecycle() {
    // Real lifecycle check: a long-running server starts, is visibly running,
    // and `stop` tears it down completely (server reaped, lockfiles removed)
    // before returning. This exercises the graceful converge-wait stop path.

    let server_name = "test_lifecycle";
    cleanup_lock_files(server_name);

    let long_running_script = get_test_helper_path("long_running.sh");
    let test_pid = std::process::id().to_string();

    // Start a persistent server with this test as the client (refcount=1).
    let output = run_command(&[
        "use",
        server_name,
        "--pid",
        &test_pid,
        "--grace-period",
        "30s",
        "--",
        long_running_script.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "Start command should work. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    thread::sleep(Duration::from_secs(2));

    // Precondition: the server is actually running.
    let temp_dir = test_lockdir();
    let server_lock = temp_dir.join(format!("{}.server.json", server_name));
    let clients_lock = temp_dir.join(format!("{}.clients.json", server_name));
    assert!(
        server_lock.exists(),
        "Server lock should exist while running"
    );

    // Stop should converge: it waits for full teardown before returning.
    let stop = run_command(&["admin", "stop", server_name, "--timeout", "8s"]);
    assert!(
        stop.status.success(),
        "Graceful stop should succeed. stderr: {}",
        String::from_utf8_lossy(&stop.stderr)
    );

    // Postcondition: both lockfiles are gone (the watcher cleaned up).
    assert!(
        !server_lock.exists(),
        "Stop must remove the server lockfile"
    );
    assert!(
        !clients_lock.exists(),
        "Stop must remove the clients lockfile"
    );

    cleanup_lock_files(server_name);
}

#[test]
#[serial]
fn test_admin_doctor_no_servers() {
    // Test that doctor command works when no servers are running
    let output = run_command(&["admin", "doctor"]);

    assert!(
        output.status.success(),
        "Doctor command should succeed even with no servers"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Running health check"),
        "Doctor should indicate it's running health check"
    );
}

#[test]
#[serial]
fn test_admin_doctor_healthy_server() {
    // Test doctor on a healthy running server
    let server_name = "test_doctor_healthy";
    cleanup_lock_files(server_name);

    let long_running_script = get_test_helper_path("long_running.sh");
    eprintln!("Script path: {}", long_running_script.display());
    eprintln!("Script exists: {}", long_running_script.exists());

    // Start a long-running server and register as client (atomic operation)
    let output = run_command(&[
        "use",
        server_name,
        "--grace-period",
        "30s",
        "--",
        long_running_script.to_str().unwrap(),
    ]);
    if !output.status.success() {
        eprintln!("Use stderr: {}", String::from_utf8_lossy(&output.stderr));
        eprintln!("Use stdout: {}", String::from_utf8_lossy(&output.stdout));
    }
    assert!(output.status.success(), "Server should be in use");

    // Wait for server to be fully started
    thread::sleep(Duration::from_secs(2));

    // Run doctor on this specific server
    let output = run_command(&["admin", "doctor", server_name]);

    assert!(
        output.status.success(),
        "Doctor should succeed on healthy server"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Checking") && stdout.contains(server_name),
        "Doctor should check the specified server"
    );
    assert!(
        stdout.contains("No issues found") || stdout.contains("✓"),
        "Healthy server should have no issues"
    );

    // Cleanup
    run_command(&["admin", "stop", server_name, "--force"]);
    thread::sleep(Duration::from_secs(1));
    cleanup_lock_files(server_name);
}

#[test]
#[serial]
fn test_admin_doctor_stale_lockfile() {
    // Test that doctor cleans up stale lockfiles
    let server_name = "test_doctor_stale";
    cleanup_lock_files(server_name);

    let long_running_script = get_test_helper_path("long_running.sh");

    // Start server and register as client (atomic operation) using test process PID
    let test_pid = std::process::id().to_string();
    let output = run_command(&[
        "use",
        server_name,
        "--pid",
        &test_pid,
        "--grace-period",
        "30s",
        "--",
        long_running_script.to_str().unwrap(),
    ]);
    if !output.status.success() {
        eprintln!("Use command failed");
        eprintln!("Use stderr: {}", String::from_utf8_lossy(&output.stderr));
        eprintln!("Use stdout: {}", String::from_utf8_lossy(&output.stdout));
    }
    assert!(output.status.success(), "Server should start");

    // Wait for server to be fully started
    thread::sleep(Duration::from_secs(2));

    // Get the PID from info command
    let info_output = run_command(&["info", server_name]);
    if !info_output.status.success() {
        eprintln!("Info command failed");
        eprintln!(
            "Info stderr: {}",
            String::from_utf8_lossy(&info_output.stderr)
        );
        eprintln!(
            "Info stdout: {}",
            String::from_utf8_lossy(&info_output.stdout)
        );
    }
    let info_str = String::from_utf8_lossy(&info_output.stdout);
    eprintln!("Info output:\n{}", info_str);

    // Read the server lockfile directly to get both the server pid and the
    // watcher pid. We must kill BOTH to create a genuinely-stale lockfile: the
    // watcher now reaps the server and removes the lockfiles itself, so if it
    // were left alive it (not doctor) would do the cleanup.
    let temp_dir = test_lockdir();
    let server_lock_path = temp_dir.join(format!("{}.server.json", server_name));
    let lock_json = fs::read_to_string(&server_lock_path).expect("server lock should exist");
    let extract = |key: &str| -> Option<i32> {
        lock_json
            .split(&format!("\"{}\"", key))
            .nth(1)?
            .split(',')
            .next()?
            .split('}')
            .next()?
            .trim_start_matches([':', ' '])
            .trim()
            .parse()
            .ok()
    };
    let pid = extract("pid").unwrap_or_else(|| {
        panic!("Should find pid in lock. Lock:\n{}", lock_json);
    });
    let watcher_pid = extract("watcher_pid");

    // Kill the watcher first (so it can't reap/clean), then the server.
    // This simulates a crash that takes out the whole apparatus, leaving a
    // stale lockfile with no live watcher — exactly what doctor must clean.
    #[cfg(unix)]
    {
        use std::process::Command as ProcessCommand;
        if let Some(wpid) = watcher_pid {
            let _ = ProcessCommand::new("kill")
                .args(["-9", &wpid.to_string()])
                .output();
        }
        ProcessCommand::new("kill")
            .args(["-9", &pid.to_string()])
            .output()
            .expect("Should be able to kill process");
    }

    // Wait a moment for the processes to die (and not be cleaned up, since the
    // watcher is now dead too).
    thread::sleep(Duration::from_secs(1));

    // Now run doctor - it should detect and clean up stale lockfile
    let output = run_command(&["admin", "doctor", server_name]);

    assert!(
        output.status.success(),
        "Doctor should succeed even with stale lockfile"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Removed stale") || stdout.contains("lockfiles exist"),
        "Doctor should detect and clean up stale lockfile"
    );

    // Verify lockfiles are actually cleaned up
    let temp_dir = test_lockdir();
    let server_lock = temp_dir.join(format!("{}.server.json", server_name));

    // After doctor runs, server lockfile should be cleaned up
    assert!(
        !server_lock.exists(),
        "Doctor should have removed stale server lockfile"
    );

    cleanup_lock_files(server_name);
}

#[test]
fn test_admin_kill_command() {
    // Test the kill command forcefully terminates a server
    let server_name = "test_kill";
    cleanup_lock_files(server_name);

    let long_running_script = get_test_helper_path("long_running.sh");
    eprintln!(
        "Starting server with script: {}",
        long_running_script.display()
    );

    // Start server and register as client (atomic operation) using test process PID
    let test_pid = std::process::id().to_string();
    let output = run_command(&[
        "use",
        server_name,
        "--pid",
        &test_pid,
        "--grace-period",
        "30s",
        "--",
        long_running_script.to_str().unwrap(),
    ]);
    if !output.status.success() {
        eprintln!("Use stderr: {}", String::from_utf8_lossy(&output.stderr));
        eprintln!("Use stdout: {}", String::from_utf8_lossy(&output.stdout));
    }
    assert!(output.status.success(), "Server should start");

    // Wait for server to be fully started
    thread::sleep(Duration::from_secs(2));

    // Check server state
    let info_output = run_command(&["info", server_name]);
    eprintln!(
        "Server info after start:\n{}",
        String::from_utf8_lossy(&info_output.stdout)
    );
    eprintln!(
        "Server info stderr:\n{}",
        String::from_utf8_lossy(&info_output.stderr)
    );

    // Verify server is running
    let list_output = run_command(&["list"]);
    let list_str = String::from_utf8_lossy(&list_output.stdout);
    assert!(
        list_str.contains(server_name),
        "Server should appear in list"
    );

    // Kill the server
    let kill_output = run_command(&["admin", "kill", server_name]);

    assert!(kill_output.status.success(), "Kill command should succeed");

    let kill_str = String::from_utf8_lossy(&kill_output.stdout);
    assert!(
        kill_str.contains("Force killing") || kill_str.contains("SIGKILL"),
        "Kill output should mention force killing"
    );
    assert!(
        kill_str.contains("forcefully terminated"),
        "Kill output should confirm termination"
    );

    // Wait a moment for cleanup
    thread::sleep(Duration::from_secs(1));

    // Verify server is no longer in list
    let list_output = run_command(&["list"]);
    let list_str = String::from_utf8_lossy(&list_output.stdout);
    assert!(
        !list_str.contains(server_name) || list_str.contains("No servers"),
        "Server should be removed from list after kill"
    );

    // Verify lockfiles are cleaned up
    let temp_dir = test_lockdir();
    let server_lock = temp_dir.join(format!("{}.server.json", server_name));
    let clients_lock = temp_dir.join(format!("{}.clients.json", server_name));

    assert!(!server_lock.exists(), "Kill should remove server lockfile");
    assert!(
        !clients_lock.exists(),
        "Kill should remove clients lockfile"
    );

    cleanup_lock_files(server_name);
}

#[test]
#[serial]
fn test_stop_force_escalation() {
    // A server that ignores SIGTERM: plain `stop` must time out (and leave state
    // intact, telling the user to use --force), while `stop --force` escalates to
    // SIGKILL and tears everything down.
    let server_name = "test_stop_force";
    cleanup_lock_files(server_name);

    let ignore_sigterm_script = get_test_helper_path("ignore_sigterm.sh");
    let test_pid = std::process::id().to_string();

    let output = run_command(&[
        "use",
        server_name,
        "--pid",
        &test_pid,
        "--grace-period",
        "30s",
        "--",
        ignore_sigterm_script.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "Server should start. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    thread::sleep(Duration::from_secs(2));

    // Precondition: it's really running.
    let temp_dir = test_lockdir();
    let server_lock = temp_dir.join(format!("{}.server.json", server_name));
    assert!(server_lock.exists(), "Server lock should exist before stop");

    // Plain stop must FAIL (SIGTERM ignored) and not escalate.
    let stop = run_command(&["admin", "stop", server_name, "--timeout", "1s"]);
    assert!(
        !stop.status.success(),
        "Plain stop should fail when SIGTERM is ignored"
    );
    let stop_err = String::from_utf8_lossy(&stop.stderr);
    assert!(
        stop_err.contains("did not stop") && stop_err.contains("--force"),
        "Plain stop should tell the user to use --force. stderr: {}",
        stop_err
    );
    // State must be intact: server still running, lock still present.
    assert!(
        server_lock.exists(),
        "Plain stop must leave the server lockfile intact on timeout"
    );

    // Forced stop must succeed: escalates to SIGKILL, then converges.
    let force = run_command(&["admin", "stop", server_name, "--force", "--timeout", "5s"]);
    assert!(
        force.status.success(),
        "Forced stop should succeed. stderr: {}",
        String::from_utf8_lossy(&force.stderr)
    );

    // Fully torn down: both lockfiles gone.
    assert!(
        !server_lock.exists(),
        "Forced stop must remove the server lockfile"
    );
    let clients_lock = temp_dir.join(format!("{}.clients.json", server_name));
    assert!(
        !clients_lock.exists(),
        "Forced stop must remove the clients lockfile"
    );

    cleanup_lock_files(server_name);
}

#[test]
#[serial]
fn test_restart_after_force_stop() {
    // Restart safety: `stop --force` must fully tear down (server gone, watcher
    // gone, lockfiles removed) before returning, so an immediate re-start with
    // the same name succeeds and is not clobbered by the old watcher.
    let server_name = "test_restart";
    cleanup_lock_files(server_name);

    let long_running_script = get_test_helper_path("long_running.sh");
    let test_pid = std::process::id().to_string();

    // Start instance #1
    let output = run_command(&[
        "use",
        server_name,
        "--pid",
        &test_pid,
        "--grace-period",
        "30s",
        "--",
        long_running_script.to_str().unwrap(),
    ]);
    assert!(output.status.success(), "First start should succeed");
    thread::sleep(Duration::from_secs(2));

    // Force-stop instance #1. This must converge before returning.
    let stop = run_command(&["admin", "stop", server_name, "--force", "--timeout", "8s"]);
    assert!(
        stop.status.success(),
        "Force stop should succeed. stderr: {}",
        String::from_utf8_lossy(&stop.stderr)
    );

    // Locks must be gone immediately after a successful stop.
    let temp_dir = test_lockdir();
    let server_lock = temp_dir.join(format!("{}.server.json", server_name));
    assert!(
        !server_lock.exists(),
        "Server lockfile must be gone after successful force stop"
    );

    // Immediately start instance #2 with the same name.
    let output = run_command(&[
        "use",
        server_name,
        "--pid",
        &test_pid,
        "--grace-period",
        "30s",
        "--",
        long_running_script.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "Restart should succeed. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Give any stale watcher from instance #1 a chance to (wrongly) delete the
    // new instance's lockfiles, then confirm instance #2 is still healthy.
    thread::sleep(Duration::from_secs(2));
    let list_output = run_command(&["list"]);
    let list_str = String::from_utf8_lossy(&list_output.stdout);
    assert!(
        list_str.contains(server_name),
        "Restarted server should appear in list (not clobbered by old watcher)"
    );
    assert!(
        server_lock.exists(),
        "Restarted server's lockfile must still exist"
    );

    // Cleanup
    run_command(&["admin", "kill", server_name]);
    thread::sleep(Duration::from_secs(1));
    cleanup_lock_files(server_name);
}

#[test]
#[serial]
fn test_incref_idempotent_and_grace_keeps_clients_lock() {
    // H1: a repeat attach from the SAME client PID must not inflate the refcount.
    // H3: when the refcount hits 0 the server enters grace but stays alive, and
    // the clients lockfile is kept (no longer deleted mid-life).
    let server_name = "test_idem_grace";
    cleanup_lock_files(server_name);

    let long_running = get_test_helper_path("long_running.sh");
    let test_pid = std::process::id().to_string();

    let out = run_command(&[
        "use",
        server_name,
        "--pid",
        &test_pid,
        "--grace-period",
        "30s",
        "--",
        long_running.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "use should succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    thread::sleep(Duration::from_secs(2));

    // H1: second incref from the same PID — refcount must stay 1.
    let inc = run_command(&["admin", "incref", server_name, "--pid", &test_pid]);
    assert!(
        inc.status.success(),
        "incref should succeed: {}",
        String::from_utf8_lossy(&inc.stderr)
    );
    let info = run_command(&["info", server_name, "--json"]);
    let info_s = String::from_utf8_lossy(&info.stdout);
    assert!(
        info_s.contains("\"refcount\": 1"),
        "same-PID re-incref must keep refcount at 1, got:\n{}",
        info_s
    );

    // H3: a single decref drops to 0 -> grace (alive, exit 1), not stopped, and
    // the clients lockfile must still exist.
    let dec = run_command(&["admin", "decref", server_name, "--pid", &test_pid]);
    assert!(
        dec.status.success(),
        "decref should succeed: {}",
        String::from_utf8_lossy(&dec.stderr)
    );
    let chk = run_command(&["check", server_name]);
    assert_eq!(
        chk.status.code(),
        Some(1),
        "after decref to 0 the server should be in grace (exit 1), not stopped"
    );
    let clients_lock = test_lockdir().join(format!("{}.clients.json", server_name));
    assert!(
        clients_lock.exists(),
        "clients lockfile must persist during grace (H3)"
    );

    run_command(&["admin", "kill", server_name]);
    thread::sleep(Duration::from_secs(1));
    cleanup_lock_files(server_name);
}

#[test]
fn test_admin_incref_decref_require_pid() {
    // M3: the low-level admin incref/decref must require --pid (no self-default,
    // which would register the short-lived CLI process as a dead client).
    let inc = run_command(&["admin", "incref", "whatever"]);
    assert!(
        !inc.status.success(),
        "admin incref without --pid should fail"
    );
    let inc_err = String::from_utf8_lossy(&inc.stderr).to_lowercase();
    assert!(
        inc_err.contains("pid"),
        "incref error should mention the required --pid, got: {}",
        inc_err
    );

    let dec = run_command(&["admin", "decref", "whatever"]);
    assert!(
        !dec.status.success(),
        "admin decref without --pid should fail"
    );
    let dec_err = String::from_utf8_lossy(&dec.stderr).to_lowercase();
    assert!(
        dec_err.contains("pid"),
        "decref error should mention the required --pid, got: {}",
        dec_err
    );
}

#[test]
#[serial]
fn test_corrupt_server_lock_is_tolerated_and_cleaned() {
    // M4: a corrupt/unparseable server lock must read as Stopped (not a hard
    // error from every command). M1: doctor must clean it rather than abort.
    let server_name = "test_corrupt";
    cleanup_lock_files(server_name);

    let lockdir = test_lockdir();
    let _ = fs::create_dir_all(&lockdir);
    let server_lock = lockdir.join(format!("{}.server.json", server_name));
    fs::write(&server_lock, b"this is not valid json {{{").expect("write corrupt lock");

    // `check` must report stopped (exit 2), not crash with a parse error.
    let chk = run_command(&["check", server_name]);
    assert_eq!(
        chk.status.code(),
        Some(2),
        "corrupt lock should read as stopped (exit 2). stderr: {}",
        String::from_utf8_lossy(&chk.stderr)
    );

    // `doctor` must succeed and remove the corrupt lock.
    let doc = run_command(&["admin", "doctor", server_name]);
    assert!(
        doc.status.success(),
        "doctor should succeed on a corrupt lock. stderr: {}",
        String::from_utf8_lossy(&doc.stderr)
    );
    assert!(
        !server_lock.exists(),
        "doctor should have removed the corrupt server lock"
    );

    cleanup_lock_files(server_name);
}

#[test]
fn test_admin_kill_already_stopped() {
    // Test that kill fails gracefully on a stopped server
    let server_name = "test_kill_stopped";
    cleanup_lock_files(server_name);

    // Try to kill a non-existent server
    let output = run_command(&["admin", "kill", server_name]);

    assert!(
        !output.status.success(),
        "Kill should fail on stopped server"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not running") || stderr.contains("Stopped"),
        "Error message should indicate server is not running"
    );

    cleanup_lock_files(server_name);
}
