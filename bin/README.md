# Shell Tools for shareserver

This directory contains shell tools that allow non-Neovim processes to participate in the shareserver lifecycle with **grace period support**.

## Overview

Two complementary tools provide complete server lifecycle management:

### `sharedserver` - Control Plane
State management and server operations (stateless CLI):
- **check** - Query server existence (no side effects)
- **info** - Get server details as JSON
- **start** - Launch server (blocking, becomes server process)
- **incref/decref** - Explicit refcount management
- **list** - Show all managed servers

### `process-wrapper` - Process Lifecycle
Transparent process wrapper with automatic cleanup:
- Wraps any command with auto-decref on exit
- Handles EXIT, TERM, INT signals
- Delegates state management to `sharedserver`

## Architecture

### Two-Lockfile Design

Two separate lockfiles provide clean state separation:

**`<name>.server.json`** - Server is alive
```json
{
    "pid": 12345,
    "server_name": "opencode",
    "started_at": 1708123456,
    "command": "opencode",
    "args": ["serve", "--port", "4097"],
    "grace_period": "30m",
    "watcher_pid": 12346
}
```

**`<name>.clients.json`** - Clients are using server
```json
{
    "refcount": 2,
    "server_pid": 12345,
    "clients": [
        {"pid": 12347, "type": "shell", "attached_at": 1708123460},
        {"pid": 12348, "type": "neovim", "attached_at": 1708123465}
    ]
}
```

When `clients.json` is deleted (refcount → 0), the watcher enters **grace period mode**.

### Grace Period State Machine

```
ACTIVE → GRACE → EXIT

ACTIVE:  clients.json exists, monitor server health every 5s
GRACE:   clients.json deleted, countdown timer running
         - If clients.json recreated → back to ACTIVE
         - If timer expires → kill server, cleanup server.json
EXIT:    Server died, cleanup and exit watcher
```

## Requirements

**Linux:** Built-in `flock` command
**macOS:** `brew install flock`
**Both:** `jq` for JSON manipulation

## sharedserver Usage

### check - Test if server exists

```bash
# Exit 0 if server exists, 1 otherwise (no side effects)
sharedserver check opencode
echo $?  # 0 or 1
```

### info - Get server details

```bash
# Returns JSON with server info
sharedserver info opencode
```

Output:
```json
{
    "pid": 12345,
    "server_name": "opencode",
    "command": "opencode",
    "args": ["serve", "--port", "4097"],
    "grace_period": "30m",
    "watcher_pid": 12346,
    "status": "active",
    "refcount": 2
}
```

**Status values:**
- `active` - Clients connected (clients.json exists)
- `grace` - No clients, grace period running (clients.json deleted)

### start - Launch server

```bash
# Start server with 30-minute grace period (blocking)
sharedserver start --timeout 30m opencode opencode serve --port 4097

# Custom shutdown signal (default: TERM)
sharedserver start --timeout 1h --signal INT myserver server-command

# No grace period (immediate shutdown when refcount → 0)
sharedserver start opencode opencode serve --port 4097
```

**Behavior:**
- Creates both `server.json` and `clients.json` (refcount=1)
- Forks detached watcher process
- **Execs into server** - this process becomes the server (blocking)
- Returns only if exec fails

### incref - Increment refcount

```bash
# Increment with caller's PID
sharedserver incref opencode

# Specify client PID and type
sharedserver incref opencode --pid $$ --type shell
sharedserver incref opencode --pid $(nvim --version | head -1) --type neovim
```

**Behavior:**
- If `clients.json` exists: increment refcount
- If `clients.json` doesn't exist (grace period): recreate with refcount=1, cancel shutdown
- Fails if server doesn't exist

### decref - Decrement refcount

```bash
# Decrement for specific client PID
sharedserver decref opencode 12345
```

**Behavior:**
- Decrements refcount in `clients.json`
- If refcount reaches 0: **deletes `clients.json`** (triggers grace period)
- Watcher detects deletion and starts grace timer

### list - Show all servers

```bash
# Returns JSON array of all servers
sharedserver list
```

Output:
```json
[
    {"pid": 12345, "server_name": "opencode", "status": "active", "refcount": 2, ...},
    {"pid": 12346, "server_name": "postgres", "status": "grace", "refcount": 0, ...}
]
```

