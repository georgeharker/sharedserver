# Test Suite Documentation: Monitoring and Recovery Tests

## Overview

This test suite (`tests/test-monitoring-and-recovery.sh`) validates the sharedserver manager's monitoring and recovery functionality under various failure scenarios, ensuring lock file integrity, proper watcher behavior, and correct client tracking.

## Test Categories

### Category 1: Server Crash / Failure to Start - Lock File Integrity

Tests that verify lock files maintain consistent and valid state when servers fail or crash.

**Test 1.1: Server fails to start (invalid command)**
- Validates: Lock files must not exist after failed server start
- Scenario: Attempts to start server with invalid/nonexistent command
- Expected: No `*.server.json` or `*.clients.json` files created

**Test 1.2: Server crashes immediately**
- Validates: Watcher detects crash and cleans up lock files
- Scenario: Server process exits with error code immediately after start
- Expected: Lock files cleaned up within 5-6 seconds (watcher poll interval)

**Test 1.3: Server with active clients crashes**
- Validates: Lock files contain valid state before cleanup; no empty/null PIDs
- Scenario: Server with registered clients is killed with SIGKILL
- Steps:
  1. Start server, add client
  2. Verify lock files exist and contain valid JSON with correct PID and refcount
  3. Kill server process
  4. Verify watcher cleans up within 5-6 seconds
- Expected: Lock files valid before crash, cleaned up after

**Test 1.4: Empty/corrupted lock file handling**
- Validates: Commands handle corrupted lock files gracefully
- Scenario: Create empty lock files manually
- Expected: `sharedserver info` fails gracefully without crashing

### Category 2: Server Use/Unuse Tracking

Tests that verify client reference counting works correctly.

**Test 2.1: Multiple incref/decref maintain correct refcount**
- Validates: Reference counting is accurate through multiple operations
- Scenario:
  1. Start server
  2. Add 3 clients (incref × 3)
  3. Remove 1 client (decref × 1)
  4. Remove remaining clients
- Expected: 
  - Refcount = 3 after increfs
  - Refcount = 2 after first decref
  - Clients lockfile deleted when refcount reaches 0

**Test 2.2: Decref of non-existent client handled gracefully**
- Validates: Decref of unknown PID doesn't crash or stop server
- Scenario: Attempt to decref a PID that was never registered
- Expected: Server continues running, no errors

### Category 3: Watcher Behavior on Server Exit

Tests that verify the watcher process correctly monitors and cleans up after server exits.

**Test 3.1: Normal server exit - watcher updates and exits**
- Validates: Watcher detects clean server exit and cleans up
- Scenario: Server process exits normally (exit code 0)
- Expected: Lock files cleaned up, watcher exits

**Test 3.2: Server killed with SIGTERM**
- Validates: Watcher detects SIGTERM and cleans up
- Scenario: Server process killed with `kill -TERM`
- Expected: Lock files cleaned up within 5-6 seconds

**Test 3.3: Server killed with SIGKILL**
- Validates: Watcher detects SIGKILL and cleans up
- Scenario: Server process killed with `kill -9`
- Expected: Lock files cleaned up within 5-6 seconds

### Category 4: Client Exit Handling and Timeout

Tests that verify dead client detection and grace period handling.

**Test 4.1: Dead client detection**
- Validates: Watcher detects dead clients and adjusts refcount
- Scenario:
  1. Start server with 2 clients
  2. Let one client process die
  3. Wait for watcher poll (5 seconds)
- Expected: Dead client removed from `clients.json`, refcount decremented

**Test 4.2: All clients exit - grace period triggered**
- Validates: Server enters grace period when refcount reaches 0
- Scenario: Remove all clients via decref
- Expected:
  - `clients.json` deleted
  - Server still running (during grace period)

**Test 4.3: Grace period timeout - server shut down**
- Validates: Server terminates after grace period expires
- Scenario: 
  1. Start server with 5s grace period
  2. Trigger grace period (remove all clients)
  3. Wait 7 seconds
- Expected:
  - Lock files deleted
  - Server process terminated

**Test 4.4: New client during grace period - shutdown cancelled**
- Validates: Grace period cancellation when new client attaches
- Scenario:
  1. Start server with 10s grace period
  2. Trigger grace period
  3. Wait 2 seconds
  4. Add new client
  5. Wait past original grace period
- Expected:
  - `clients.json` recreated with refcount=1
  - Server still running after grace period would have expired

## Running the Tests

### Prerequisites

1. Build the sharedserver binary:
```bash
cd rust
cargo build --release
```

