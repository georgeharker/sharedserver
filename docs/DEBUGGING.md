# Debugging Server Startup Issues

## Overview

If a server exits immediately after starting, the plugin now includes a **health check** that automatically notifies you within 3 seconds.

## Health Check Command

Before debugging issues, verify your setup with:

```vim
:checkhealth sharedserver
```

This checks:
- ✓ sharedserver binary is installed and in PATH
- ✓ Binary version information
- ✓ Lock directory is accessible and writable
- ✓ Plugin is loaded correctly
- ✓ Status of configured servers

## Health Check Notification

When a server starts successfully but dies within 3 seconds, you'll see:

```
Error: sharedserver: 'server_name' died unexpectedly after start
```

This notification respects your `notify.on_error` config setting and helps catch configuration issues that would otherwise fail silently.

## Understanding Server Output

**Important:** The `sharedserver` CLI is designed to be transparent - your server's stdout/stderr go directly to Neovim's terminal, not through the plugin. This means:

- ✅ **sharedserver status messages** ("✓ Started server...") are captured by the plugin
- ✅ **Server output** goes directly to your terminal/Neovim UI
- ❌ **Server output cannot be captured** from the Lua plugin layer

This is by design - it ensures that tools expecting to read the server's output directly will work correctly.

## Debugging Approach

### 1. Check the Health Check Notification

After running `:ServerStart server_name`, wait 3 seconds. If the server dies, you'll get a notification.

### 2. Redirect Server Output

If you need to capture your server's output for debugging, redirect it in your command:

```lua
require("sharedserver").setup({
    servers = {
        myserver = {
            command = "bash",
            args = { "-c", "myserver 2>&1 | tee /tmp/myserver.log" },
            -- or just redirect to file:
            -- args = { "-c", "myserver > /tmp/myserver.log 2>&1" },
        },
    },
})
```

Then check `/tmp/myserver.log` for server output.

### 3. Check Server Status

Use `:ServerStatus` to see if the server is running:

```vim
:ServerStatus myserver
```

This shows:
- Running state (Active/Grace/Stopped)
- PID if running
- Refcount
- Uptime

### 4. Run Manually in Shell

Compare running the server manually vs through sharedserver:

```bash
# Manual (what you expect to work)
env VAR1=value1 VAR2=value2 myserver --args

# Through sharedserver (what the plugin does)
sharedserver use --grace-period 30m \
    --env VAR1=value1 \
    --env VAR2=value2 \
    myserver -- myserver --args
```

If manual works but sharedserver doesn't, check:
- Environment variables
- Working directory
- File descriptors (stdin/stdout/stderr)
- TTY expectations

## Common Issues

### Server Exits Immediately

**Symptoms**:
- Health check notification appears after 3 seconds
- `:ServerStatus` shows "stopped"

**Possible Causes**:
1. Missing environment variables
2. Configuration file not found
3. Port already in use
4. Missing dependencies
5. Server expects TTY but runs in background

**Debug Steps**:
1. Add output redirection to capture server logs
2. Run the server manually to verify it works
3. Check environment variables match
4. Verify file paths are absolute, not relative
5. Check if server requires interactive TTY

### Command Not Found

**Symptoms**:
- Error: "command 'xxx' is not executable"

**Solution**:
- Ensure command is in PATH
- Use absolute path: `command = "/usr/local/bin/xxx"`

### Environment Variables Not Set

**Symptoms**:
- Server starts but can't find config files
- Authentication errors

**Solution**:
```lua
servers = {
    myserver = {
        command = "myserver",
        env = {
            CONFIG_PATH = vim.fn.expand("$HOME") .. "/.config/myserver",
            API_KEY = "your-key",
        },
    },
}
```

### Port Already in Use

**Symptoms**:
- Server fails to bind to port
- "Address already in use" errors

**Solution**:
1. Check if server is already running: `:ServerStatus`
2. Stop it: `:ServerStop myserver`
3. Or check manually: `lsof -i :PORT`

## Example: Debugging goog_ws

Let's say `goog_ws` works manually but fails through Neovim:

