# SharedServer Test Suite

This directory contains comprehensive tests for the sharedserver monitoring and recovery system.

## Quick Start

```bash
# Build the project first
cd rust && cargo build --release && cd ..

# Run the monitoring tests
./tests/test-monitoring-and-recovery.sh
```

## Test Files

### `test-monitoring-and-recovery.sh`

**Comprehensive test suite covering:**

1. **Server Crash/Failure Scenarios**
   - Failed server starts (invalid commands)
   - Immediate server crashes
   - Server crashes with active clients
   - Corrupted/empty lock file handling

2. **Client Reference Counting**
   - Multiple incref/decref operations
   - Non-existent client decref handling
   - Refcount accuracy validation

3. **Watcher Process Behavior**
   - Normal server exit detection
   - SIGTERM signal handling
   - SIGKILL signal handling
   - Lock file cleanup verification

4. **Client Exit and Timeout Handling**
   - Dead client detection and removal
   - Grace period triggering
   - Grace period timeout and shutdown
   - Grace period cancellation

**Total Tests:** ~24 individual test cases across 4 categories

### `MONITORING_TESTS.md`

Detailed documentation including:
- Test descriptions and validation criteria
- Expected behavior for each scenario
- Lock file structure reference
- Troubleshooting guide
- CI/CD integration examples

## Test Output

### Success
```
=========================================
sharedserver Monitoring & Recovery Tests
=========================================

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Category 1: Server Crash/Failure - Lock File Integrity
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
✓ Test 1.1: server lockfile correctly not created after failed start
✓ Test 1.2: server lockfile correctly cleaned up after immediate crash
...

Test Summary
Total tests:  24
Passed:       24
Failed:       0

All tests passed!
```

### Failure
```
✗ Test 1.3: server lockfile not cleaned up after crash

Test Summary
Total tests:  24
Passed:       23
Failed:       1

Some tests failed!
```

## Environment Variables

- `SHAREDSERVER_LOCKDIR`: Lock file directory (auto-set to isolated test dir)
- `SHAREDSERVER_DEBUG`: Enable debug output (set to `1` for verbose logging)
- `SHAREDSERVER`: Path to sharedserver binary (defaults to `rust/target/release/sharedserver`)
- `FAIL_FAST`: Exit on first test failure (set to `1` to enable)

## Examples

### Run with debug output
```bash
SHAREDSERVER_DEBUG=1 ./tests/test-monitoring-and-recovery.sh
```

### Stop on first failure
```bash
FAIL_FAST=1 ./tests/test-monitoring-and-recovery.sh
```

### Use custom binary
```bash
SHAREDSERVER=/opt/sharedserver/bin/sharedserver ./tests/test-monitoring-and-recovery.sh
```

## Prerequisites

- **Rust toolchain** (for building sharedserver)
- **jq** (for JSON parsing in tests)
- **bash 4.0+**

### Install jq

macOS:
```bash
brew install jq
```

Ubuntu/Debian:
```bash
sudo apt-get install jq
```

## Test Validation Criteria

Each test validates specific behaviors:

### Lock File Integrity
- ✅ No empty lock files created
- ✅ All PIDs are valid (non-null, non-empty)
- ✅ Refcount matches actual client count
- ✅ Lock files cleaned up after crashes

### Watcher Behavior
- ✅ Detects server exit within 5 seconds
- ✅ Cleans up all lock files
- ✅ Handles all signal types (SIGTERM, SIGKILL)
- ✅ Watcher process exits after cleanup

### Reference Counting
- ✅ Accurate refcount after incref/decref
- ✅ Grace period triggered at refcount=0
- ✅ Dead clients removed automatically
- ✅ Graceful handling of invalid operations

### Grace Period
- ✅ Server stays alive during grace period
- ✅ Server shuts down after timeout
- ✅ Grace period cancelled by new client
- ✅ Lock files in correct state

## Continuous Integration

Example GitHub Actions workflow:

```yaml
name: Tests

on: [push, pull_request]

jobs:
  monitoring-tests:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - uses: actions-rs/toolchain@v1
        with:
          toolchain: stable
      - name: Install dependencies
        run: sudo apt-get install -y jq
      - name: Build
        run: cd rust && cargo build --release
      - name: Run monitoring tests
        run: ./tests/test-monitoring-and-recovery.sh
```

## Troubleshooting

### "sharedserver not found"
Build the binary: `cd rust && cargo build --release`

### "jq: command not found"  
Install jq: `brew install jq` (macOS) or `apt-get install jq` (Linux)

### Tests hang
Kill stray processes:
```bash
pkill -f "sleep 3600"
pkill -f "sharedserver.*watch"
rm -rf /tmp/sharedserver-test-monitoring-*
```

### Timing-related failures
Increase sleep durations in the test script if running on slow systems.

## Contributing

When adding new tests:

1. Follow the existing test structure (numbered tests within categories)
2. Use the provided helper functions (`pass`, `fail`, `info`, `warn`)
3. Increment `TESTS_TOTAL` counter
4. Add documentation to `MONITORING_TESTS.md`
5. Ensure tests clean up resources properly

## Related Documentation

- [MONITORING_TESTS.md](MONITORING_TESTS.md) - Detailed test documentation
- [../rust/sharedserver-core/README.md](../rust/sharedserver-core/README.md) - Core library docs
- [../rust/sharedserver-cli/README.md](../rust/sharedserver-cli/README.md) - CLI documentation
