#!/bin/bash
# Test suite for serverctl + process-wrapper two-tool architecture

set -euo pipefail

# Setup
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SERVERCTL="$SCRIPT_DIR/serverctl"
PROCESS_WRAPPER="$SCRIPT_DIR/process-wrapper"
TEST_LOCKDIR="/tmp/sharedserver-test-$$"
export SHAREDSERVER_LOCKDIR="$TEST_LOCKDIR"
export SHAREDSERVER_DEBUG="${SHAREDSERVER_DEBUG:-0}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Test helpers
pass() {
	echo -e "${GREEN}✓${NC} $1"
}

fail() {
	echo -e "${RED}✗${NC} $1"
	exit 1
}

info() {
	echo -e "${YELLOW}ℹ${NC} $1"
}

cleanup() {
	info "Cleaning up test environment"
	pkill -f "sleep 3600" 2>/dev/null || true
	pkill -f "sharedserver-watcher" 2>/dev/null || true
	rm -rf "$TEST_LOCKDIR"
}

# Setup test environment
trap cleanup EXIT
mkdir -p "$TEST_LOCKDIR"

echo "========================================="
echo "Testing serverctl + process-wrapper"
echo "========================================="
echo "Lock directory: $TEST_LOCKDIR"
echo ""

# Test 1: serverctl check - nonexistent server
info "Test 1: serverctl check - nonexistent server"
if "$SERVERCTL" check test-server 2>/dev/null; then
	fail "Test 1: check should fail for nonexistent server"
else
	pass "Test 1: check correctly returns exit code 1 for nonexistent server"
fi

# Test 2: serverctl start - start new server
info "Test 2: serverctl start - start new server in background"
"$SERVERCTL" start --timeout 10s test-server sleep 3600 &
SERVER_PID=$!
sleep 1 # Wait for server to start

if [ ! -f "$TEST_LOCKDIR/test-server.server.json" ]; then
	fail "Test 2: server lockfile not created"
fi

if [ ! -f "$TEST_LOCKDIR/test-server.clients.json" ]; then
	fail "Test 2: client lockfile not created"
fi

pass "Test 2: serverctl start created both lockfiles"

# Test 3: serverctl check - existing server
info "Test 3: serverctl check - existing server"
if ! "$SERVERCTL" check test-server; then
	fail "Test 3: check should succeed for existing server"
fi
pass "Test 3: check correctly detects running server"

# Test 4: serverctl info - get server details
info "Test 4: serverctl info - get server details"
INFO=$("$SERVERCTL" info test-server)

if [ -z "$INFO" ]; then
	fail "Test 4: info returned empty output"
fi

SERVER_PID_FROM_INFO=$(echo "$INFO" | jq -r '.pid')
if [ "$SERVER_PID_FROM_INFO" != "$SERVER_PID" ]; then
	fail "Test 4: info returned wrong PID (expected $SERVER_PID, got $SERVER_PID_FROM_INFO)"
fi

STATUS=$(echo "$INFO" | jq -r '.status')
if [ "$STATUS" != "active" ]; then
	fail "Test 4: status should be 'active', got '$STATUS'"
fi

REFCOUNT=$(echo "$INFO" | jq -r '.refcount')
if [ "$REFCOUNT" != "0" ]; then
	fail "Test 4: refcount should be 0 (no clients attached), got $REFCOUNT"
fi

pass "Test 4: info returns correct server details (pid=$SERVER_PID, status=$STATUS, refcount=$REFCOUNT)"

# Test 5: serverctl incref - increment refcount
info "Test 5: serverctl incref - increment refcount"
"$SERVERCTL" incref test-server --pid 99999 --type test

REFCOUNT_AFTER=$("$SERVERCTL" info test-server | jq -r '.refcount')
if [ "$REFCOUNT_AFTER" != "1" ]; then
	fail "Test 5: refcount should be 1, got $REFCOUNT_AFTER"
fi

