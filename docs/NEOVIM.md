# Neovim Integration — Detailed Guide

This document covers advanced Neovim integration topics. For a quick overview, see the
[Neovim Integration section](../README.md#neovim-integration) in the main README.

## Building from Source

If you prefer to build locally rather than `cargo install`:

```bash
cd sharedserver/rust
cargo build --release
# binary at rust/target/release/sharedserver
```

The plugin searches for the binary in this order:

1. `<plugin-dir>/rust/target/release/sharedserver`
2. `~/.cargo/bin/sharedserver` (via PATH after `cargo install`)
3. `~/.local/bin/sharedserver`
4. `/usr/local/bin/sharedserver`
5. `/opt/homebrew/bin/sharedserver`

### System-wide Installation

For use outside of Neovim (shell scripts, cron, other tools):

```bash
# Linux
sudo cp rust/target/release/sharedserver /usr/local/bin/

# macOS with Homebrew
sudo cp rust/target/release/sharedserver /opt/homebrew/bin/

# User-local (no sudo)
cp rust/target/release/sharedserver ~/.local/bin/
```

## Installation

### Using lazy.nvim

```lua
{
    "georgeharker/sharedserver",
    dependencies = { "nvim-lua/plenary.nvim" },
    build = "cargo install sharedserver --force",
    config = function()
        require("sharedserver").setup({
            servers = {
                chroma = {
                    command = "chroma",
                    args = { "run", "--path", "~/.local/share/chromadb" },
                    idle_timeout = "30m",
                },
            }
        })
    end
}
```

### Local Development

```lua
{
    dir = "~/Development/neovim-plugins/sharedserver",
    dependencies = { "nvim-lua/plenary.nvim" },
    build = "cargo install --path rust --force",
    config = function()
        require("sharedserver").setup({
            servers = {
                -- your servers
            }
        })
    end
}
```

## Health Monitoring

### Automatic Startup Check

The plugin monitors new servers for 3 seconds after startup. If a server exits
immediately, you get an error notification:

```
Error: sharedserver: 'myserver' died unexpectedly after start
```

This catches configuration issues that would otherwise fail silently (wrong
command, missing dependencies, port conflicts, etc.).

### Health Check Command

```vim
:checkhealth sharedserver
```

Verifies:
- sharedserver binary is installed and accessible
- Binary version information
- Lock directory is accessible and writable
- Plugin API is loaded correctly
- Current status of configured servers

See [DEBUGGING.md](./DEBUGGING.md) for comprehensive troubleshooting.

## Status UI

The `:ServerStatus` command opens a floating window showing all registered servers:

```
╭──────────────────────────── Shared Servers ─────────────────────────────╮
│ NAME                 STATUS       PID      REFS     UPTIME               │
│ ────────────────────────────────────────────────────────────────────────│
│ ● chroma             running      1234     2        2h 15m               │
│ ● redis              running      5678     1        45m 32s              │
│ ⏳ postgres           GRACE        9012     0        3h 22m               │
│ ○ myserver           stopped      -        -        - [lazy]             │
│                                                                           │
│ Press q or <Esc> to close                                                │
╰───────────────────────────────────────────────────────────────────────────╯
```

Status indicators:
- `●` Running with active clients (refcount > 0)
- `⏳` Grace period (refcount = 0, waiting for timeout)
- `○` Stopped

Use `:ServerStatus <name>` for a single server's detail view (includes full
command, arguments, and connection info).

## Detailed API Reference

### `setup(opts)`

Initialize and register servers:

```lua
require("sharedserver").setup({
    servers = {
        chroma = { command = "chroma", args = { "run" } },
    },
    commands = true,  -- create user commands (default)
    notify = {
        on_start = true,   -- notify on first start
        on_attach = false,  -- quiet on attach to existing
        on_stop = false,    -- quiet on stop
        on_error = true,    -- always show errors
    },
})
```

### `register(name, config)`

Add a server after initial setup:

```lua
require("sharedserver").register("newserver", {
    command = "myserver",
    args = { "--port", "9000" },
})
```

### `start(name)` / `stop(name)` / `restart(name)`

Manual server control. `start()` returns `true` on success:

```lua
local success = require("sharedserver").start("chroma")
```

### `stop_all()`

Stop all registered servers.

### `status(name)`

Returns a table with server state:

```lua
local s = require("sharedserver").status("chroma")
-- s.running   (boolean)
-- s.pid       (number or nil)
-- s.refcount  (number)
-- s.attached  (boolean)
-- s.lazy      (boolean)
```

### `status_all()`

Returns `{ [name] = status_table, ... }` for all servers.

### `list()`

Returns an array of registered server names.

## Manual sharedserver Usage from Lua

For custom workflows outside the plugin, you can call the binary directly:

```lua
local sharedserver = vim.fn.exepath("sharedserver")

-- Check if server exists
vim.fn.system({ sharedserver, "check", "myserver" })
if vim.v.shell_error == 0 then
    print("Server is running")
end

-- Get server info as JSON
local json = vim.fn.system({ sharedserver, "info", "myserver", "--json" })
local info = vim.fn.json_decode(json)
print("PID:", info.pid, "Status:", info.status, "Refcount:", info.refcount)

-- Manual attach/detach
vim.fn.system({
    sharedserver, "admin", "incref", "myserver",
    "--pid", tostring(vim.fn.getpid()),
})

vim.api.nvim_create_autocmd("VimLeave", {
    callback = function()
        vim.fn.system({
            sharedserver, "admin", "decref", "myserver",
            "--pid", tostring(vim.fn.getpid()),
        })
    end,
})
```

## Lazy Loading

The `lazy` option is useful for servers that:
- You want to share between projects but not start automatically
- Might be started by external tools or shell scripts
- Are expensive to start and should only run on demand

```lua
require("sharedserver").setup({
    servers = {
        expensive_db = {
            command = "heavy-database-server",
            lazy = true,  -- only attach if already running
        },
    }
})

-- Start manually when needed
vim.keymap.set("n", "<leader>sd", function()
    require("sharedserver").start("expensive_db")
end, { desc = "Start expensive database" })
```

On `VimEnter`, the plugin checks if a lazy server is already running and
attaches if so. It never starts a lazy server automatically.

## Notification Configuration

### Quiet mode (errors only)

```lua
notify = {
    on_start = false,
    on_attach = false,
    on_stop = false,
    on_error = true,
}
```

### Verbose mode

```lua
notify = {
    on_start = true,
    on_attach = true,
    on_stop = true,
    on_error = true,
}
```

### Per-server callbacks

Override default notifications with `on_start` / `on_exit`:

```lua
servers = {
    chroma = {
        command = "chroma",
        args = { "run" },
        on_start = function(pid)
            vim.notify("ChromaDB ready at http://localhost:8000")
        end,
        on_exit = function(exit_code)
            if exit_code ~= 0 then
                vim.notify("ChromaDB crashed!", vim.log.levels.WARN)
            end
        end,
    },
}
```

## Disabling Commands

```lua
require("sharedserver").setup({
    servers = { ... },
    commands = false,
})
```

When disabled, use the Lua API directly or create your own keymaps.
