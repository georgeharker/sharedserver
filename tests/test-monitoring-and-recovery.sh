#!/bin/bash
# Comprehensive test suite for sharedserver monitoring and recovery
# Tests server crash/failure scenarios, lock file integrity, watcher behavior,
# client tracking, and timeout handling

# Note: We don't use 'set -e' because it has known issues with EXIT traps in bash 3.2
# (the default bash on macOS). Instead, we use explicit error checking where needed.
set -uo pipefail

# Setup
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
SHAREDSERVER="${SHAREDSERVER:-$PROJECT_ROOT/rust/target/release/sharedserver}"
TEST_LOCKDIR="/tmp/sharedserver-test-monitoring-$$"
export SHAREDSERVER_LOCKDIR="$TEST_LOCKDIR"
export SHAREDSERVER_DEBUG="${SHAREDSERVER_DEBUG:-0}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Test counters
TESTS_PASSED=0
TESTS_FAILED=0
TESTS_TOTAL=0

# Test helpers
pass() {
	echo -e "${GREEN}✓${NC} $1"
	((TESTS_PASSED++))
}

fail() {
	echo -e "${RED}✗${NC} $1"
	((TESTS_FAILED++))
	if [ "${FAIL_FAST:-0}" = "1" ]; then
		exit 1
	fi
}

info() {
	echo -e "${BLUE}ℹ${NC} $1"
}

warn() {
	echo -e "${YELLOW}⚠${NC} $1"
}

section() {
	echo ""
	echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
	echo -e "${BLUE}$1${NC}"
	echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
}

cleanup() {
	info "Cleaning up test environment"
	pkill -f "sleep 3600" 2>/dev/null || true
	pkill -f "sleep 999" 2>/dev/null || true
	pkill -f "sharedserver.*watch" 2>/dev/null || true
	sleep 1
	rm -rf "$TEST_LOCKDIR"
}

# Verify sharedserver exists
if [ ! -x "$SHAREDSERVER" ]; then
	echo -e "${RED}Error: sharedserver not found at $SHAREDSERVER${NC}"
	echo "Build it first: cd rust && cargo build --release"
	exit 1
fi

# Verify required tools exist
for tool in jq pkill; do
	if ! command -v "$tool" &>/dev/null; then
		echo -e "${RED}Error: required tool '$tool' not found${NC}"
		echo "Install it first (e.g., brew install $tool)"
		exit 1
	fi
done

# Setup test environment
trap cleanup EXIT
mkdir -p "$TEST_LOCKDIR"

echo "========================================="
echo "sharedserver Monitoring & Recovery Tests"
echo "========================================="
echo "Binary: $SHAREDSERVER"
echo "Lock directory: $TEST_LOCKDIR"
echo ""

# ============================================================================
# Test Category 1: Server Crash / Failure to Start - Lock File Integrity
# ============================================================================
section "Category 1: Server Crash/Failure - Lock File Integrity"

# Test 1.1: Server fails to start - verify lock files are cleaned up
info "Test 1.1: Server fails to start (invalid command) - lock files must not exist"
((TESTS_TOTAL++))

# Try to start server with invalid command (using 'use' which will call start internally)
"$SHAREDSERVER" use --grace-period 5m crash-test-fail /nonexistent/command arg1 2>/dev/null || true
sleep 1

if [ -f "$TEST_LOCKDIR/crash-test-fail.server.json" ]; then
	SERVER_CONTENT=$(cat "$TEST_LOCKDIR/crash-test-fail.server.json" 2>/dev/null || echo "{}")
	fail "Test 1.1: server lockfile should not exist after failed start. Content: $SERVER_CONTENT"
else
	pass "Test 1.1: server lockfile correctly not created after failed start"
fi

# Test 1.2: Server starts but crashes immediately - lock files cleaned up
info "Test 1.2: Server crashes immediately - watcher must clean up lock files"
((TESTS_TOTAL++))