pass "Test 5: incref correctly incremented refcount to $REFCOUNT_AFTER"

# Test 6: serverctl decref - decrement refcount
info "Test 6: serverctl decref - decrement refcount"
"$SERVERCTL" decref test-server 99999

REFCOUNT_AFTER_DECREF=$("$SERVERCTL" info test-server | jq -r '.refcount')
if [ "$REFCOUNT_AFTER_DECREF" != "0" ]; then
	fail "Test 6: refcount should be 0, got $REFCOUNT_AFTER_DECREF"
fi

pass "Test 6: decref correctly decremented refcount to $REFCOUNT_AFTER_DECREF"

# Test 7: serverctl decref last client - triggers grace period
info "Test 7: serverctl decref last client - triggers grace period"
"$SERVERCTL" decref test-server "$SERVER_PID"

sleep 1

if [ -f "$TEST_LOCKDIR/test-server.clients.json" ]; then
	fail "Test 7: client lockfile should be deleted after last decref"
fi

if [ ! -f "$TEST_LOCKDIR/test-server.server.json" ]; then
	fail "Test 7: server lockfile should still exist during grace period"
fi

STATUS_GRACE=$("$SERVERCTL" info test-server | jq -r '.status')
if [ "$STATUS_GRACE" != "grace" ]; then
	fail "Test 7: status should be 'grace', got '$STATUS_GRACE'"
fi

pass "Test 7: decref last client correctly triggered grace period (status=$STATUS_GRACE)"

# Test 8: serverctl incref during grace period - cancel shutdown
info "Test 8: serverctl incref during grace period - cancel shutdown"
"$SERVERCTL" incref test-server --pid 88888 --type test

if [ ! -f "$TEST_LOCKDIR/test-server.clients.json" ]; then
	fail "Test 8: client lockfile should be recreated"
fi

STATUS_AFTER_GRACE=$("$SERVERCTL" info test-server | jq -r '.status')
if [ "$STATUS_AFTER_GRACE" != "active" ]; then
	fail "Test 8: status should be 'active' after grace cancellation, got '$STATUS_AFTER_GRACE'"
fi

pass "Test 8: incref during grace period correctly cancelled shutdown (status=$STATUS_AFTER_GRACE)"

# Test 9: serverctl list - show all servers
info "Test 9: serverctl list - show all servers"
LIST_OUTPUT=$("$SERVERCTL" list)

if [ -z "$LIST_OUTPUT" ]; then
	fail "Test 9: list returned empty output"
fi

LIST_COUNT=$(echo "$LIST_OUTPUT" | jq 'length')
if [ "$LIST_COUNT" -lt 1 ]; then
	fail "Test 9: list should show at least 1 server, got $LIST_COUNT"
fi

pass "Test 9: list correctly shows $LIST_COUNT server(s)"

# Test 10: Grace period timeout - server shuts down
info "Test 10: Grace period timeout (10s) - verify watcher shuts down server"
"$SERVERCTL" decref test-server 88888

info "Waiting 12 seconds for grace period to expire..."
sleep 12

# Check if server lockfile was cleaned up
if [ -f "$TEST_LOCKDIR/test-server.server.json" ]; then
	fail "Test 10: server lockfile should be deleted after grace period expires"
fi

# Check if server process was killed
if kill -0 "$SERVER_PID" 2>/dev/null; then
	fail "Test 10: server process should be killed after grace period"
fi

pass "Test 10: watcher correctly shut down server after grace period"

# Test 11: process-wrapper - auto-decref on exit
info "Test 11: process-wrapper - auto-decref on exit"

# Start a new server
"$SERVERCTL" start --timeout 30s test-wrapper sleep 3600 &
WRAPPER_SERVER_PID=$!
sleep 1

# Increment refcount manually
"$SERVERCTL" incref test-wrapper --pid 77777 --type test

