#!/usr/bin/env bash
# Test helper that echoes environment variables and exits

# Print any TEST_VAR or ANOTHER_VAR if set
if [ -n "$TEST_VAR" ]; then
    echo "TEST_VAR=$TEST_VAR"
fi

if [ -n "$ANOTHER_VAR" ]; then
    echo "ANOTHER_VAR=$ANOTHER_VAR"
fi

# Exit successfully after a brief moment
sleep 0.5
exit 0
