# sharedserver.nvim Examples

This document contains various configuration examples for sharedserver.nvim.

## Basic Setup

### Method 1: Using lazy.nvim

```lua
return {
    "sharedserver.nvim",
    dependencies = { "nvim-lua/plenary.nvim" },
    config = function()
        -- Setup multiple servers
        require("sharedserver").setup({
            -- ChromaDB vector database (auto-start)
            chroma = {
                command = "chroma",
                args = { "run", "--path", vim.fn.expand("~/.local/share/chromadb") },
                on_start = function(pid)
                    vim.notify("ChromaDB started on http://localhost:8000")
                end,
            },

            -- Redis (lazy mode - only attach if already running)
            redis = {
                command = "redis-server",
                args = { "--port", "6379" },
                lazy = true,
            },

            -- Local development server (lazy mode)
            devserver = {
                command = "python",
                args = { "-m", "http.server", "8080" },
                working_dir = vim.fn.getcwd(),
                lazy = true,
                on_start = function(pid)
                    vim.notify("Dev server running on http://localhost:8080")
                end,
            },
        })
    end,
}
```

### Method 2: Direct setup (in init.lua)

```lua
require("sharedserver").setup({
    chroma = {
        command = "chroma",
        args = { "run", "--path", vim.fn.expand("~/.local/share/chromadb") },
    },
})
```

### Method 3: Add servers dynamically

```lua
require("sharedserver").register("myserver", {
    command = "my-custom-server",
    args = { "--config", "path/to/config" },
    lazy = true,
})
```

## Command Examples

User commands are automatically created when you call `setup()`. Available commands:

- `:ServerStart <name>` - Start a named server
- `:ServerStop <name>` - Stop a named server
- `:ServerRestart <name>` - Restart a named server
- `:ServerStatus [name]` - Show server status (all servers if no name given)
- `:ServerList` - List all registered servers
- `:ServerStopAll` - Stop all servers

## Keymap Examples

### Basic server control

```lua
vim.keymap.set("n", "<leader>ss", function()
    vim.cmd("ServerList")
end, { desc = "List servers" })

vim.keymap.set("n", "<leader>sS", function()
    vim.cmd("ServerStatus")
end, { desc = "Server status" })
```

### Individual server control

```lua
vim.keymap.set("n", "<leader>sc", function()
    require("sharedserver").start("chroma")
end, { desc = "Start ChromaDB" })

vim.keymap.set("n", "<leader>sC", function()
    require("sharedserver").stop("chroma")
end, { desc = "Stop ChromaDB" })

vim.keymap.set("n", "<leader>sd", function()
    require("sharedserver").start("devserver")
end, { desc = "Start dev server" })
```

## Advanced Examples

### Example 1: ChromaDB + Redis

```lua
require("sharedserver").setup({
    chroma = {
        command = "chroma",
        args = { "run", "--path", vim.fn.expand("~/.local/share/chromadb") },
        on_start = function(pid)
            vim.notify("ChromaDB started on http://localhost:8000")
        end,
    },
    redis = {
        command = "redis-server",
        args = { "--port", "6379" },
        lazy = true,  -- Only use if already running
    },
})
```

### Example 2: Multiple Development Servers

```lua
require("sharedserver").setup({
    frontend = {
        command = "npm",
        args = { "run", "dev" },
        working_dir = vim.fn.getcwd() .. "/frontend",
        lazy = true,
    },
    backend = {
        command = "python",
        args = { "-m", "uvicorn", "main:app", "--reload" },
        working_dir = vim.fn.getcwd() .. "/backend",
        lazy = true,
    },
})

-- Start both with a single command
vim.keymap.set("n", "<leader>sD", function()
    require("sharedserver").start("frontend")
    require("sharedserver").start("backend")
end, { desc = "Start dev servers" })
```

### Example 3: Project-specific development environment

```lua
require("sharedserver").setup({
    postgres = {
        command = "postgres",
        args = { "-D", vim.fn.expand("~/.local/share/postgres/data") },
        lazy = true,
    },
    redis = {
        command = "redis-server",
        args = { "--port", "6379" },
        lazy = true,
    },
    api_server = {
        command = "npm",
        args = { "run", "dev" },
        working_dir = vim.fn.getcwd(),
        lazy = true,
        on_start = function(pid)
            vim.notify("API server running on http://localhost:3000")
        end,
        on_exit = function(exit_code)
            vim.notify("API server exited with code " .. exit_code)
        end,
    },
})
```

## API Usage Examples

### Start/stop servers programmatically

```lua
-- Start a server
local success = require("sharedserver").start("chroma")
if success then
    print("Server started successfully")
end

-- Stop a server
require("sharedserver").stop("chroma")

-- Restart a server
require("sharedserver").restart("chroma")
```

### Check server status

```lua
-- Check single server status
local status = require("sharedserver").status("chroma")
if status.running then
    print("Server running with PID: " .. status.pid)
    print("Reference count: " .. status.refcount)
    print("Attached: " .. tostring(status.attached))
    print("Lazy: " .. tostring(status.lazy))
else
    print("Server not running")
end

-- Check all servers
local statuses = require("sharedserver").status_all()
for name, status in pairs(statuses) do
    print(name, status.running)
end
```

### List registered servers

```lua
local servers = require("sharedserver").list()
for _, name in ipairs(servers) do
    print("Registered server: " .. name)
end
```

## Notification Configuration Examples

### Quiet Mode (Minimal Notifications)

```lua
require("sharedserver").setup({
    chroma = {
        command = "chroma",
        args = { "run" },
    },
}, {
    notify = {
        on_start = false,  -- Silent even on first start
        on_attach = false,
        on_stop = false,
        on_error = true,   -- Still show errors
    }
})
```

### Verbose Mode (All Notifications)

```lua
require("sharedserver").setup({
    chroma = {
        command = "chroma",
        args = { "run" },
    },
}, {
    notify = {
        on_start = true,
        on_attach = true,  -- Notify on every attach
        on_stop = true,    -- Notify on stop
        on_error = true,
    }
})
```

### Default (Recommended)

The default configuration is quiet during normal operations:

```lua
require("sharedserver").setup({
    chroma = {
        command = "chroma",
        args = { "run" },
    },
})
-- Equivalent to:
-- {
--     notify = {
--         on_start = true,   -- Only when starting NEW server
--         on_attach = false, -- Silent when attaching to existing
--         on_stop = false,   -- Silent on normal stop
--         on_error = true,   -- Always show errors
--     }
-- }
```

### Per-Server Custom Notifications

Use `on_start` and `on_exit` callbacks to override default notifications:

```lua
require("sharedserver").setup({
    chroma = {
        command = "chroma",
        args = { "run" },
        on_start = function(pid)
            -- Custom notification (overrides default)
            vim.notify("🔥 ChromaDB ready at http://localhost:8000", vim.log.levels.INFO)
        end,
        on_exit = function(exit_code)
            if exit_code ~= 0 then
                vim.notify("⚠️  ChromaDB crashed!", vim.log.levels.WARN)
            end
        end,
    },
})
```

