#!/bin/bash
# test-wrapper-v2.sh - Test two-lockfile wrapper with grace period

set -euo pipefail

WRAPPER="$(dirname "$0")/sharedserver-wrapper"
LOCKDIR="/tmp/sharedserver-test-$$"
export SHAREDSERVER_LOCKDIR="$LOCKDIR"
export SHAREDSERVER_DEBUG=1

echo "=== Testing sharedserver-wrapper (Two-Lockfile + Grace Period) ==="
echo "Lockdir: $LOCKDIR"
echo

# Cleanup function
cleanup() {
	echo
	echo "=== Cleanup ==="
	pkill -f "sleep 3600" 2>/dev/null || true
	pkill -f "sharedserver-watcher" 2>/dev/null || true
	rm -rf "$LOCKDIR"
}
trap cleanup EXIT

mkdir -p "$LOCKDIR"

# Test 1: Start first server
echo "Test 1: First client starts server"
echo "-----------------------------------"
$WRAPPER --timeout 10s test-server sleep 3600 &
SERVER_PID=$!
sleep 2

echo "  Server PID: $SERVER_PID"

if ! ps -p $SERVER_PID >/dev/null 2>&1; then
	echo "  ✗ FAIL: Server not running"
	exit 1
fi

if ps -p $SERVER_PID -o comm= | grep -q "sleep"; then
	echo "  ✓ Server is 'sleep' (exec worked)"
else
	echo "  ✗ FAIL: Server is not 'sleep'"
	exit 1
fi

if [ -f "$LOCKDIR/test-server.server.json" ]; then
	echo "  ✓ server.json created"
	cat "$LOCKDIR/test-server.server.json" | jq -c .
else
	echo "  ✗ FAIL: server.json missing"
	exit 1
fi

if [ -f "$LOCKDIR/test-server.clients.json" ]; then
	echo "  ✓ clients.json created"
	cat "$LOCKDIR/test-server.clients.json" | jq -c .
else
	echo "  ✗ FAIL: clients.json missing"
	exit 1
fi

REFCOUNT=$(jq -r '.refcount' "$LOCKDIR/test-server.clients.json")
if [ "$REFCOUNT" = "1" ]; then
	echo "  ✓ Initial refcount=1"
else
	echo "  ✗ FAIL: refcount=$REFCOUNT, expected 1"
	exit 1
fi

# Test 2: Attach second client
echo
echo "Test 2: Second client attaches (should NOT start new server)"
echo "-------------------------------------------------------------"
OUTPUT=$($WRAPPER --timeout 10s test-server sleep 3600 2>&1)
EXIT_CODE=$?

echo "  Exit code: $EXIT_CODE"
echo "  Output: $OUTPUT"

if [ $EXIT_CODE -eq 0 ]; then
	echo "  ✓ Attach succeeded"
else
	echo "  ✗ FAIL: Attach failed with exit code $EXIT_CODE"
	exit 1
fi

if echo "$OUTPUT" | grep -q "attached to 'test-server'"; then
	echo "  ✓ Attach message printed"
else
	echo "  ✗ FAIL: Missing attach message"
fi

REFCOUNT=$(jq -r '.refcount' "$LOCKDIR/test-server.clients.json")
if [ "$REFCOUNT" = "2" ]; then
	echo "  ✓ Refcount incremented to 2"
else
	echo "  ✗ FAIL: refcount=$REFCOUNT, expected 2"
	exit 1
fi

# Test 3: Server still running (only one server process)
echo
echo "Test 3: Only one server process running"
echo "---------------------------------------"
SLEEP_COUNT=$(pgrep -f "sleep 3600" | wc -l | tr -d ' ')
if [ "$SLEEP_COUNT" = "1" ]; then
	echo "  ✓ Exactly one sleep process"
else
	echo "  ✗ FAIL: Found $SLEEP_COUNT sleep processes, expected 1"
	exit 1
fi

# Test 4: Manual refcount decrement (simulate client detach)
echo
echo "Test 4: Simulate client detach (decrement refcount)"
echo "---------------------------------------------------"
# Manually decrement refcount (this is what Neovim plugin would do)
jq '.refcount -= 1 | .clients = .clients[1:]' "$LOCKDIR/test-server.clients.json" >"$LOCKDIR/test-server.clients.json.tmp"
mv "$LOCKDIR/test-server.clients.json.tmp" "$LOCKDIR/test-server.clients.json"

REFCOUNT=$(jq -r '.refcount' "$LOCKDIR/test-server.clients.json")
if [ "$REFCOUNT" = "1" ]; then
	echo "  ✓ Refcount decremented to 1"
else
	echo "  ✗ FAIL: refcount=$REFCOUNT, expected 1"
	exit 1
fi

if ps -p $SERVER_PID >/dev/null 2>&1; then
	echo "  ✓ Server still running (refcount > 0)"
else
	echo "  ✗ FAIL: Server died prematurely"
	exit 1
fi

# Test 5: Trigger grace period (delete clients.json)
echo
echo "Test 5: Trigger grace period (delete clients.json)"
echo "---------------------------------------------------"
rm -f "$LOCKDIR/test-server.clients.json"
echo "  Deleted clients.json, watcher should enter grace mode"
sleep 2

if ps -p $SERVER_PID >/dev/null 2>&1; then
	echo "  ✓ Server still alive during grace period"
else
	echo "  ✗ FAIL: Server died (should be in grace period)"
	exit 1
fi

if [ -f "$LOCKDIR/test-server.server.json" ]; then
	echo "  ✓ server.json still exists"
else
	echo "  ✗ FAIL: server.json deleted prematurely"
	exit 1
fi

# Test 6: Cancel grace period (attach new client)
echo
echo "Test 6: Cancel grace period (new client attaches)"
echo "--------------------------------------------------"
OUTPUT=$($WRAPPER --timeout 10s test-server sleep 3600 2>&1)

if [ -f "$LOCKDIR/test-server.clients.json" ]; then
	echo "  ✓ clients.json recreated"
else
	echo "  ✗ FAIL: clients.json not recreated"
	exit 1
fi

REFCOUNT=$(jq -r '.refcount' "$LOCKDIR/test-server.clients.json")
if [ "$REFCOUNT" = "1" ]; then
	echo "  ✓ Refcount reset to 1"
else
	echo "  ✗ FAIL: refcount=$REFCOUNT, expected 1"
	exit 1
fi

# Test 7: Kill server and check watcher cleanup
echo
echo "Test 7: Kill server, watcher should clean up server.json"
echo "---------------------------------------------------------"
kill $SERVER_PID
sleep 3

if [ ! -f "$LOCKDIR/test-server.server.json" ]; then
	echo "  ✓ server.json removed by watcher"
else
	echo "  ✗ FAIL: server.json still exists"
	cat "$LOCKDIR/test-server.server.json"
	exit 1
fi

echo
echo "=== All tests passed! ==="