2. Ensure `jq` is installed (for JSON parsing):
```bash
# macOS
brew install jq

# Ubuntu/Debian
apt-get install jq
```

### Execute Tests

```bash
# Run all tests
./tests/test-monitoring-and-recovery.sh

# Run with debug output
SHAREDSERVER_DEBUG=1 ./tests/test-monitoring-and-recovery.sh

# Fail fast (exit on first failure)
FAIL_FAST=1 ./tests/test-monitoring-and-recovery.sh

# Use custom sharedserver binary
SHAREDSERVER=/path/to/custom/sharedserver ./tests/test-monitoring-and-recovery.sh
```

### Expected Output

```
=========================================
sharedserver Monitoring & Recovery Tests
=========================================
Binary: /path/to/sharedserver
Lock directory: /tmp/sharedserver-test-monitoring-XXXXX

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Category 1: Server Crash/Failure - Lock File Integrity
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
ℹ Test 1.1: Server fails to start (invalid command) - lock files must not exist
✓ Test 1.1: server lockfile correctly not created after failed start
...

=========================================
Test Summary
=========================================
Total tests:  XX
Passed:       XX
Failed:       0

All tests passed!
```

## Test Implementation Details

### Lock File Locations

Tests use isolated lock directory: `/tmp/sharedserver-test-monitoring-$$`
- `$SHAREDSERVER_LOCKDIR` environment variable set to this directory
- Automatic cleanup on exit via `trap`

### Timing Considerations

- Watcher poll interval: 5 seconds
- Most tests wait 6 seconds after triggering events to ensure watcher has polled
- Grace periods set to 5-30 seconds depending on test requirements

### Lock File Structure

**server.json:**
```json
{
  "pid": 12345,
  "command": ["sleep", "3600"],
  "grace_period": "5m",
  "watcher_pid": 12346,
  "started_at": "2024-01-01T00:00:00Z"
}
```

**clients.json:**
```json
{
  "refcount": 2,
  "clients": {
    "10001": {
      "attached_at": "2024-01-01T00:00:00Z",
      "metadata": "client1"
    },
    "10002": {
      "attached_at": "2024-01-01T00:00:00Z",
      "metadata": "client2"
    }
  }
}
```

## Validation Criteria

### Lock File Validity

1. **Non-empty**: Files must contain valid JSON
2. **Required fields**: 
   - server.json: `pid`, `command`, `grace_period`, `watcher_pid`, `started_at`
   - clients.json: `refcount`, `clients`
3. **Field values**: 
   - PIDs must be non-null, non-empty integers
   - Refcount must match number of clients in `clients` map

### State Transitions

1. **ACTIVE → GRACE**: Triggered when refcount reaches 0
   - `clients.json` deleted
   - `server.json` remains
   
2. **GRACE → ACTIVE**: Triggered when new client attaches
   - `clients.json` recreated
   
3. **GRACE → STOPPED**: Triggered when grace period expires
   - All lock files deleted
   - Server process terminated

4. **CRASHED**: Any state → STOPPED when server process dies
   - Watcher detects within 5 seconds
   - All lock files cleaned up

## Troubleshooting

### Tests fail with "sharedserver not found"

Build the release binary:
```bash
cd rust && cargo build --release
```

### Tests fail with "jq: command not found"

Install jq:
```bash
brew install jq  # macOS
apt-get install jq  # Linux
```

### Tests hang or timeout

Check for stray processes:
```bash
pkill -f "sleep 3600"
pkill -f "sharedserver.*watch"
```

Clean up test lock directories:
```bash
rm -rf /tmp/sharedserver-test-monitoring-*
```

### False failures due to timing

Some tests may fail on slow systems if the 5-second watcher poll interval isn't sufficient. Increase sleep durations in the test script if needed.

## Integration with CI/CD

### GitHub Actions Example

```yaml
name: Test Monitoring and Recovery

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      
      - name: Install Rust
        uses: actions-rs/toolchain@v1
        with:
          toolchain: stable
      
      - name: Install jq
        run: sudo apt-get install -y jq
      
      - name: Build sharedserver
        run: cd rust && cargo build --release
      
      - name: Run monitoring tests
        run: ./tests/test-monitoring-and-recovery.sh
```

## Future Enhancements

Potential additions to the test suite:

1. **Stress testing**: Multiple concurrent servers with many clients
2. **Race condition testing**: Rapid incref/decref operations
3. **Filesystem failure simulation**: Unwritable lock directory
4. **Network filesystem testing**: Lock files on NFS/CIFS
5. **Performance benchmarking**: Watcher overhead measurement
6. **Memory leak detection**: Long-running server monitoring