## process-wrapper Usage

Wraps any command and automatically calls `sharedserver decref` on exit.

```bash
# Basic usage
process-wrapper <server-name> <client-pid> -- <command> [args...]

# Example: wrap a shell session
process-wrapper opencode $$ -- bash
# When bash exits, auto-decref

# Example: wrap a one-off command
process-wrapper opencode $$ -- curl http://localhost:4097/health
# When curl exits, auto-decref
```

**How it works:**
1. Sets up EXIT/TERM/INT signal handlers
2. Runs command as subprocess (NOT exec - trap must survive)
3. Waits for command to complete
4. On ANY exit: calls `sharedserver decref <server-name> <client-pid>`

## Usage Patterns

### Pattern 1: Manual Management (Shell Script)

```bash
#!/bin/bash

# Check if server exists
if ! sharedserver check opencode; then
    # Start server in background
    sharedserver start --timeout 30m opencode opencode serve --port 4097 &
    sleep 1  # Wait for startup
fi

# Get connection info
INFO=$(sharedserver info opencode)
PORT=$(echo "$INFO" | jq -r '.connection.port')  # Requires adding port to server.json

# Register as client
sharedserver incref opencode --pid $$ --type shell

# Use server
curl "http://localhost:$PORT/api/health"

# Unregister when done
sharedserver decref opencode $$
```

### Pattern 2: Auto-decref (Shell Script)

```bash
#!/bin/bash

# Check and start if needed
if ! sharedserver check opencode; then
    sharedserver start --timeout 30m opencode opencode serve --port 4097 &
    sleep 1
fi

# Register as client
sharedserver incref opencode --pid $$ --type shell

# Wrap script in process-wrapper for auto-cleanup
exec process-wrapper opencode $$ -- bash -c '
    # Use server...
    curl http://localhost:4097/api/health
    # More work...
'
# Auto-decref when bash exits
```

### Pattern 3: Neovim Integration (Manual)

```lua
-- lua/sharedserver/init.lua
local M = {}

local sharedserver = '/path/to/sharedserver'

function M.start_or_attach(name, config)
    -- Check if server exists
    local exists = vim.fn.system(sharedserver .. ' check ' .. name)
    
    if vim.v.shell_error == 0 then
        -- Server exists, increment refcount
        vim.fn.system(string.format('%s incref %s --pid %d --type neovim',
            sharedserver, name, vim.fn.getpid()))
        
        -- Get connection info
        local info = vim.fn.json_decode(
            vim.fn.system(sharedserver .. ' info ' .. name)
        )
        
        -- Setup auto-decref on VimLeave
        vim.api.nvim_create_autocmd('VimLeave', {
            callback = function()
                vim.fn.system(string.format('%s decref %s %d',
                    sharedserver, name, vim.fn.getpid()))
            end
        })
        
        return {attached = true, info = info}
    else
        -- Server doesn't exist, start it
        local cmd = {sharedserver, 'start', '--timeout', '30m', name, config.command}
        vim.list_extend(cmd, config.args)
        
        local job_id = vim.fn.jobstart(cmd, {
            detach = false,
            on_exit = function(_, exit_code)
                print(string.format('Server %s exited: %d', name, exit_code))
            end
        })
        
        return {attached = false, job_id = job_id}
    end
end

return M
```

### Pattern 4: Neovim + process-wrapper (Auto-decref)

```lua
local process_wrapper = '/path/to/process-wrapper'

function M.start_with_wrapper(name, config)
    if vim.fn.system(sharedserver .. ' check ' .. name) ~= 0 then
        -- Server doesn't exist, start with auto-decref wrapper
        local cmd = {
            process_wrapper,
            name,
            tostring(vim.fn.getpid()),
            '--',
            config.command
        }
        vim.list_extend(cmd, config.args)
        
        -- Auto-decref when nvim exits (wrapper handles it)
        return vim.fn.jobstart(cmd, {detach = false})
    else
        -- Server exists, just incref
        vim.fn.system(string.format('%s incref %s --pid %d --type neovim',
            sharedserver, name, vim.fn.getpid()))
    end
end
```

## Grace Period Examples

### Immediate shutdown (no grace period)

```bash
sharedserver start opencode opencode serve --port 4097
# When last client decrefs → server killed immediately
```

