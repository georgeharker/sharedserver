# sharedserver.nvim

A generic Neovim plugin for keeping server processes alive across multiple Neovim instances using reference counting.

## Features

- **Multiple server management**: Manage multiple servers simultaneously with named configurations
- **Shared server management**: One server process shared across multiple Neovim instances
- **Reference counting**: Servers stay alive as long as at least one Neovim instance is attached
- **Lazy loading**: Optionally only attach to servers if they're already running
- **Automatic lifecycle**: Servers start on VimEnter, stop on VimLeave
- **Manual control**: Start, stop, restart, and query status of named servers
- **Flexible configuration**: Configure command, args, working directory, and pidfile location per server
- **Status monitoring**: Check server status and get PID/refcount info
- **Built-in commands**: User commands are automatically created for easy server management

## Requirements

- Neovim 0.5+
- [plenary.nvim](https://github.com/nvim-lua/plenary.nvim)

## Installation

Using [lazy.nvim](https://github.com/folke/lazy.nvim):

```lua
{
    "yourusername/sharedserver.nvim",
    dependencies = { "nvim-lua/plenary.nvim" },
    config = function()
        require("sharedserver").setup({
            chroma = {
                command = "chroma",
                args = { "run", "--path", "~/.local/share/chromadb" },
            },
            redis = {
                command = "redis-server",
                lazy = true,  -- Only attach if already running
            }
        })
    end
}
```

Or for local development:

```lua
{
    dir = "~/Development/neovim-plugins/sharedserver.nvim",
    dependencies = { "nvim-lua/plenary.nvim" },
    config = function()
        require("sharedserver").setup({
            -- your config
        })
    end
}
```

See [EXAMPLES.md](./EXAMPLES.md) for more configuration examples and usage patterns.

## Configuration

### Setup Options

The `setup()` function accepts two parameters:

```lua
require("sharedserver").setup(servers, opts)
```

- **servers**: A table of server configurations (see below)
- **opts**: Optional configuration table with the following options:
  - **commands** (default: `true`): Whether to automatically create user commands

### Multiple Servers (Recommended)

```lua
require("sharedserver").setup({
    -- Server name as key
    chroma = {
        command = "chroma",
        args = { "run", "--path", "~/.local/share/chromadb" },
        lazy = false,  -- Start immediately (default)
    },
    redis = {
        command = "redis-server",
        args = { "--port", "6379" },
        lazy = true,  -- Only attach if already running, don't start
    },
    webserver = {
        command = "python",
        args = { "-m", "http.server", "8080" },
        working_dir = vim.fn.getcwd(),
        on_start = function(pid)
            vim.notify("Web server started: http://localhost:8080")
        end,
    },
})
```

### Disable Commands

To disable automatic command creation:

```lua
require("sharedserver").setup({
    chroma = {
        command = "chroma",
        args = { "run", "--path", "~/.local/share/chromadb" },
    },
}, { commands = false })
```

### Single Server (Backward Compatible)

```lua
require("sharedserver").setup({
    name = "myserver",  -- Optional, defaults to "default"
    command = "chroma",
    args = { "run", "--path", "~/.local/share/chromadb" },
})
```

### Server Configuration Options

For each server:

- **command** (required): Command to execute (can be full path or command in PATH)
- **args** (optional, default: `{}`): Arguments to pass to the command
- **lazy** (optional, default: `false`): If `true`, only attach to server if already running, don't start a new one
- **pidfile** (optional, default: `stdpath("cache")/<command-name>.lock.json`): PID file location
- **working_dir** (optional, default: `nil`): Working directory for the server
- **on_start** (optional): Callback function called with `(pid)` when server starts
- **on_exit** (optional): Callback function called with `(exit_code)` when server exits

### Lazy Loading

The `lazy` option is useful for servers that:
- You want to share between projects but not start automatically
- Might be started by external tools
- Are expensive to start and should only run when needed

```lua
require("sharedserver").setup({
    expensive_db = {
        command = "heavy-database-server",
        lazy = true,  -- Only attach if already running
    },
})

-- Later, manually start when needed:
vim.keymap.set("n", "<leader>sd", function()
    require("sharedserver").start("expensive_db")
end, { desc = "Start expensive database" })
```

## API

All API functions are available through `require("sharedserver")`:

```lua
local sharedserver = require("sharedserver")

-- Setup servers and optionally enable commands
sharedserver.setup(servers, opts)

-- Register a server after initial setup
sharedserver.register(name, config)

-- Manually control servers
sharedserver.start(name)
sharedserver.stop(name)
sharedserver.restart(name)
sharedserver.stop_all()

-- Query server status
local status = sharedserver.status(name)
local all_statuses = sharedserver.status_all()
local server_names = sharedserver.list()
```

## Commands

When commands are enabled (default), the following user commands are automatically created:

## Commands

When commands are enabled (default), the following user commands are automatically created:

- `:ServerStart <name>` - Start a named server
- `:ServerStop <name>` - Stop a named server
- `:ServerRestart <name>` - Restart a named server
- `:ServerStatus [name]` - Show server status (all servers if no name given)
- `:ServerList` - List all registered servers
- `:ServerStopAll` - Stop all servers

To disable command creation, pass `{ commands = false }` to `setup()`.

## Detailed API Reference

### `setup(servers, opts)`

### `setup(servers, opts)`

Initialize and register servers. Accepts:
- **servers**: A table of named server configurations, or a single server configuration (backward compatible)
- **opts**: Optional table with options:
  - `commands` (default: `true`): Whether to create user commands

Example:
```lua
require("sharedserver").setup({
    chroma = { command = "chroma", args = { "run" } },
}, { commands = true })
```

### `register(name, config)`

Register a new server after initial setup.

```lua
require("sharedserver").register("newserver", {
    command = "myserver",
    args = { "--port", "9000" },
})
```

### `start(name)`

Manually start a named server. Returns `true` on success, `false` on failure.

```lua
local success = require("sharedserver").start("chroma")
```

### `stop(name)`

Manually stop a named server.

```lua
require("sharedserver").stop("chroma")
```

### `stop_all()`

Stop all registered servers.

```lua
require("sharedserver").stop_all()
```

### `restart(name)`

Restart a named server.

```lua
require("sharedserver").restart("chroma")
```

### `status(name)`

Get the status of a named server.

```lua
local status = require("sharedserver").status("chroma")
if status.running then
    print("Server running with PID: " .. status.pid)
    print("Reference count: " .. status.refcount)
    print("Attached: " .. tostring(status.attached))
    print("Lazy: " .. tostring(status.lazy))
else
    print("Server not running")
end
```

### `status_all()`

Get status of all registered servers.

```lua
local statuses = require("sharedserver").status_all()
for name, status in pairs(statuses) do
    print(name, status.running)
end
```

### `list()`

Get a list of all registered server names.

```lua
local servers = require("sharedserver").list()
for _, name in ipairs(servers) do
    print("Registered server: " .. name)
end
```

## How It Works

1. When Neovim starts, the plugin checks for existing lock files for each registered server
2. For non-lazy servers:
   - If a lock file exists, it increments the reference count (another Neovim instance is using the server)
   - If no lock file exists, it starts the server and creates a lock file with refcount=1
3. For lazy servers:
   - If a lock file exists, it increments the reference count (attaches to existing server)
   - If no lock file exists, it does nothing (waits for manual start)
4. When Neovim exits, it decrements the reference count for all attached servers
5. When refcount reaches 0, the server is terminated and the lock file is removed

## Use Cases

- **Database servers**: ChromaDB, Redis, PostgreSQL for development
- **Language servers**: Custom LSP servers, code analysis tools
- **Development servers**: HTTP servers, WebSocket servers
- **Background processes**: File watchers, sync daemons
- **Expensive services**: Large ML models, heavy databases (use `lazy = true`)

For detailed configuration examples and usage patterns, see [EXAMPLES.md](./EXAMPLES.md).

## License

MIT
