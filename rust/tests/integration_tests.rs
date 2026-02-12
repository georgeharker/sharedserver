use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::Duration;

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

/// Run a command and return its output
fn run_command(args: &[&str]) -> std::process::Output {
    let binary = get_binary_path();
    Command::new(binary)
        .args(args)
        .output()
        .expect("Failed to execute command")
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
