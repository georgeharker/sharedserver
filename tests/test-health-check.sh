#!/usr/bin/env bash
# Test script to verify health check notification when server dies immediately

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
BINARY="$PROJECT_ROOT/rust/target/release/sharedserver"
TEST_SERVER_NAME="test_immediate_exit"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Setup test lockdir
export SHAREDSERVER_LOCKDIR="/tmp/sharedserver-test-health-$$"
mkdir -p "$SHAREDSERVER_LOCKDIR"

echo "=== Testing Health Check for Immediate Server Death ==="
echo "Lock directory: $SHAREDSERVER_LOCKDIR"
echo

cleanup() {
	echo
	echo "=== Cleanup ==="
	"$BINARY" admin stop "$TEST_SERVER_NAME" 2>/dev/null || true
	sleep 1
	rm -rf "$SHAREDSERVER_LOCKDIR"
	echo -e "${GREEN}✓${NC} Cleanup complete"
}

trap cleanup EXIT

# Test: Server that exits immediately
echo "Test: Server exits immediately after start"
echo "Starting server with 'bash -c \"exit 1\"'..."

# Start server that exits immediately
"$BINARY" use "$TEST_SERVER_NAME" -- bash -c "exit 1" &
USE_PID=$!

# Wait for use command to complete
wait $USE_PID 2>/dev/null || true

sleep 1

# Check if server lock file exists
if [ -f "$SHAREDSERVER_LOCKDIR/$TEST_SERVER_NAME.server.json" ]; then
	echo -e "${RED}✗${NC} Server lock file still exists (server didn't exit or timeout didn't work)"
	exit 1
fi

# Check if clients lock file exists
if [ -f "$SHAREDSERVER_LOCKDIR/$TEST_SERVER_NAME.clients.json" ]; then
	echo -e "${RED}✗${NC} Clients lock file still exists (watcher didn't clean up)"
	exit 1
fi

echo -e "${GREEN}✓${NC} Lock files cleaned up correctly"

# Now test with a server that exits after a delay (to test health check timing)
echo
echo "Test: Server exits after 2 seconds (should trigger health check)"
TEST_SERVER_NAME="test_delayed_exit"

echo "Starting server with 'bash -c \"sleep 2; exit 1\"'..."
"$BINARY" use "$TEST_SERVER_NAME" -- bash -c "sleep 2; exit 1" &
USE_PID=$!

# Wait for use command to complete (server should start successfully)
wait $USE_PID 2>/dev/null || true

sleep 1

# Verify server is running
if ! "$BINARY" check "$TEST_SERVER_NAME"; then
	echo -e "${RED}✗${NC} Server not running after start"
	exit 1
fi

echo -e "${GREEN}✓${NC} Server started successfully"
echo "Waiting for server to exit (2s) + watcher poll (5s) + buffer (2s)..."

# Wait for server to die and watcher to detect it
sleep 9

# Check that server is no longer running
if "$BINARY" check "$TEST_SERVER_NAME" 2>/dev/null; then
	echo -e "${RED}✗${NC} Server still running after expected exit"
	exit 1
fi

echo -e "${GREEN}✓${NC} Server exited as expected"

# Check if lock files were cleaned up
if [ -f "$SHAREDSERVER_LOCKDIR/$TEST_SERVER_NAME.server.json" ]; then
	echo -e "${RED}✗${NC} Server lock file still exists after death"
	exit 1
fi

if [ -f "$SHAREDSERVER_LOCKDIR/$TEST_SERVER_NAME.clients.json" ]; then
	echo -e "${RED}✗${NC} Clients lock file still exists after death"
	exit 1
fi

echo -e "${GREEN}✓${NC} Lock files cleaned up after death"

echo
echo -e "${GREEN}=== All Health Check Tests Passed ===${NC}"
echo
echo "Note: The Lua health check notification can only be tested in Neovim."
echo "To test manually:"
echo "1. Add 'debug_log = \"/tmp/sharedserver-debug.log\"' to your server config"
echo "2. Start a server that exits immediately"
echo "3. You should see an error notification after 3 seconds"