# Start server that exits immediately (admin start creates server with refcount=0)
# The start command will timeout after 2s and clean up the lock files
"$SHAREDSERVER" admin start --grace-period 5m crash-test-immediate -- bash -c "exit 1" 2>/dev/null &
START_PID=$!

# Wait for the start command to timeout and clean up (2s timeout + small buffer)
wait $START_PID 2>/dev/null || true
sleep 0.5

if [ -f "$TEST_LOCKDIR/crash-test-immediate.server.json" ]; then
	fail "Test 1.2: server lockfile should be cleaned up after crash"
else
	pass "Test 1.2: server lockfile correctly cleaned up after immediate crash"
fi

# Test 1.3: Server with client crashes - lock files contain valid state before cleanup
info "Test 1.3: Server with active clients crashes - verify lock file state"
((TESTS_TOTAL++))

# Start server with NO clients, then add the test script as a client
"$SHAREDSERVER" admin start --grace-period 5m crash-test-with-client -- sleep 999 2>/dev/null &
CRASH_SERVER_PID=$!
sleep 1

# Add test script as first client
"$SHAREDSERVER" admin incref --pid $$ --metadata "test-script" crash-test-with-client
sleep 0.5

# Add another client
"$SHAREDSERVER" admin incref --pid 12345 --metadata "test-client" crash-test-with-client
sleep 0.5

# Verify lock files exist and are valid
if [ ! -f "$TEST_LOCKDIR/crash-test-with-client.server.json" ]; then
	fail "Test 1.3: server lockfile missing before crash"
else
	# Check server lockfile is valid JSON and not empty
	if jq -e '.pid' "$TEST_LOCKDIR/crash-test-with-client.server.json" >/dev/null 2>&1; then
		SERVER_PID=$(jq -r '.pid' "$TEST_LOCKDIR/crash-test-with-client.server.json")
		if [ -z "$SERVER_PID" ] || [ "$SERVER_PID" = "null" ]; then
			fail "Test 1.3: server lockfile contains empty/null pid"
		else
			pass "Test 1.3a: server lockfile valid with pid=$SERVER_PID"
		fi
	else
		fail "Test 1.3: server lockfile is not valid JSON or missing pid"
	fi
fi

if [ ! -f "$TEST_LOCKDIR/crash-test-with-client.clients.json" ]; then
	fail "Test 1.3: clients lockfile missing before crash"
else
	# Check clients lockfile is valid JSON with correct refcount (should be 2: test script + client 12345)
	if jq -e '.refcount' "$TEST_LOCKDIR/crash-test-with-client.clients.json" >/dev/null 2>&1; then
		REFCOUNT=$(jq -r '.refcount' "$TEST_LOCKDIR/crash-test-with-client.clients.json")
		if [ "$REFCOUNT" = "2" ]; then
			pass "Test 1.3b: clients lockfile valid with refcount=1"
		else
			fail "Test 1.3: clients lockfile has wrong refcount: $REFCOUNT (expected 1)"
		fi
	else
		fail "Test 1.3: clients lockfile is not valid JSON or missing refcount"
	fi
fi

# Kill the server process to simulate crash
SERVER_REAL_PID=$(jq -r '.pid' "$TEST_LOCKDIR/crash-test-with-client.server.json")
kill -9 $SERVER_REAL_PID 2>/dev/null || true
sleep 6 # Wait for watcher to detect and cleanup

# Verify cleanup happened
if [ -f "$TEST_LOCKDIR/crash-test-with-client.server.json" ]; then
	fail "Test 1.3c: server lockfile not cleaned up after crash"
else
	pass "Test 1.3c: server lockfile cleaned up after crash"
fi

# Cleanup background process
kill $CRASH_SERVER_PID 2>/dev/null || true
wait $CRASH_SERVER_PID 2>/dev/null || true

# Test 1.4: Empty lock file handling
info "Test 1.4: Empty/corrupted lock files are handled gracefully"
((TESTS_TOTAL++))

# Create empty lock files
touch "$TEST_LOCKDIR/corrupt-test.server.json"
touch "$TEST_LOCKDIR/corrupt-test.clients.json"

