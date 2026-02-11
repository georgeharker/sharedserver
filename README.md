# shareserver

[![crates][crates]](https://crates.io/crates/sharedserver)

A generic Neovim plugin for keeping server processes alive across multiple Neovim instances using reference counting with automatic grace period support.

## Features

- **Multiple server management**: Manage multiple servers simultaneously with named configurations
- **Shared server management**: One server process shared across multiple Neovim instances
- **Reference counting**: Servers stay alive as long as at least one Neovim instance is attached
- **Grace period support**: Servers can stay alive for a configurable period after all clients disconnect
- **Lazy loading**: Optionally only attach to servers if they're already running
- **Automatic lifecycle**: Servers start on VimEnter, stop on VimLeave with grace period
- **Manual control**: Start, stop, restart, and query status of named servers
- **Flexible configuration**: Configure command, args, working directory, and idle timeout per server
- **Status monitoring**: Check server status and get PID/refcount/grace period info
- **Built-in commands**: User commands are automatically created for easy server management
- **Robust state management**: Two-lockfile architecture with stale lock cleanup and atomic operations

## Why Use SharedServer?

SharedServer solves a common problem: **efficiently managing long-running service processes across multiple instances of your editor or between different tools**.

### Key Benefits

**🔄 Reuse Servers Between Processes**
- Start a service once (e.g., ChromaDB, Redis, development server)
- Share it across multiple Neovim instances
- Automatic reference counting ensures it stays alive as long as any instance needs it

**⏱️ Smart Lifecycle Management**
- Servers automatically shut down when no longer needed
- Configurable grace periods keep services warm for quick restarts
- Dead client detection prevents resource leaks from crashed processes

**🎯 Built for Neovim**
- Zero-configuration lifecycle hooks (`VimEnter`/`VimLeave`)
- Automatic attachment to existing servers
- Rich status monitoring with `:ServerStatus`
- Optional lazy loading for expensive services

**🔧 Beyond Neovim**
- Shell script integration via `sharedserver` CLI
- Use it as a service wrapper in any program
- Replace manual process management or shell scripts
- Works standalone or integrated with your editor

### Example Use Cases

**Replace manual server management:**
```bash
# Instead of this fragile pattern:
pkill -f "python -m http.server" || true
python -m http.server 8000 &
# Do your work...
pkill -f "python -m http.server"

# Use sharedserver:
sharedserver use webserver -- python -m http.server 8000
# Do your work...
sharedserver unuse webserver  # Server stays alive if other clients need it
```

**Share expensive services:**
```lua
-- ChromaDB takes 10s to start, costs 2GB RAM
-- Without sharedserver: Every Neovim instance starts its own (slow, wasteful)
-- With sharedserver: One instance shared by all, 30min grace period after last editor closes
require("sharedserver").setup({
    chroma = {
        command = "chroma",
        args = { "run", "--path", "~/.local/share/chromadb" },
        idle_timeout = "30m",
    }
})
```

**Lazy-load heavy services:**
```lua
-- Don't start expensive ML inference server until explicitly needed
require("sharedserver").setup({
    llm_server = {
        command = "ollama",
        args = { "serve" },
        lazy = true,  -- Only attach if already running
    }
})
-- Start manually when needed: :ServerStart llm_server
```

### Why Not Just Use systemd/launchd?

System services (systemd, launchd, etc.) are great for **always-on** infrastructure, but SharedServer is designed for **on-demand development services**:

| System Service | SharedServer |
|----------------|--------------|
| Always running, even when unused | Starts when needed, stops when done |
| Requires root/system configuration | User-space, no sudo needed |
| Global configuration files | Per-project configuration in your editor config |
| Manual start/stop commands | Automatic lifecycle tied to your workflow |
| One instance system-wide | Multiple isolated instances per project possible |

**When to use system services:**
- Production servers
- Always-on infrastructure (databases, web servers)
- Services needed by multiple users

**When to use SharedServer:**
- Development databases (ChromaDB, Redis for testing)
- Project-specific dev servers
- Heavy services you only need occasionally
- Services tied to your editor workflow
- When you want automatic cleanup after work

## Requirements

- Neovim 0.5+
- [plenary.nvim](https://github.com/nvim-lua/plenary.nvim)
- Rust toolchain (for building sharedserver) - install from [rustup.rs](https://rustup.rs)
- **macOS**: Built-in `flock` via Rust (no additional dependencies)
- **Linux**: Built-in `flock` via Rust (no additional dependencies)

## Building from Source

The plugin requires building the Rust-based `sharedserver` binary:

```bash
cd shareserver/rust
cargo build --release
```

The binary will be available at `rust/target/release/sharedserver`. The plugin will automatically find it in this location.

### Installation via Cargo (Recommended)

The simplest way to install `sharedserver` for use with the Neovim plugin:

```bash
cargo install sharedserver
```

Or install from the repository:

```bash
# From repository root
cargo install --path rust
```

Both methods install the binary to `~/.cargo/bin/sharedserver`, which should be in your PATH. This is sufficient for Neovim plugin usage.

### System-wide Installation (Optional)

Install system-wide if you want to use `sharedserver` **outside of Neovim** (e.g., in shell scripts, cron jobs, or other programs):

```bash
# Linux
sudo cp rust/target/release/sharedserver /usr/local/bin/

# macOS with Homebrew
sudo cp rust/target/release/sharedserver /opt/homebrew/bin/

# User-local installation (no sudo)
cp rust/target/release/sharedserver ~/.local/bin/
```

**Why system-wide?**
- Use `sharedserver` CLI from any shell script
- Integrate with systemd, cron, or other system services
- Share servers between different tools (not just Neovim)

The plugin searches for `sharedserver` in the following order:
1. `<plugin-dir>/rust/target/release/sharedserver` (default after build)
2. `~/.local/bin/sharedserver`
3. `/usr/local/bin/sharedserver`
4. `/opt/homebrew/bin/sharedserver`

### Shell Completions

Generate shell completion scripts for `sharedserver`:

```bash
# Bash
sharedserver completion bash > ~/.local/share/bash-completion/completions/sharedserver

# Zsh
sharedserver completion zsh > ~/.zsh/completions/_sharedserver
# Then add to ~/.zshrc: fpath=(~/.zsh/completions $fpath)

# Fish
sharedserver completion fish > ~/.config/fish/completions/sharedserver.fish
```

## Installation

Using [lazy.nvim](https://github.com/folke/lazy.nvim):

```lua
{
    "georgeharker/shareserver",
    dependencies = { "nvim-lua/plenary.nvim" },
    build = "cargo install sharedserver --force",
    config = function()
        require("sharedserver").setup({
            chroma = {
                command = "chroma",
                args = { "run", "--path", "~/.local/share/chromadb" },
                idle_timeout = "30m",  -- Keep alive 30 minutes after last client
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
    dir = "~/Development/neovim-plugins/shareserver",
    dependencies = { "nvim-lua/plenary.nvim" },
    build = "cargo install --path rust --force",
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
  - **notify**: Notification preferences (see below)

### Notification Configuration

Control when the plugin shows notifications:

```lua
require("sharedserver").setup({
    -- servers...
}, {
    commands = true,
    notify = {
        on_start = true,   -- Notify when starting a new server (default: true)
        on_attach = false, -- Notify when attaching to existing (default: false)
        on_stop = false,   -- Notify when stopping a server (default: false)
        on_error = true,   -- Always notify on errors (default: true)
    }
})
```

By default, the plugin is quiet during normal operations (attach/detach) and only notifies when:
- A new server is started for the first time
- An error occurs
- A server exits unexpectedly (non-zero exit code)

### Multiple Servers (Recommended)

```lua
require("sharedserver").setup({
    -- Server name as key
    chroma = {
        command = "chroma",
        args = { "run", "--path", "~/.local/share/chromadb" },
        lazy = false,  -- Start immediately (default)
        idle_timeout = "1h",  -- Keep alive 1 hour after last client
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
        idle_timeout = "30m",
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
- **working_dir** (optional, default: `nil`): Working directory for the server
- **idle_timeout** (optional, default: `nil`): Grace period duration after last client disconnects (e.g., `"30m"`, `"1h"`, `"2h30m"`)
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

- `:ServerStart <name>` - Start a named server
- `:ServerStop <name>` - Stop a named server
- `:ServerRestart <name>` - Restart a named server
- `:ServerStatus [name]` - Show server status in a floating window (all servers if no name given)
- `:ServerList` - List all registered servers (same as `:ServerStatus` with no args)
- `:ServerStopAll` - Stop all servers

The `:ServerStatus` command displays a rich floating window with:
- Server name and running status (●/○)
- PID, refcount, and uptime for running servers
- Attached/detached state
- Lazy mode indicator
- Full command details when viewing a specific server

Press `q` or `<Esc>` to close the status window.

**Example output of `:ServerStatus`:**
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

The plugin uses **sharedserver** (Rust-based, bundled in `rust/target/release/`) for robust server lifecycle management with a two-lockfile architecture and automatic dead client detection.

### Architecture

**Two lockfiles per server** (default location: `$XDG_RUNTIME_DIR/sharedserver/` or `/tmp/sharedserver/`):
- **`<name>.server.json`**: Persistent server state (PID, command, start time, grace period settings)
- **`<name>.clients.json`**: Active client tracking (refcount, client PIDs with metadata)

**Three states:**
- **ACTIVE**: `clients.json` exists (refcount > 0), server running normally
- **GRACE**: `clients.json` deleted (refcount = 0) but server still alive, waiting for timeout or new client
- **STOPPED**: Both files deleted, server terminated

### Lifecycle Flow

1. **Neovim starts** (`VimEnter`):
   - Plugin checks if server exists using `sharedserver check <name>`
   - For non-lazy servers:
     - If exists: Increments refcount via `sharedserver incref` (attaches to existing)
     - If not: Starts via `sharedserver start <name> -- <command> <args>`
   - For lazy servers:
     - If exists: Attaches (increments refcount)
     - If not: Does nothing (waits for manual start)

2. **Multiple Neovim instances**:
   - Each instance calls `sharedserver incref <name>` on startup
   - Refcount tracked in `clients.json` (e.g., 3 instances = refcount 3)
   - Server process itself is registered as a client

3. **Neovim exits** (`VimLeave`):
   - Plugin calls `sharedserver decref <name>` automatically
   - Refcount decremented
   - If refcount reaches 0:
     - `clients.json` deleted → Server enters **GRACE period**
     - Watcher starts countdown timer (e.g., 30 minutes)
     - **If new client attaches**: Grace cancelled, back to ACTIVE
     - **If grace expires**: Server receives TERM signal, both lockfiles deleted

4. **Dead client detection** (automatic):
   - Watcher polls every 5 seconds
   - Checks each client PID with health checks (Linux: `/proc`, macOS: `proc_pidinfo()`)
   - Automatically removes dead clients and recalculates refcount
   - Prevents refcount leaks from crashed Neovim instances
   - If all clients die: Triggers grace period automatically

### Grace Period Example

```lua
require("sharedserver").setup({
    myserver = {
        command = "myserver",
        idle_timeout = "30m",  -- Stay alive 30 min after last client
    }
})
```

Timeline:
- T+0: First nvim starts → Server launched, refcount=1
- T+5: Second nvim starts → refcount=2
- T+10: First nvim exits → refcount=1
- T+15: Second nvim exits → refcount=0, enter GRACE period (30min countdown)
- T+20: Third nvim starts → refcount=1, grace cancelled, back to ACTIVE
- T+25: Third nvim exits → refcount=0, enter GRACE again
- T+55: Grace expires (30min after T+25) → Server terminated

## Use Cases

- **Database servers**: ChromaDB, Redis, PostgreSQL for development
- **Language servers**: Custom LSP servers, code analysis tools
- **Development servers**: HTTP servers, WebSocket servers
- **Background processes**: File watchers, sync daemons
- **Expensive services**: Large ML models, heavy databases (use `lazy = true`)

For detailed configuration examples and usage patterns, see [EXAMPLES.md](./EXAMPLES.md).

## Shell Integration

The `rust/target/release/sharedserver` binary allows shell scripts and other programs to participate in the shared server lifecycle.

### sharedserver Commands

The sharedserver binary provides a clean command-line interface for managing shared servers:

**Everyday Commands:**
- `use <name> [-- <command> [args...]]` - Attach to server (starts if needed)
- `unuse <name>` - Detach from server
- `list` - Show all managed servers
- `info <name> [--json]` - Get server details (formatted or JSON)
- `check <name>` - Test if server exists (exit codes: 0=active, 1=grace, 2=stopped)
- `completion <shell>` - Generate shell completions (bash/zsh/fish)

**Admin Commands** (for troubleshooting):
- `admin start <name> --pid <pid> -- <command> [args...]` - Manually start server
- `admin stop <name> [--force]` - Emergency stop (sends SIGTERM)
- `admin incref <name> --pid <pid>` - Manually increment refcount
- `admin decref <name> --pid <pid>` - Manually decrement refcount
- `admin debug <name>` - Show invocation logs

**PID Behavior:**
- User commands (`use`, `unuse`): `--pid` defaults to parent process (the caller)
- Admin commands: `--pid` defaults to current process (must be specified)

### Examples

```bash
# Start or attach to a server
sharedserver use myserver -- python -m http.server 8000

# Detach from server
sharedserver unuse myserver

# Check server status
sharedserver check myserver
sharedserver info myserver

# List all servers
sharedserver list

# Emergency stop (admin)
sharedserver admin stop myserver --force
```

### Two-Lockfile Architecture

Serverctl uses a two-lockfile design for robust lifecycle management:

- **`<name>.server.json`**: Persistent while server is running
  - Contains: PID, command, start time, grace period settings, watcher PID
  - Created when server starts
  - Deleted only when server truly dies (after grace period)

- **`<name>.clients.json`**: Exists only while clients are connected (refcount > 0)
  - Contains: refcount, map of client PIDs to metadata (attached timestamp, custom metadata)
  - Created when first client attaches (or when server starts with itself as first client)
  - Deleted when last client dies or decref's (triggers grace period)
  - Watcher automatically removes dead client PIDs every 5 seconds

### Grace Period

When the last client decrements refcount to 0, the server enters a **grace period** instead of immediately shutting down:

```bash
# Start server with 30-minute grace period
sharedserver start --grace-period 30m opencode -- opencode serve --port 4097

# Or 1 hour grace period
sharedserver start --grace-period 1h myserver -- ./server.sh
```

**Grace period flow:**
1. Last client decrefs or dies → `clients.json` deleted
2. Watcher enters GRACE mode, starts countdown timer (e.g., 30 minutes)
3. **If new client increfs during grace**: `clients.json` recreated, timer cancelled, back to ACTIVE
4. **If grace expires**: Watcher sends SIGTERM, waits 5s, sends SIGKILL if needed, `server.json` deleted

### Shell Script Usage

**Requirements:**
- Rust toolchain (see Building from Source section)

**Basic usage:**

```bash
# Check if server exists
if ! sharedserver check opencode; then
    # Start server with 30-minute grace period
    sharedserver start --grace-period 30m opencode -- opencode serve --port 4097 &
    sleep 1  # Wait for startup
fi

# Register as client (optional - for tracking only)
sharedserver incref opencode --metadata "shell-script-$$"

# Use server...
curl http://localhost:4097/health

# Unregister when done
sharedserver decref opencode
```

### Neovim Integration

**The plugin automatically uses sharedserver** - no manual configuration needed:

```lua
require("sharedserver").setup({
    opencode = {
        command = "opencode",
        args = { "serve", "--port", "4097" },
        idle_timeout = "30m",  -- Grace period after all clients disconnect
    }
})
```

**What happens internally:**
- Plugin starts server with: `sharedserver start --grace-period 30m opencode -- opencode serve --port 4097`
- Server survives 30 minutes after all Neovim instances close
- Auto-decref when Neovim exits
- Grace period can be cancelled if a new client attaches
- Dead clients are automatically cleaned up by the watcher

**Lockfile location:**
- Default: `$XDG_RUNTIME_DIR/sharedserver/` or `/tmp/sharedserver/`
- `<name>.server.json` - Server state
- `<name>.clients.json` - Client refcount
- Set `SHAREDSERVER_LOCKDIR` environment variable to override

**Advanced: Manual sharedserver usage from Lua**

If you need to call sharedserver directly (e.g., for custom workflows):

```lua
local sharedserver = vim.fn.stdpath("config") .. "/../path/to/rust/target/release/sharedserver"

-- Check if server exists
local result = vim.fn.system({sharedserver, "check", "myserver"})
if vim.v.shell_error == 0 then
    print("Server is running")
end

-- Get server info as JSON
local json = vim.fn.system({sharedserver, "info", "myserver"})
local info = vim.fn.json_decode(json)
print("PID:", info.pid, "Status:", info.status, "Refcount:", info.refcount)
```

## License

MIT

[crates]: https://img.shields.io/crates/v/sharedserver.svg