### 30-minute grace period

```bash
sharedserver start --timeout 30m opencode opencode serve --port 4097

# Last client disconnects
sharedserver decref opencode 12345
# → clients.json deleted, watcher starts 30-minute timer

# New client connects within 30 minutes
sharedserver incref opencode --pid $$
# → clients.json recreated, timer cancelled, back to ACTIVE
```

### Custom signal

```bash
# Send SIGINT instead of SIGTERM on timeout
sharedserver start --timeout 1h --signal INT myserver ./server.sh
```

## Files

- **sharedserver** - Control plane CLI (check, info, start, incref, decref, list)
- **process-wrapper** - Auto-decref process wrapper
- **test-sharedserver.sh** - Comprehensive test suite
- **sharedserver-wrapper** - (Deprecated) Old combined wrapper

## Environment Variables

```bash
# Override default lockfile directory
export SHAREDSERVER_LOCKDIR="/custom/path"

# Enable debug output
export SHAREDSERVER_DEBUG=1
```

Default lockdir: `$XDG_RUNTIME_DIR/sharedserver` or `/tmp/sharedserver`

## Testing

Run the test suite:

```bash
cd bin
bash test-sharedserver.sh
```

Tests cover:
- ✅ check/info commands
- ✅ start server with grace period
- ✅ incref/decref operations
- ✅ Grace period triggering and cancellation
- ✅ Watcher cleanup after grace expiry
- ✅ process-wrapper auto-decref on exit
- ✅ process-wrapper auto-decref on signal
- ✅ Combined workflows

## Design Rationale

### Why two tools instead of one?

**Separation of concerns:**
- `sharedserver` = pure state management (check, start, incref, decref)
- `process-wrapper` = lifecycle management (auto-decref on exit)

**Benefits:**
- Users can choose manual or automatic refcount management
- `sharedserver` has no side effects for read operations (check, info, list)
- `process-wrapper` is simple (~20 lines) and delegates to `sharedserver`
- Composable - can build higher-level tools on top

### Why two lockfiles?

**Prevents deadlock:**
- Watcher can read `server.json` while clients update `clients.json`
- Each lockfile has its own `.lock` file for atomic operations

**Clean state transitions:**
- File existence is atomic - unambiguous signal for grace period
- `clients.json` deletion triggers grace period (no parsing needed)
- `clients.json` recreation cancels grace period (instant detection)

### Why fork in process-wrapper instead of exec?

**Trap preservation:**
- Bash loses EXIT trap after exec
- Forking preserves trap → reliable auto-decref
- Slight overhead (extra process) vs reliability tradeoff

## Migration from Old Wrapper

Old single-command wrapper:
```bash
sharedserver-wrapper --timeout 30m opencode opencode serve --port 4097
```

New two-tool approach:
```bash
# Explicit control
sharedserver check opencode || sharedserver start --timeout 30m opencode opencode serve --port 4097 &
sharedserver incref opencode --pid $$
# ... use server ...
sharedserver decref opencode $$

# Or with auto-decref
sharedserver check opencode || sharedserver start --timeout 30m opencode opencode serve --port 4097 &
sharedserver incref opencode --pid $$
exec process-wrapper opencode $$ -- your-command
```

## Troubleshooting

### Server lockfile exists but process is dead

```bash
# sharedserver auto-detects stale locks
sharedserver check opencode  # Returns 1, cleans up stale files
```

### Grace period not triggering

Check watcher logs:
```bash
# Watcher outputs to stderr
# Look for: "Last client disconnected, entering grace period"
```

### Refcount mismatch

List clients:
```bash
sharedserver info opencode | jq '.clients'
```

Manually cleanup:
```bash
rm -f /tmp/sharedserver/opencode.clients.json  # Triggers grace
rm -f /tmp/sharedserver/opencode.server.json   # Force cleanup (kills watcher)
```

## Future Improvements

- [ ] Add connection info to `server.json` (port, socket path)
- [ ] Rust rewrite for cross-platform support (Windows)
- [ ] Configurable watcher poll interval
- [ ] Metrics (uptime, connection count, grace period hits)
- [ ] `sharedserver status` - detailed server status
- [ ] `sharedserver logs` - tail watcher logs