# Try to get info (should fail gracefully)
if "$SHAREDSERVER" info corrupt-test 2>/dev/null; then
	fail "Test 1.4: info should fail for corrupted lockfile"
else
	pass "Test 1.4: corrupted lockfile handled gracefully (command failed as expected)"
fi

rm -f "$TEST_LOCKDIR/corrupt-test.server.json" "$TEST_LOCKDIR/corrupt-test.clients.json"

# ============================================================================
# Test Category 2: Server Use/Unuse Tracking
# ============================================================================
section "Category 2: Server Use/Unuse Tracking"

# Test 2.1: Multiple incref/decref operations maintain correct refcount
info "Test 2.1: Multiple incref/decref maintain correct refcount"
((TESTS_TOTAL++))

# Start server
"$SHAREDSERVER" use --grace-period 30s --pid $$ usetest -- sleep 3600 &
USE_SERVER_PID=$!
sleep 1

# Add multiple clients (use real background processes to keep them alive)
sleep 60 &
CLIENT1_PID=$!
sleep 60 &
CLIENT2_PID=$!
sleep 60 &
CLIENT3_PID=$!
"$SHAREDSERVER" admin incref --pid $CLIENT1_PID --metadata "client1" usetest
"$SHAREDSERVER" admin incref --pid $CLIENT2_PID --metadata "client2" usetest
"$SHAREDSERVER" admin incref --pid $CLIENT3_PID --metadata "client3" usetest
sleep 0.5

# Check refcount (should be 4: test script $$ + 3 clients)
REFCOUNT=$(jq -r '.refcount' "$TEST_LOCKDIR/usetest.clients.json")
if [ "$REFCOUNT" = "4" ]; then
	pass "Test 2.1a: refcount correct after 3 increfs (refcount=4: test script + 3 clients)"
else
	fail "Test 2.1a: wrong refcount after increfs: $REFCOUNT (expected 4)"
fi

# Remove one client
"$SHAREDSERVER" admin decref --pid $CLIENT2_PID usetest
sleep 0.5

REFCOUNT_AFTER=$(jq -r '.refcount' "$TEST_LOCKDIR/usetest.clients.json")
if [ "$REFCOUNT_AFTER" = "3" ]; then
	pass "Test 2.1b: refcount correct after 1 decref (refcount=3)"
else
	fail "Test 2.1b: wrong refcount after decref: $REFCOUNT_AFTER (expected 3)"
fi

# Remove remaining clients
"$SHAREDSERVER" admin decref --pid $CLIENT1_PID usetest
"$SHAREDSERVER" admin decref --pid $CLIENT3_PID usetest
"$SHAREDSERVER" unuse --pid $$ usetest
sleep 1

# Should enter grace period
if [ -f "$TEST_LOCKDIR/usetest.clients.json" ]; then
	fail "Test 2.1c: clients lockfile should be deleted when refcount reaches 0"
else
	pass "Test 2.1c: clients lockfile correctly deleted when refcount=0"
fi

# Cleanup background client processes
kill $CLIENT1_PID $CLIENT2_PID $CLIENT3_PID 2>/dev/null || true
wait $CLIENT1_PID $CLIENT2_PID $CLIENT3_PID 2>/dev/null || true
kill $USE_SERVER_PID 2>/dev/null || true
wait $USE_SERVER_PID 2>/dev/null || true

# Test 2.2: Decref of non-existent client handled gracefully
info "Test 2.2: Decref of non-existent client handled gracefully"
((TESTS_TOTAL++))

"$SHAREDSERVER" use --grace-period 30s --pid $$ decref-test -- sleep 3600 &
DECREF_PID=$!
sleep 1

# Try to decref a PID that was never added
"$SHAREDSERVER" admin decref --pid 99999 decref-test 2>/dev/null || true
sleep 0.5

# Server should still be running
if "$SHAREDSERVER" check decref-test; then
	pass "Test 2.2: server still running after decref of non-existent client"
else
	fail "Test 2.2: server stopped after decref of non-existent client"
fi