REFCOUNT_BEFORE=$("$SERVERCTL" info test-wrapper | jq -r '.refcount')
if [ "$REFCOUNT_BEFORE" != "1" ]; then
	fail "Test 11: refcount should be 1 before wrapper test, got $REFCOUNT_BEFORE"
fi

# Launch process-wrapper that exits immediately
"$PROCESS_WRAPPER" test-wrapper 77777 -- bash -c "sleep 0.5"

sleep 1

# Verify refcount was decremented
REFCOUNT_AFTER=$("$SERVERCTL" info test-wrapper | jq -r '.refcount')
if [ "$REFCOUNT_AFTER" != "0" ]; then
	fail "Test 11: refcount should be 0 after process-wrapper exit, got $REFCOUNT_AFTER"
fi

pass "Test 11: process-wrapper correctly auto-decremented on exit"

# Test 12: process-wrapper - auto-decref on signal
info "Test 12: process-wrapper - auto-decref on signal (SIGTERM)"

"$SERVERCTL" incref test-wrapper --pid 66666 --type test

REFCOUNT_BEFORE_SIGNAL=$("$SERVERCTL" info test-wrapper | jq -r '.refcount')

# Launch process-wrapper in background, then kill it
"$PROCESS_WRAPPER" test-wrapper 66666 -- sleep 3600 &
WRAPPER_PID=$!
sleep 0.5

kill -TERM "$WRAPPER_PID" 2>/dev/null || true
sleep 1

REFCOUNT_AFTER_SIGNAL=$("$SERVERCTL" info test-wrapper | jq -r '.refcount')
if [ "$REFCOUNT_AFTER_SIGNAL" != "0" ]; then
	fail "Test 12: refcount should be 0 after SIGTERM, got $REFCOUNT_AFTER_SIGNAL"
fi

pass "Test 12: process-wrapper correctly auto-decremented on SIGTERM"

# Cleanup for test 11/12
"$SERVERCTL" decref test-wrapper "$WRAPPER_SERVER_PID"
kill "$WRAPPER_SERVER_PID" 2>/dev/null || true

# Test 13: Combined workflow - realistic usage
info "Test 13: Combined workflow - check, start, incref, use, decref"

# Check if server exists
if "$SERVERCTL" check combined-test 2>/dev/null; then
	fail "Test 13: server should not exist initially"
fi

# Start server
"$SERVERCTL" start --timeout 5s combined-test sleep 3600 &
COMBINED_PID=$!
sleep 1

# Verify server is running
if ! "$SERVERCTL" check combined-test; then
	fail "Test 13: server should exist after start"
fi

# Get connection info
INFO_COMBINED=$("$SERVERCTL" info combined-test)
PID_COMBINED=$(echo "$INFO_COMBINED" | jq -r '.pid')

if [ "$PID_COMBINED" != "$COMBINED_PID" ]; then
	fail "Test 13: info returned wrong PID"
fi

# Increment refcount (simulate client connection)
"$SERVERCTL" incref combined-test --pid 55555 --type neovim

# Verify refcount
REFCOUNT_COMBINED=$("$SERVERCTL" info combined-test | jq -r '.refcount')
if [ "$REFCOUNT_COMBINED" != "1" ]; then
	fail "Test 13: refcount should be 1, got $REFCOUNT_COMBINED"
fi

# Decrement refcount (simulate client disconnect)
"$SERVERCTL" decref combined-test 55555

# Final decrement (triggers grace)
"$SERVERCTL" decref combined-test "$COMBINED_PID"

# Verify grace period
STATUS_COMBINED=$("$SERVERCTL" info combined-test | jq -r '.status')
if [ "$STATUS_COMBINED" != "grace" ]; then
	fail "Test 13: status should be 'grace', got '$STATUS_COMBINED'"
fi

# Cleanup
kill "$COMBINED_PID" 2>/dev/null || true

pass "Test 13: combined workflow completed successfully"

echo ""
echo "========================================="
echo -e "${GREEN}All tests passed!${NC}"
echo "========================================="
