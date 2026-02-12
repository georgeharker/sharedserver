# Recent Improvements - Health Check & Diagnostics

## Summary

Added comprehensive health check and debugging features to help users diagnose server startup issues and verify their setup is correct.

## New Features

### 1. `:checkhealth sharedserver` Command

Standard Neovim health check support for quick diagnostics:

```vim
:checkhealth sharedserver
```

**Checks:**
- ✓ sharedserver binary installation and location
- ✓ Binary version information
- ✓ Lock directory accessibility and permissions
- ✓ Plugin API is loaded correctly
- ✓ Current status of all configured servers
- ✓ Lists available features

**Files:**
- `lua/sharedserver/health.lua` - Health check implementation
- `tests/test-health-check-command.sh` - Verification tests

### 2. Automatic Server Death Detection

Monitors servers for 3 seconds after startup and notifies if they die:

```
Error: sharedserver: 'server_name' died unexpectedly after start
```

**Implementation:**
- `lua/sharedserver/init.lua:310-327` - `_schedule_health_check()` function
- Runs 3 seconds after server start
- Only for new starts (not when attaching to existing servers)
- Respects `notify.on_error` configuration

**Files:**
- `lua/sharedserver/init.lua` - Health check notification logic
- `tests/test-health-check.sh` - Verification tests

### 3. Comprehensive Debugging Documentation

Created extensive documentation for troubleshooting server issues:

**New Documentation:**
- `docs/DEBUGGING.md` - Complete troubleshooting guide
  - Health check usage
  - Server output architecture explanation
  - How to capture server output for debugging
  - Common issues and solutions
  - Example: debugging goog_ws startup issues
  - Advanced debugging techniques

- `docs/example-debug-config.lua` - Testing configurations
  - Server that exits immediately
  - Server that exits after delay
  - Ready to copy/paste for testing

**Updated Documentation:**
- `README.md` - Debugging section updated
  - Added health check command
  - Removed misleading debug_log option
  - Documented correct approach for output capture
  - Added health monitoring section

## Architecture Understanding

### Server Output Transparency

The `sharedserver` CLI is designed to be **transparent**:

```
Neovim Lua → sharedserver use NAME -- COMMAND
              ↓ (status to stdout, captured by Lua)
              watcher process (detached)
              ↓
              server process (inherits Neovim's stdio)
              ↓ (bypasses Lua)
              Terminal Output
```

**Key Points:**
- ✅ Status messages ("✓ Started server...") go to Lua
- ✅ Server output goes directly to terminal
- ❌ Server output cannot be captured from Lua (by design)

**Why:** Ensures tools expecting direct server output work correctly.

### Debugging Server Issues

**Recommended approach:**

1. Run `:checkhealth sharedserver` first
2. Redirect server output to file for debugging:
   ```lua
   args = { "-c", "myserver 2>&1 | tee /tmp/myserver.log" }
   ```
3. Monitor the health check notification (3 seconds)
4. Compare with manual execution in terminal
5. Check environment variables match

## Testing

All tests passing:

```bash
# Health notification tests (12s)
./tests/test-health-check.sh

# Health command tests (2s)
./tests/test-health-check-command.sh

# Comprehensive suite (93s)
./tests/test-monitoring-and-recovery.sh

# Rust integration tests (76s)
cargo test --manifest-path rust/Cargo.toml --test integration_tests
```

## Commits

1. **9bf4d8b**: feat: add health check for server death detection and comprehensive debugging docs
   - Health notification system
   - DEBUGGING.md guide
   - README updates
   - Test suite

2. **98e3e23**: feat: add :checkhealth sharedserver support
   - health.lua module
   - README :checkhealth documentation
   - Test suite

## User Impact

### Before
- Servers died silently with no feedback
- No easy way to verify setup
- Unclear how to debug server issues
- No guidance on capturing output

### After
- ✅ Automatic notification when servers die (3s)
- ✅ `:checkhealth sharedserver` verifies setup
- ✅ Comprehensive debugging guide
- ✅ Clear examples and troubleshooting steps
- ✅ Understanding of output architecture

## Next Steps for Users

To diagnose `goog_ws` (or any server) startup issues:

1. **Verify setup:**
   ```vim
   :checkhealth sharedserver
   ```

2. **Redirect output:**
   ```lua
   goog_ws = {
       command = "bash",
       args = { "-c", "uvx workspace-mcp --transport streamable-http 2>&1 | tee /tmp/goog_ws.log" },
       -- ... rest of config
   }
   ```

3. **Start and monitor:**
   ```vim
   :ServerStart goog_ws
   ```
   - Wait for health notification (3s)
   - Check `/tmp/goog_ws.log`

4. **Compare with manual run:**
   ```bash
   # Copy environment from config
   env WORKSPACE_MCP_PORT=8002 \
       GOOGLE_CLIENT_SECRET_PATH=~/.cache/secrets/$USER.gcp-oauth.keys.json \
       uvx workspace-mcp --transport streamable-http
   ```

## Files Changed

```
lua/sharedserver/
├── init.lua              # Added _schedule_health_check()
└── health.lua            # NEW: :checkhealth implementation

docs/
├── DEBUGGING.md          # NEW: Comprehensive guide
├── example-debug-config.lua  # NEW: Test configurations
└── CHANGELOG.md          # NEW: This file

tests/
├── test-health-check.sh           # NEW: Health notification tests
└── test-health-check-command.sh   # NEW: :checkhealth tests

README.md                 # Updated debugging section
```

## Related Issues

- ✅ Server death goes unnoticed - FIXED
- ✅ No way to verify setup - FIXED (`:checkhealth`)
- ✅ Unclear how to debug - FIXED (DEBUGGING.md)
- ✅ Output capture confusion - FIXED (documentation)

---

**All functionality tested and working correctly.**
