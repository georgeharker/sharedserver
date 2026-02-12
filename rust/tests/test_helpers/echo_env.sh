#!/bin/bash
# Test helper: Echo environment variables and stay alive
echo "TEST_VAR=$TEST_VAR"
echo "ANOTHER_VAR=$ANOTHER_VAR"
echo "Started successfully"

# Stay alive for testing
sleep 30
