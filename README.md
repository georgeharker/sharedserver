# sharedserver

[![crates][crates]](https://crates.io/crates/sharedserver)

A shared process manager with reference counting, grace periods, and dead-client detection. Use it standalone from the command line or integrate it with Neovim for automatic server lifecycle management.

## Overview

```
┌──────────────────────────────────────────────────────────────┐
│                      sharedserver CLI                         │
│                                                              │
│   use/unuse    ┌─────────────┐   start/stop/check            │
│  ─────────────>│  lockfiles  │<──────────────────             │
│                │  server.json│                                │
│   refcount++   │  clients.json│  grace period                 │
│   refcount--   └─────────────┘  dead-client detection         │
│                       │                                       │
│                       ▼                                       │
│               ┌──────────────┐                                │
│               │ managed      │                                │
│               │ server proc  │                                │
│               └──────────────┘                                │
└──────────────────────────────────────────────────────────────┘
        ▲               ▲               ▲
        │               │               │
   Neovim #1       Neovim #2       shell script
   (refcount=1)    (refcount=1)    (refcount=1)
```

One server process, shared across any number of clients. When the last client disconnects, an optional grace period keeps the server warm before shutdown.

## Standalone CLI

### Install

```bash
cargo install sharedserver
```

Or build from source:

```bash
git clone https://github.com/georgeharker/sharedserver
cd sharedserver/rust
cargo build --release
# binary at rust/target/release/sharedserver
```

### Quick Start

```bash
# Start or attach to a server (starts if not running)
sharedserver use myserver -- python -m http.server 8000

# Detach when done (server stays alive if other clients are attached)
sharedserver unuse myserver

# Check status
sharedserver info myserver
sharedserver list
```

The `use` command increments the refcount (starting the server if needed), and `unuse` decrements it. When refcount hits zero, the server enters a grace period or shuts down immediately.

### Grace Periods

Keep servers warm after the last client disconnects:

```bash
# Start with a 30-minute grace period
sharedserver use myserver --grace-period 30m -- ./expensive-server

# All clients disconnect -> server survives 30 minutes
# New client attaches during grace -> grace cancelled, back to active
# Grace expires -> server receives SIGTERM
```

Duration formats: `30s`, `5m`, `1h`, `2h30m`.

### Shell Script Integration

```bash
#!/bin/bash
# Ensure ChromaDB is running, share it across scripts
sharedserver use chroma --grace-period 1h -- chroma run --path ~/.local/share/chromadb

# Do work...
curl http://localhost:8000/api/v1/heartbeat

# Detach when done
sharedserver unuse chroma
```

Replace fragile `pkill`/`pgrep` patterns:

```bash
# Instead of this:
pkill -f "python -m http.server" || true
python -m http.server 8000 &
# ...work...
pkill -f "python -m http.server"

# Use sharedserver:
sharedserver use webserver -- python -m http.server 8000
# ...work...
sharedserver unuse webserver  # server stays alive if others need it
```

### CLI Commands

**Everyday commands:**

| Command | Description |
|---------|-------------|
| `use <name> [-- <cmd> [args...]]` | Attach to server (starts if needed) |
| `unuse <name>` | Detach from server |
| `list` | Show all managed servers |
| `info <name> [--json]` | Server details (formatted or JSON) |
| `check <name>` | Test if server exists (exit: 0=active, 1=grace, 2=stopped) |
| `completion <shell>` | Generate shell completions (bash/zsh/fish) |

**Admin commands** (troubleshooting):

| Command | Description |
|---------|-------------|
| `admin start <name> -- <cmd>` | Manually start a server with no clients (refcount 0) |
| `admin stop <name> [--force]` | Emergency stop (SIGTERM) |
| `admin incref <name> --pid <pid>` | Manual refcount increment |
| `admin decref <name> --pid <pid>` | Manual refcount decrement |
| `admin debug <name>` | Show invocation logs |
| `admin doctor [name]` | Validate state, clean stale lockfiles |
| `admin kill <name>` | Force kill (SIGKILL) and clean up |

**PID behavior:**
- User commands (`use`, `unuse`): `--pid` defaults to parent process (the caller)
- Admin commands: `--pid` defaults to current process

### Shell Completions

```bash
# Bash
sharedserver completion bash > ~/.local/share/bash-completion/completions/sharedserver

# Zsh
sharedserver completion zsh > ~/.zsh/completions/_sharedserver

# Fish
sharedserver completion fish > ~/.config/fish/completions/sharedserver.fish
```

## How It Works

### Two-Lockfile Architecture

Each server uses two lockfiles (default: `$XDG_RUNTIME_DIR/sharedserver/` or `/tmp/sharedserver/`):

- **`<name>.server.json`** -- Server state: PID, command, start time, grace period, watcher PID. Created on start, deleted on final shutdown.
- **`<name>.clients.json`** -- Client tracking: refcount, map of client PIDs with timestamps. Created on first attach, deleted when refcount hits zero (triggers grace period).

Override location with `SHAREDSERVER_LOCKDIR`.

### Three States

```
                 use/incref               unuse/decref (refcount=0)
  STOPPED ─────────────────> ACTIVE ──────────────────────────> GRACE
     ^                        ^                                   │
     │                        │          use/incref               │
     │                        └───────────────────────────────────┤
     │                                   (grace cancelled)        │
     │                                                            │
     └────────────────────────────────────────────────────────────┘
                              grace expires → SIGTERM → cleanup
```

- **ACTIVE**: `clients.json` exists (refcount > 0), server running normally
- **GRACE**: `clients.json` deleted (refcount = 0), server alive but countdown running
- **STOPPED**: Both files deleted, server terminated

### Dead Client Detection

A watcher process polls every 5 seconds, checking each client PID:
- Linux: `/proc/<pid>` existence
- macOS: `proc_pidinfo()` system call

Dead clients are automatically removed from the refcount. If all clients die (e.g., crash), the grace period starts automatically. This prevents refcount leaks.

### Lifecycle Timeline

```
T+0:   First client attaches  → server starts, refcount=1
T+5:   Second client attaches → refcount=2
T+10:  First client detaches  → refcount=1
T+15:  Second client detaches → refcount=0, GRACE starts (e.g., 30m)
T+20:  Third client attaches  → refcount=1, grace cancelled
T+25:  Third client detaches  → refcount=0, GRACE starts again
T+55:  Grace expires           → SIGTERM, cleanup
```

---

## Neovim Integration

For the full guide — building from source, health monitoring, status UI details,
manual Lua usage, lazy loading, notification config — see
[docs/NEOVIM.md](docs/NEOVIM.md).

### Requirements

- Neovim 0.10+

### Installation

Using [lazy.nvim](https://github.com/folke/lazy.nvim):

```lua
{
    "georgeharker/sharedserver",
    build = "cargo install --path rust",
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

The plugin searches for the `sharedserver` binary in order:
1. `<plugin-dir>/rust/target/release/sharedserver`
2. `~/.local/bin/sharedserver`
3. `/usr/local/bin/sharedserver`
4. `/opt/homebrew/bin/sharedserver`

It does not search `$PATH`, so a binary that only lives in `~/.cargo/bin`
won't be found — build with the `build` command above, or copy it to one of
the locations listed.

### What the Plugin Does

On `VimEnter`:
- Non-lazy servers: checks if running → attaches (incref) or starts
- Lazy servers: attaches if running, otherwise does nothing

On `VimLeave`:
- Automatically decrements refcount for all attached servers

This means multiple Neovim instances share the same server process, and the server survives editor restarts within the grace period.

### Server Configuration

```lua
require("sharedserver").setup({
    servers = {
        myserver = {
            command = "myserver",           -- required: command to run
            args = { "--port", "8080" },    -- optional: arguments
            env = { DEBUG = "1" },          -- optional: extra env vars (additive)
            working_dir = "/path/to/dir",   -- optional: working directory
            log_file = "/tmp/myserver.log", -- optional: capture stdout/stderr
            lazy = false,                   -- optional: only attach if already running
            idle_timeout = "30m",           -- optional: grace period after last client
            on_start = function(pid) end,   -- optional: callback on start
            on_exit = function(code) end,   -- optional: callback on exit (reserved; not yet invoked)
        },
    },
    commands = true,  -- create user commands (default: true)
    notify = {
        on_start = true,   -- notify when starting new server
        on_attach = false,  -- notify when attaching to existing
        on_stop = false,    -- notify when stopping
        on_error = true,    -- always notify on errors
    },
})
```

### Commands

| Command | Description |
|---------|-------------|
| `:ServerStart <name>` | Start a named server |
| `:ServerStop <name>` | Stop a named server |
| `:ServerRestart <name>` | Restart a named server |
| `:ServerStatus [name]` | Show status in floating window |
| `:ServerList` | List all registered servers |
| `:ServerStopAll` | Stop all servers |

`:ServerStatus` shows a floating window with status indicators:
- `●` Running (active or in grace period)
- `○` Stopped

The single-server view (`:ServerStatus <name>`) additionally flags servers in
their grace period.

### Lua API

```lua
local ss = require("sharedserver")

ss.setup({ servers = { ... } })   -- initialize
ss.register(name, config)          -- add server after setup
ss.start(name)                     -- manual start
ss.stop(name)                      -- manual stop
ss.restart(name)                   -- restart
ss.stop_all()                      -- stop all servers
ss.status(name)                    -- { running, pid, refcount, attached, lazy }
ss.status_all()                    -- all server statuses
ss.list()                          -- registered server names
```

### Health Check

```vim
:checkhealth sharedserver
```

Verifies binary installation, lock directory access, and server status.

## Use Cases

**Development databases** -- ChromaDB, Redis, PostgreSQL shared across editor instances with grace periods for quick restarts.

**Project dev servers** -- frontend/backend servers that survive editor restarts.

**Expensive services** -- ML inference servers with `lazy = true`, started only when needed.

**CI/test infrastructure** -- shell scripts managing shared test services with automatic cleanup.

### Why Not systemd/launchd?

| System Service | sharedserver |
|----------------|--------------|
| Always running | Starts when needed, stops when done |
| Requires root/system config | User-space, no sudo |
| Global config files | Per-project config |
| Manual start/stop | Automatic lifecycle |
| One instance system-wide | Multiple isolated instances |

Use system services for production/always-on infrastructure. Use sharedserver for on-demand development services tied to your workflow.

## Debugging

### Capture Server Output

```lua
-- Option 1: log_file option
{
    command = "myserver",
    log_file = "/tmp/myserver.log",
}

-- Option 2: shell redirect
{
    command = "bash",
    args = { "-c", "myserver 2>&1 | tee /tmp/myserver.log" },
}
```

### Common Issues

- **Server exits immediately**: capture output with `log_file`, check environment, use absolute paths
- **Command not found**: use absolute path in `command`
- **Port in use**: check `:ServerStatus`, `sharedserver list`, or `lsof -i :PORT`
- **Stale lockfiles**: `sharedserver admin doctor` to validate and clean up

See [DEBUGGING.md](docs/DEBUGGING.md) for the full troubleshooting guide, and [EXAMPLES.md](./EXAMPLES.md) for more configuration patterns.

## License

MIT

[crates]: https://img.shields.io/crates/v/sharedserver.svg