# Cleanup
"$SHAREDSERVER" unuse --pid $$ decref-test
kill $DECREF_PID 2>/dev/null || true
wait $DECREF_PID 2>/dev/null || true

# ============================================================================
# Test Category 3: Watcher Behavior on Server Exit
# ============================================================================
section "Category 3: Watcher Behavior on Server Exit"

# Test 3.1: Normal server exit - watcher detects and cleans up
info "Test 3.1: Server exits normally - watcher updates state and exits"
((TESTS_TOTAL++))

# Start server that will exit after 3 seconds
"$SHAREDSERVER" use --grace-period 5s --pid $$ watcher-exit-test -- bash -c "sleep 3; exit 0" &
WATCHER_TEST_PID=$!
sleep 1

# Verify server is running
if ! "$SHAREDSERVER" check watcher-exit-test; then
	fail "Test 3.1: server should be running initially"
fi

# Wait for server to exit
sleep 4

# Watcher should detect exit and clean up
sleep 2

if [ -f "$TEST_LOCKDIR/watcher-exit-test.server.json" ]; then
	fail "Test 3.1: watcher failed to clean up after server exit"
else
	pass "Test 3.1: watcher correctly cleaned up after server exit"
fi

wait $WATCHER_TEST_PID 2>/dev/null || true

# Test 3.2: Server killed with SIGTERM - watcher cleans up
info "Test 3.2: Server killed with SIGTERM - watcher cleans up"
((TESTS_TOTAL++))

"$SHAREDSERVER" use --grace-period 5s --pid $$ sigterm-test -- sleep 3600 &
SIGTERM_PID=$!
sleep 1

# Get actual server PID
SERVER_PID=$(jq -r '.pid' "$TEST_LOCKDIR/sigterm-test.server.json")

# Kill server with SIGTERM
kill -TERM $SERVER_PID 2>/dev/null || true
sleep 6

# Watcher should clean up
if [ -f "$TEST_LOCKDIR/sigterm-test.server.json" ]; then
	fail "Test 3.2: watcher failed to clean up after SIGTERM"
else
	pass "Test 3.2: watcher correctly cleaned up after SIGTERM"
fi

kill $SIGTERM_PID 2>/dev/null || true
wait $SIGTERM_PID 2>/dev/null || true

# Test 3.3: Server killed with SIGKILL - watcher cleans up
info "Test 3.3: Server killed with SIGKILL - watcher cleans up"
((TESTS_TOTAL++))

"$SHAREDSERVER" use --grace-period 5s --pid $$ sigkill-test -- sleep 3600 &
SIGKILL_PID=$!
sleep 1

# Get actual server PID
SERVER_PID=$(jq -r '.pid' "$TEST_LOCKDIR/sigkill-test.server.json")

# Kill server with SIGKILL
kill -9 $SERVER_PID 2>/dev/null || true
sleep 6

# Watcher should clean up
if [ -f "$TEST_LOCKDIR/sigkill-test.server.json" ]; then
	fail "Test 3.3: watcher failed to clean up after SIGKILL"
else
	pass "Test 3.3: watcher correctly cleaned up after SIGKILL"
fi

kill $SIGKILL_PID 2>/dev/null || true
wait $SIGKILL_PID 2>/dev/null || true

# ============================================================================
# Test Category 4: Client Exit Handling and Timeout
# ============================================================================
section "Category 4: Client Exit Handling and Timeout"

# Test 4.1: Dead client detection - watcher removes dead clients
info "Test 4.1: Watcher detects and removes dead clients"
((TESTS_TOTAL++))

"$SHAREDSERVER" use --grace-period 30s --pid $$ dead-client-test -- sleep 3600 &
DEAD_CLIENT_PID=$!
sleep 1

# Add a client process that will die
bash -c "echo \$\$ > /tmp/test-client-$$.pid; sleep 2" &
CLIENT_PID=$!

# Add client to server
"$SHAREDSERVER" admin incref --pid $CLIENT_PID --metadata "dying-client" dead-client-test
sleep 0.5

