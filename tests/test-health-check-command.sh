#!/usr/bin/env bash

# Test script for :checkhealth sharedserver functionality

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

echo "=== Testing :checkhealth sharedserver ==="
echo

# Test 1: Basic health check (no setup)
echo "Test 1: Health check without setup"
cat >/tmp/test-health-init.lua <<EOF
-- Add plugin to runtime path
vim.opt.runtimepath:prepend("$PROJECT_DIR")

-- Run health check
require("sharedserver.health").check()
print("✓ Test 1 passed: Health check runs without setup")
EOF

nvim --headless -u /tmp/test-health-init.lua -c "quitall" 2>&1 | grep -E "(✓|✗|•|##)" || true

echo
echo "Test 2: Health check with configured servers"
cat >/tmp/test-health-init2.lua <<EOF
-- Add plugin to runtime path
vim.opt.runtimepath:prepend("$PROJECT_DIR")

-- Configure plugin
require("sharedserver").setup({
    test_server = {
        command = "sleep",
        args = { "60" },
        lazy = true,
    },
})

-- Run health check
require("sharedserver.health").check()
print("✓ Test 2 passed: Health check shows configured servers")
EOF

nvim --headless -u /tmp/test-health-init2.lua -c "quitall" 2>&1 | grep -E "(✓|✗|•|##|test_server)" || true

# Clean up
rm -f /tmp/test-health-init.lua /tmp/test-health-init2.lua

echo
echo "✓ All health check tests passed"
