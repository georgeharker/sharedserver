#!/bin/bash
# test-wrapper.sh - Test the sharedserver-wrapper

set -euo pipefail

WRAPPER="./sharedserver-wrapper"
LOCKDIR="/tmp/sharedserver-test-$$"
export SHAREDSERVER_LOCKDIR="$LOCKDIR"
export SHAREDSERVER_DEBUG=1

echo "=== Testing sharedserver-wrapper ==="
echo

# Cleanup function
cleanup() {
	echo
	echo "=== Cleaning up ==="
	rm -rf "$LOCKDIR"
	pkill -f "test-server-$$" || true
}
trap cleanup EXIT

# Test 1: Basic wrapper functionality
echo "Test 1: Launch a simple server through wrapper"
echo "-----------------------------------------------"

# Create a dummy server that just sleeps
cat >"/tmp/test-server-$$" <<'EOF'
#!/bin/bash
echo "Test server started with PID $$"
echo "Args: $@"
sleep 30
EOF
chmod +x "/tmp/test-server-$$"

# Launch through wrapper in background
echo "Launching server through wrapper..."
$WRAPPER test-server "/tmp/test-server-$$" arg1 arg2 &
WRAPPER_PID=$!

# Give it time to fork and exec
sleep 1

# Check what PID we got
echo "Wrapper PID was: $WRAPPER_PID"

# Check if process is actually the test server (not the wrapper)
if ps -p $WRAPPER_PID -o command= | grep -q "test-server"; then
	echo "✓ Process $WRAPPER_PID is the actual server (wrapper exec'd correctly)"
else
	echo "✗ Process $WRAPPER_PID is still the wrapper (exec failed)"
	exit 1
fi

# Check lockfile
LOCKFILE="$LOCKDIR/test-server.lock.json"
if [ -f "$LOCKFILE" ]; then
	echo "✓ Lockfile created: $LOCKFILE"
	echo "  Contents:"
	cat "$LOCKFILE" | jq .
else
	echo "✗ Lockfile not created"
	exit 1
fi

# Test 2: Refcount increment
echo
echo "Test 2: Launch second instance (should increment refcount)"
echo "-----------------------------------------------------------"

$WRAPPER test-server "/tmp/test-server-$$" arg1 arg2 &
WRAPPER_PID_2=$!

sleep 1

REFCOUNT=$(jq -r '.refcount' "$LOCKFILE")
echo "Refcount after second launch: $REFCOUNT"

if [ "$REFCOUNT" = "2" ]; then
	echo "✓ Refcount incremented correctly"
else
	echo "✗ Refcount is $REFCOUNT, expected 2"
	exit 1
fi

# Test 3: Watcher functionality
echo
echo "Test 3: Kill first instance, check refcount decrements"
echo "-------------------------------------------------------"

kill $WRAPPER_PID
echo "Killed first instance (PID $WRAPPER_PID)"

# Wait for watcher to decrement
sleep 2

REFCOUNT=$(jq -r '.refcount' "$LOCKFILE")
echo "Refcount after killing first instance: $REFCOUNT"

if [ "$REFCOUNT" = "1" ]; then
	echo "✓ Refcount decremented by watcher"
else
	echo "✗ Refcount is $REFCOUNT, expected 1"
	exit 1
fi

# Test 4: Lockfile cleanup
echo
echo "Test 4: Kill last instance, check lockfile removed"
echo "---------------------------------------------------"

kill $WRAPPER_PID_2
echo "Killed second instance (PID $WRAPPER_PID_2)"

# Wait for watcher to cleanup
sleep 2

if [ ! -f "$LOCKFILE" ]; then
	echo "✓ Lockfile removed when refcount reached 0"
else
	echo "✗ Lockfile still exists:"
	cat "$LOCKFILE"
	exit 1
fi

# Test 5: stdio passthrough
echo
echo "Test 5: Test stdio passthrough"
echo "-------------------------------"

# Create a server that echoes input
cat >"/tmp/test-echo-server-$$" <<'EOF'
#!/bin/bash
echo "Echo server ready"
while read line; do
    echo "ECHO: $line"
done
EOF
chmod +x "/tmp/test-echo-server-$$"

# Launch and test stdio
echo "test input" | $WRAPPER test-echo "/tmp/test-echo-server-$$" >"/tmp/test-output-$$" 2>&1 &
sleep 1

if grep -q "ECHO: test input" "/tmp/test-output-$$"; then
	echo "✓ stdio passthrough works"
else
	echo "✗ stdio passthrough failed"
	cat "/tmp/test-output-$$"
	exit 1
fi

echo
echo "=== All tests passed! ==="