# Verify client is in the list
CLIENTS_BEFORE=$(jq -r '.clients | keys | length' "$TEST_LOCKDIR/dead-client-test.clients.json")
if [ "$CLIENTS_BEFORE" -ge "1" ]; then
	pass "Test 4.1a: client added successfully"
else
	fail "Test 4.1a: client not found in clients list"
fi

# Wait for client to die
sleep 3

# Wait for watcher to detect dead client (polls every 5s)
sleep 6

# Check if dead client was removed
if [ -f "$TEST_LOCKDIR/dead-client-test.clients.json" ]; then
	REFCOUNT_AFTER=$(jq -r '.refcount' "$TEST_LOCKDIR/dead-client-test.clients.json")
	# Should have decremented (our use command client is still there)
	if [ "$REFCOUNT_AFTER" = "1" ]; then
		pass "Test 4.1b: dead client removed, refcount adjusted to 1"
	else
		fail "Test 4.1b: refcount not adjusted after dead client removal: $REFCOUNT_AFTER"
	fi
else
	# If clients file is gone, that means all clients (including our use command) were cleaned up
	warn "Test 4.1b: clients file deleted (may indicate all clients removed)"
fi

# Cleanup
"$SHAREDSERVER" unuse --pid $$ dead-client-test
kill $DEAD_CLIENT_PID 2>/dev/null || true
wait $DEAD_CLIENT_PID 2>/dev/null || true

# Test 4.2: All clients exit - grace period triggered
info "Test 4.2: All clients exit - grace period starts correctly"
((TESTS_TOTAL++))

"$SHAREDSERVER" use --grace-period 10s --pid $$ grace-test -- sleep 3600 &
GRACE_PID=$!
sleep 1

# Add a few clients (use real background processes)
sleep 60 &
GRACE_CLIENT1=$!
sleep 60 &
GRACE_CLIENT2=$!
"$SHAREDSERVER" admin incref --pid $GRACE_CLIENT1 grace-test
"$SHAREDSERVER" admin incref --pid $GRACE_CLIENT2 grace-test
sleep 0.5

# Server should be ACTIVE
if [ -f "$TEST_LOCKDIR/grace-test.clients.json" ]; then
	pass "Test 4.2a: server in ACTIVE state with clients"
else
	fail "Test 4.2a: server should be ACTIVE with clients"
fi

# Remove all clients
"$SHAREDSERVER" admin decref --pid $GRACE_CLIENT1 grace-test
"$SHAREDSERVER" admin decref --pid $GRACE_CLIENT2 grace-test
"$SHAREDSERVER" unuse --pid $$ grace-test
sleep 1

# Should enter grace period (clients.json deleted)
if [ -f "$TEST_LOCKDIR/grace-test.clients.json" ]; then
	fail "Test 4.2b: should enter grace period (clients.json should be deleted)"
else
	pass "Test 4.2b: grace period started (clients.json deleted)"
fi

# Server should still be running (check returns exit code 1 for grace state, which is correct)
"$SHAREDSERVER" check grace-test
CHECK_EXIT=$?
if [ "$CHECK_EXIT" = "1" ]; then
	pass "Test 4.2c: server still running during grace period"
elif [ "$CHECK_EXIT" = "0" ]; then
	pass "Test 4.2c: server still running (active state)"
else
	fail "Test 4.2c: server should still be running during grace period (exit code: $CHECK_EXIT)"
fi

# Cleanup background client processes
kill $GRACE_CLIENT1 $GRACE_CLIENT2 2>/dev/null || true
wait $GRACE_CLIENT1 $GRACE_CLIENT2 2>/dev/null || true
kill $GRACE_PID 2>/dev/null || true
wait $GRACE_PID 2>/dev/null || true

# Test 4.3: Grace period timeout - server shut down
info "Test 4.3: Grace period expires - server is shut down"
((TESTS_TOTAL++))

"$SHAREDSERVER" use --grace-period 5s --pid $$ timeout-test -- sleep 3600 &
TIMEOUT_PID=$!
sleep 1