**1. Add output redirection:**
```lua
require("sharedserver").setup({
    servers = {
        goog_ws = {
            command = "bash",
            args = {
                "-c",
                "uvx workspace-mcp --transport streamable-http 2>&1 | tee /tmp/goog_ws.log"
            },
            env = {
                GOOGLE_CLIENT_SECRET_PATH = vim.fn.expand("$HOME") .. "/.cache/secrets/" .. vim.fn.expand("$USER") .. ".gcp-oauth.keys.json",
                WORKSPACE_MCP_PORT = "8002",
            },
            lazy = true,
        },
    },
})
```

**2. Start the server:**
```vim
:ServerStart goog_ws
```

**3. Check for health notification** (appears after 3 seconds if server dies)

**4. Examine server output:**
```bash
tail -f /tmp/goog_ws.log
```

**5. Compare with manual execution:**
```bash
env WORKSPACE_MCP_PORT=8002 \
    GOOGLE_CLIENT_SECRET_PATH=~/.cache/secrets/$USER.gcp-oauth.keys.json \
    uvx workspace-mcp --transport streamable-http
```

**6. Look for differences:**
- Does manual version show different output?
- Are there missing environment variables?
- Does the server expect stdin input?
- Does it require a TTY?

## Testing the Health Check

Create a test server that exits after a delay:

```lua
require("sharedserver").setup({
    servers = {
        test_delayed_exit = {
            command = "bash",
            args = { "-c", "echo 'Starting...'; sleep 2; echo 'Exiting...'; exit 1" },
        },
    },
})
```

Then start it:
```vim
:ServerStart test_delayed_exit
```

You should see:
1. Success notification immediately
2. "Starting..." in your terminal
3. Error notification after 3 seconds: "test_delayed_exit died unexpectedly after start"
4. "Exiting..." may or may not appear depending on timing

## Advanced Debugging

### Capture Environment Differences

In your shell:
```bash
env | sort > /tmp/shell-env.txt
```

In Neovim:
```vim
:lua vim.fn.writefile(vim.fn.sort(vim.tbl_keys(vim.fn.environ())), "/tmp/nvim-env.txt")
```

Compare:
```bash
diff /tmp/shell-env.txt /tmp/nvim-env.txt
```

### Check Process State

While server is running:
```bash
# Check if process exists
ps aux | grep myserver

# Check open files/ports
lsof -p <PID>

# Check environment of running process
cat /proc/<PID>/environ | tr '\0' '\n'  # Linux
ps eww <PID>  # macOS
```

### Wrap Server with Debug Script

Create a wrapper that logs everything:

```bash
#!/bin/bash
# /tmp/debug-wrapper.sh
echo "=== Starting server at $(date) ===" >> /tmp/server-debug.log
echo "PWD: $(pwd)" >> /tmp/server-debug.log
echo "Environment:" >> /tmp/server-debug.log
env | sort >> /tmp/server-debug.log
echo "=== Executing command ===" >> /tmp/server-debug.log
exec "$@" 2>&1 | tee -a /tmp/server-debug.log
```

Then use it:
```lua
servers = {
    myserver = {
        command = "/tmp/debug-wrapper.sh",
        args = { "myserver", "--port", "8080" },
    },
}
```

### Admin Commands for Troubleshooting

The `sharedserver` CLI provides admin commands for diagnosing and fixing issues:

**Health Check and Cleanup:**
```bash
# Check all servers for issues (validates processes, refcounts, cleans stale lockfiles)
sharedserver admin doctor

# Check specific server
sharedserver admin doctor myserver
```

The `doctor` command validates:
- Server and watcher processes are actually alive
- All client PIDs are valid
- Refcount matches actual client count
- State constraints are valid (Active has clients, Grace has none)
- Automatically removes stale lockfiles for stopped servers

**View Debug Logs:**
```bash
# Show invocation history for troubleshooting
sharedserver admin debug myserver
```

**Force Kill Unresponsive Server:**
```bash
# Send SIGKILL and clean up all state (use when server won't stop normally)
sharedserver admin kill myserver
```

**Emergency Stop:**
```bash
# Send SIGTERM (or SIGKILL with --force)
sharedserver admin stop myserver --force
```

## Summary

The key points for debugging:

1. ✅ **Health check** automatically notifies you when servers die
2. ✅ **Use `admin doctor`** to validate server state and clean up issues
3. ✅ **Server output** goes to terminal - redirect it yourself if needed
4. ✅ **Compare** manual vs sharedserver execution to find differences
5. ✅ **Check** environment, working directory, and file descriptors
6. ❌ **Don't expect** to capture server output from Lua - it's transparent by design
