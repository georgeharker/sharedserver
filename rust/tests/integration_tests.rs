use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::Duration;
use serial_test::serial;

/// Get the path to the sharedserver binary
fn get_binary_path() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("target");
    // Always use release build for integration tests
    path.push("release");
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
    let temp_dir = env::temp_dir().join("sharedserver");

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
    let child = Command::new(&binary)
        .args(args)
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
            return handle.join().expect("Thread panicked").expect("Failed to wait for child");
        }
        
        if start.elapsed() > timeout {
            // Timeout - kill the process
            eprintln!("Command timed out after {:?}: {:?}", timeout, args);
            unsafe {
                libc::kill(child_id as i32, libc::SIGKILL);
            }
            panic!("Command timed out after {:?}: sharedserver {}", timeout, args.join(" "));
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

    // Wait for watcher to detect death and clean up (polls every 5s + buffer)
    thread::sleep(Duration::from_secs(7));

    // CRITICAL: Both lock files must be deleted
    let temp_dir = env::temp_dir().join("sharedserver");
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
fn test_server_lifecycle() {
    // SMOKE TEST: Basic sanity check that start/stop commands work.
    // The comprehensive bash test suite covers detailed lifecycle scenarios.
    // This just verifies the CLI fundamentals are functional.

    let server_name = "test_lifecycle";
    cleanup_lock_files(server_name);

    let echo_env_script = get_test_helper_path("echo_env.sh");

    // Start server - should succeed
    let output = run_command(&[
        "admin",
        "start",
        server_name,
        "--",
        echo_env_script.to_str().unwrap(),
    ]);
    assert!(output.status.success(), "Start command should work");

    // Attempt to stop (may already be stopped if server died quickly)
    thread::sleep(Duration::from_secs(1));
    run_command(&["admin", "stop", server_name]);

    // Cleanup
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
        eprintln!("Info stderr: {}", String::from_utf8_lossy(&info_output.stderr));
        eprintln!("Info stdout: {}", String::from_utf8_lossy(&info_output.stdout));
    }
    let info_str = String::from_utf8_lossy(&info_output.stdout);
    eprintln!("Info output:\n{}", info_str);

    // Extract PID from output (format: "Server: name (PID: 12345)")
    let pid: i32 = info_str
        .lines()
        .find(|line| line.contains("PID:"))
        .and_then(|line| {
            line.split("PID:")
                .nth(1)?
                .split(')')
                .next()?
                .trim()
                .parse()
                .ok()
        })
        .unwrap_or_else(|| {
            panic!("Should find PID in info output. Full output:\n{}", info_str);
        });

    // Kill the process directly (simulating crash/SIGKILL)
    #[cfg(unix)]
    {
        use std::process::Command as ProcessCommand;
        ProcessCommand::new("kill")
            .args(&["-9", &pid.to_string()])
            .output()
            .expect("Should be able to kill process");
    }

    // Wait a moment for process to die
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
    let temp_dir = env::temp_dir().join("sharedserver");
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
    eprintln!("Starting server with script: {}", long_running_script.display());

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
    eprintln!("Server info after start:\n{}", String::from_utf8_lossy(&info_output.stdout));
    eprintln!("Server info stderr:\n{}", String::from_utf8_lossy(&info_output.stderr));

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
    let temp_dir = env::temp_dir().join("sharedserver");
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