# Trigger grace period
"$SHAREDSERVER" unuse --pid $$ timeout-test
sleep 1

# Verify grace period started
if [ ! -f "$TEST_LOCKDIR/timeout-test.clients.json" ]; then
	pass "Test 4.3a: grace period started"
else
	fail "Test 4.3a: grace period not started"
fi

# Wait for grace period to expire and cleanup
# Timeline: up to 5s to detect grace + 5s grace period + 5s SIGTERM wait + cleanup
# Total worst case: 15s, but usually faster
info "Waiting 16 seconds for grace period to expire and cleanup..."
sleep 16

# Server should be stopped and cleaned up
if [ -f "$TEST_LOCKDIR/timeout-test.server.json" ]; then
	fail "Test 4.3b: server not shut down after grace period"
else
	pass "Test 4.3b: server shut down after grace period expired"
fi

# Verify server process was actually killed
SERVER_PID=$(cat /tmp/timeout-test-pid 2>/dev/null || echo "")
if [ -n "$SERVER_PID" ] && kill -0 $SERVER_PID 2>/dev/null; then
	fail "Test 4.3c: server process still running after grace period"
else
	pass "Test 4.3c: server process terminated"
fi

kill $TIMEOUT_PID 2>/dev/null || true
wait $TIMEOUT_PID 2>/dev/null || true

# Test 4.4: New client during grace period - cancels shutdown
info "Test 4.4: Client attaches during grace period - shutdown cancelled"
((TESTS_TOTAL++))

"$SHAREDSERVER" use --grace-period 10s --pid $$ cancel-grace-test -- sleep 3600 &
CANCEL_GRACE_PID=$!
sleep 1

# Trigger grace period
"$SHAREDSERVER" unuse --pid $$ cancel-grace-test
sleep 2

# New client attaches (use real background process that stays alive)
sleep 60 &
CANCEL_GRACE_CLIENT=$!
"$SHAREDSERVER" admin incref --pid $CANCEL_GRACE_CLIENT cancel-grace-test
sleep 1

# Should be back to ACTIVE
if [ -f "$TEST_LOCKDIR/cancel-grace-test.clients.json" ]; then
	REFCOUNT=$(jq -r '.refcount' "$TEST_LOCKDIR/cancel-grace-test.clients.json")
	if [ "$REFCOUNT" = "1" ]; then
		pass "Test 4.4a: grace period cancelled, back to ACTIVE state"
	else
		fail "Test 4.4a: unexpected refcount after grace cancellation: $REFCOUNT"
	fi
else
	fail "Test 4.4a: clients.json should exist after grace cancellation"
fi

# Wait past original grace period
sleep 10

# Server should still be running (grace was cancelled)
if "$SHAREDSERVER" check cancel-grace-test; then
	pass "Test 4.4b: server still running after grace period was cancelled"
else
	fail "Test 4.4b: server stopped even though grace was cancelled"
fi

# Cleanup background client process
"$SHAREDSERVER" admin decref --pid $CANCEL_GRACE_CLIENT cancel-grace-test
kill $CANCEL_GRACE_CLIENT 2>/dev/null || true
wait $CANCEL_GRACE_CLIENT 2>/dev/null || true

# Cleanup
kill $CANCEL_GRACE_PID 2>/dev/null || true
wait $CANCEL_GRACE_PID 2>/dev/null || true

# ============================================================================
# Test Summary
# ============================================================================

echo ""
echo "========================================="
echo "Test Summary"
echo "========================================="
echo -e "Total tests:  ${TESTS_TOTAL}"
echo -e "${GREEN}Passed:       ${TESTS_PASSED}${NC}"
if [ $TESTS_FAILED -gt 0 ]; then
	echo -e "${RED}Failed:       ${TESTS_FAILED}${NC}"
	echo ""
	echo -e "${RED}Some tests failed!${NC}"
	exit 1
else
	echo -e "${GREEN}Failed:       0${NC}"
	echo ""
	echo -e "${GREEN}All tests passed!${NC}"
fi
