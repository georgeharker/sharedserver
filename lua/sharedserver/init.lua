local Job = require("plenary.job")
local Path = require("plenary.path")

local M = {}

-- Storage for multiple server configurations
M._servers = {}

-- Get system-wide lockfile directory for sharedserver
-- Matches the Rust sharedserver logic:
-- 1. Check SHAREDSERVER_LOCKDIR env var
-- 2. Use XDG_RUNTIME_DIR/sharedserver if set
-- 3. Fall back to /tmp/sharedserver
M._get_lockdir = function()
    local lockdir_env = os.getenv("SHAREDSERVER_LOCKDIR")
    if lockdir_env then
        return lockdir_env
    end

    local xdg_runtime = os.getenv("XDG_RUNTIME_DIR")
    if xdg_runtime then
        return xdg_runtime .. "/sharedserver"
    end

    return "/tmp/sharedserver"
end

-- Default configuration
M._config = {
    commands = true,
    notify = {
        on_start = true,     -- Notify when starting a new server
        on_attach = true,   -- Notify when attaching to existing server
        on_stop = false,     -- Notify when stopping a server
        on_error = true,     -- Always notify on errors
    }
}
-- Internal notification wrapper
M._notify = function(message, level, event_type)
    event_type = event_type or "info"

    -- Check if this notification type is enabled
    if event_type == "attach" and not M._config.notify.on_attach then
        return
    elseif event_type == "stop" and not M._config.notify.on_stop then
        return
    elseif event_type == "start" and not M._config.notify.on_start then
        return
    end

    -- Always show errors
    if level == vim.log.levels.ERROR or M._config.notify.on_error then
        vim.notify(message, level or vim.log.levels.INFO)
    end
end

-- ============================================================================
-- sharedserver Integration (Rust implementation)
-- ============================================================================

-- Note: The Neovim plugin uses sharedserver directly with incref/decref commands.
-- The process-wrapper binary is available for shell scripts but is NOT used here.
-- The watcher automatically detects and cleans up dead clients every 5 seconds.

-- Find sharedserver binary
M._find_sharedserver = function()
    -- Try relative to plugin directory first (Rust version)
    local script_path = debug.getinfo(1).source:sub(2)  -- Remove @ prefix
    local plugin_dir = vim.fn.fnamemodify(script_path, ':h:h:h')
    local sharedserver = plugin_dir .. '/rust/target/release/sharedserver'

    if vim.fn.executable(sharedserver) == 1 then
        return sharedserver
    end

    -- Try common installation locations
    local common_paths = {
        vim.fn.expand("~/.local/bin/sharedserver"),
        "/usr/local/bin/sharedserver",
        "/opt/homebrew/bin/sharedserver",
    }

    for _, path in ipairs(common_paths) do
        if vim.fn.executable(path) == 1 then
            return path
        end
    end

    return nil
end

-- Call sharedserver and return stdout, stderr, exit_code
M._call_sharedserver = function(args, opts)
    opts = opts or {}
    local sharedserver = M._find_sharedserver()

    if not sharedserver then
        return nil, "sharedserver not found", 1
    end

    -- Set SHAREDSERVER_LOCKDIR to system-wide cache directory
    local lockdir = M._get_lockdir()
    vim.fn.mkdir(lockdir, "p")

    local env = vim.tbl_extend("force", vim.fn.environ(), {
        SHAREDSERVER_LOCKDIR = lockdir,
        SHAREDSERVER_DEBUG = opts.debug and "1" or "0",
    })

    local stdout_lines = {}
    local stderr_lines = {}

    local job = Job:new({
        command = sharedserver,
        args = args,
        env = env,
        on_stdout = function(_, line)
            table.insert(stdout_lines, line)
        end,
        on_stderr = function(_, line)
            table.insert(stderr_lines, line)
        end,
    })

    job:sync()

    local stdout = table.concat(stdout_lines, "\n")
    local stderr = table.concat(stderr_lines, "\n")
    local exit_code = job.code

    return stdout, stderr, exit_code
end

-- Parse sharedserver info JSON output
M._parse_sharedserver_info = function(json_str)
    if not json_str or json_str == "" then
        return nil, "empty response"
    end

    local ok, parsed = pcall(vim.fn.json_decode, json_str)
    if not ok then
        return nil, "failed to parse JSON: " .. tostring(parsed)
    end

    return parsed, nil
end

-- Check if server exists using sharedserver
M._sharedserver_check = function(name)
    local _, _, exit_code = M._call_sharedserver({"check", name})
    return exit_code == 0
end

-- Get server info using sharedserver
M._sharedserver_info = function(name)
    local stdout, stderr, exit_code = M._call_sharedserver({"info", name, "--json"})

    if exit_code ~= 0 then
        return nil, stderr or "server not found"
    end

    return M._parse_sharedserver_info(stdout)
end

-- Increment refcount using sharedserver
M._sharedserver_incref = function(name, metadata)
    local pid = vim.fn.getpid()
    local args = {"admin", "incref", "--pid", tostring(pid), name}
    if metadata then
        table.insert(args, 4, "--metadata")
        table.insert(args, 5, metadata)
    end
    local _, stderr, exit_code = M._call_sharedserver(args)

    if exit_code ~= 0 then
        return false, stderr or "incref failed"
    end

    return true, nil
end

-- Decrement refcount using sharedserver
M._sharedserver_decref = function(name)
    local pid = vim.fn.getpid()
    local _, stderr, exit_code = M._call_sharedserver({"admin", "decref", "--pid", tostring(pid), name})

    if exit_code ~= 0 then
        return false, stderr or "decref failed"
    end

    return true, nil
end

-- ============================================================================
-- End of sharedserver Integration
-- ============================================================================


-- Setup multiple servers at once
M.setup = function(servers, opts)
    servers = servers or {}
    opts = opts or {}

    -- Merge config with defaults
    M._config = vim.tbl_deep_extend("force", M._config, opts)

    -- Support both single server (old API) and multiple servers (new API)
    if servers.command then
        -- Single server mode (backward compatibility)
        local name = servers.name or "default"
        M.register(name, servers)
    else
        -- Multiple servers mode
        for name, config in pairs(servers) do
            M.register(name, config)
        end
    end

    -- Setup VimLeave autocmd to stop all servers
    vim.api.nvim_create_autocmd("VimLeave", {
        callback = function()
            M.stop_all()
        end
    })

    -- Setup user commands if requested (enabled by default)
    if M._config.commands ~= false then
        M._setup_commands()
    end
end

-- Register a named server
M.register = function(name, opts)
    opts = opts or {}

    -- Validate required options
    if not opts.command then
        vim.notify("sharedserver: 'command' is required for server '" .. name .. "'", vim.log.levels.ERROR)
        return
    end

    local defaults = {
        command = nil,  -- required
        args = {},
        working_dir = nil,  -- optional, defaults to cwd
        lazy = false,  -- if true, only attach if already running, don't start
        on_start = nil,  -- optional callback(pid)
        on_exit = nil,   -- optional callback(exit_code)
        idle_timeout = nil,  -- grace period duration (e.g., "30m", "1h", "2h30m")
        env = {},  -- optional environment variables (e.g., {PATH="/usr/bin", DEBUG="1"})
        log_file = nil,  -- optional log file path for server stdout/stderr
    }

    opts = vim.tbl_extend("force", defaults, opts)

    if opts.working_dir then
        opts.working_dir = vim.fs.normalize(Path:new(opts.working_dir):normalize())
    end

    -- Initialize server state
    M._servers[name] = {
        config = opts,
        job = nil,
        attached = false,
    }

    -- Start server on VimEnter or immediately if already entered (unless lazy)
    if not opts.lazy then
        if vim.v.vim_did_enter == 0 then
            vim.api.nvim_create_autocmd("VimEnter", {
                once = true,
                callback = function()
                    M.start(name)
                end
            })
        else
            M.start(name)
        end
    else
        -- Lazy mode: only attach if already running
        if vim.v.vim_did_enter == 0 then
            vim.api.nvim_create_autocmd("VimEnter", {
                once = true,
                callback = function()
                    M.attach_if_running(name)
                end
            })
        else
            M.attach_if_running(name)
        end
    end
end

-- Schedule a health check to verify server is still running after start
M._schedule_health_check = function(name, delay_ms)
    delay_ms = delay_ms or 3000  -- Default to 3 seconds
    
    vim.defer_fn(function()
        local server = M._servers[name]
        if not server or not server.attached then
            return  -- Server was stopped manually, nothing to check
        end
        
        local info = M._sharedserver_info(name)
        if not info or info.state == "stopped" then
            -- Server died after starting
            M._notify(
                "sharedserver: '" .. name .. "' died unexpectedly after start",
                vim.log.levels.ERROR,
                "error"
            )
            server.attached = false
        end
    end, delay_ms)
end

-- Attach to server only if already running (for lazy mode)
M.attach_if_running = function(name)
    local server = M._servers[name]
    if not server then
        M._notify("sharedserver: unknown server '" .. name .. "'", vim.log.levels.ERROR, "error")
        return
    end

    -- Use sharedserver to check and attach
    if M._sharedserver_check(name) then
        local success, err = M._sharedserver_incref(name)
        if success then
            server.attached = true
            local info = M._sharedserver_info(name)
            local pid = info and info.pid or "unknown"
            M._notify("sharedserver: attached to existing '" .. name .. "' (pid " .. pid .. ")", vim.log.levels.INFO, "attach")
        else
            M._notify("sharedserver: failed to attach to '" .. name .. "': " .. (err or "unknown error"), vim.log.levels.ERROR, "error")
        end
    end
    -- If not running, do nothing (lazy mode)
end

-- Start a named server
M.start = function(name)
    local server = M._servers[name]
    if not server then
        M._notify("sharedserver: unknown server '" .. name .. "'", vim.log.levels.ERROR, "error")
        return false
    end

    local config = server.config

    if not vim.fn.executable(config.command) then
        M._notify("sharedserver: command '" .. config.command .. "' is not executable", vim.log.levels.WARN, "error")
        return false
    end

    return M._start_with_sharedserver(name, server, config)
end

-- Start server using sharedserver
M._start_with_sharedserver = function(name, server, config)
    local sharedserver = M._find_sharedserver()
    if not sharedserver then
        M._notify("sharedserver: sharedserver not found", vim.log.levels.ERROR, "error")
        return false
    end

    -- Create working directory if specified and doesn't exist
    if config.working_dir then
        local work_path = Path:new(config.working_dir)
        if not work_path:exists() then
            work_path:mkdir({parents = true})
        end
    end

    -- Set SHAREDSERVER_LOCKDIR environment variable
    local lockdir = M._get_lockdir()
    vim.fn.mkdir(lockdir, "p")

    local env = vim.tbl_extend("force", vim.fn.environ(), {
        SHAREDSERVER_LOCKDIR = lockdir,
    })

    -- Build sharedserver use command (combines start-or-attach + incref)
    -- sharedserver use [--grace-period <duration>] [--pid <pid>] [--metadata <text>] <name> [-- <command> [args...]]
    local sharedserver_args = {"use"}

    -- Add grace period if configured
    if config.idle_timeout then
        table.insert(sharedserver_args, "--grace-period")
        table.insert(sharedserver_args, config.idle_timeout)
    end

    -- IMPORTANT: Explicitly pass Neovim's PID as the client PID
    -- Without this, getppid() would return the intermediate shell/process created by plenary.job,
    -- which exits immediately and causes the watcher to think the client died
    local pid = vim.fn.getpid()
    table.insert(sharedserver_args, "--pid")
    table.insert(sharedserver_args, tostring(pid))

    -- Add metadata (Neovim instance identification)
    table.insert(sharedserver_args, "--metadata")
    table.insert(sharedserver_args, "nvim-" .. pid)

    -- Add environment variables if configured
    if config.env then
        for key, value in pairs(config.env) do
            table.insert(sharedserver_args, "--env")
            table.insert(sharedserver_args, key .. "=" .. value)
        end
    end

    -- Add log file if configured
    if config.log_file then
        table.insert(sharedserver_args, "--log-file")
        table.insert(sharedserver_args, vim.fs.normalize(config.log_file))
    end

    table.insert(sharedserver_args, name)
    table.insert(sharedserver_args, "--")  -- Separator before command
    table.insert(sharedserver_args, config.command)
    for _, arg in ipairs(config.args) do
        table.insert(sharedserver_args, arg)
    end

    -- Execute the use command (will either start server or just incref if already running)
    local stdout, stderr, exit_code = M._call_sharedserver(sharedserver_args, {capture = true})

    if exit_code == 0 then
        server.attached = true
        local info = M._sharedserver_info(name)
        local server_pid = info and info.pid or "unknown"

        -- Determine if we started a new server or attached to existing
        -- Check for "Started" vs "Attached" in the sharedserver output
        local action = stdout and stdout:match("Started") and "started" or "attached to"
        M._notify("sharedserver: " .. action .. " '" .. name .. "' (pid " .. server_pid .. ")", vim.log.levels.INFO, action == "started" and "start" or "attach")

        -- Schedule health check if we just started a new server
        if action == "started" then
            M._schedule_health_check(name, 3000)  -- Check after 3 seconds
        end

        if config.on_start and info and info.pid and action == "started" then
            config.on_start(info.pid)
        end

        return true
    else
        M._notify("sharedserver: failed to use '" .. name .. "': " .. (stderr or "unknown error"), vim.log.levels.ERROR, "error")
        return false
    end
end

-- Stop a named server
M.stop = function(name)
    local server = M._servers[name]
    if not server then
        M._notify("sharedserver: unknown server '" .. name .. "'", vim.log.levels.ERROR, "error")
        return
    end

    if not server.attached then
        return
    end

    -- Use sharedserver to decrement refcount
    local success, err = M._sharedserver_decref(name)
    if not success then
        M._notify("sharedserver: failed to decref '" .. name .. "': " .. (err or "unknown error"), vim.log.levels.WARN, "error")
    end
    server.attached = false
    -- Note: decref is called directly here. The watcher will automatically clean up
    -- any dead clients, so even if this fails, the server will eventually detect it.
end

-- Stop all servers
M.stop_all = function()
    for name, _ in pairs(M._servers) do
        M.stop(name)
    end
end

-- Utility function to manually restart a server
M.restart = function(name)
    M.stop(name)
    vim.defer_fn(function()
        M.start(name)
    end, 500)
end

-- Get current server status
M.status = function(name)
    local server = M._servers[name]
    if not server then
        return {error = "Unknown server '" .. name .. "'"}
    end

    -- Use sharedserver to get server info
    local info, err = M._sharedserver_info(name)
    if info and info.state ~= "stopped" then
        -- Parse sharedserver info response
        return {
            name = name,
            running = true,
            attached = server.attached,
            pid = info.pid,
            refcount = info.refcount or 0,
            lazy = server.config.lazy,
            state = info.state,  -- ACTIVE or GRACE
            started_at = info.started_at,
        }
    else
        -- Server not found or stopped
        return {
            name = name,
            running = false,
            attached = false,
            lazy = server.config.lazy,
        }
    end
end

-- Get status of all servers
M.status_all = function()
    local statuses = {}
    for name, _ in pairs(M._servers) do
        statuses[name] = M.status(name)
    end
    return statuses
end

-- List all registered servers
M.list = function()
    local names = {}
    for name, _ in pairs(M._servers) do
        table.insert(names, name)
    end
    return names
end

-- Format uptime from seconds
M._format_uptime = function(seconds)
    if seconds < 60 then
        return string.format("%ds", seconds)
    elseif seconds < 3600 then
        return string.format("%dm %ds", math.floor(seconds / 60), seconds % 60)
    elseif seconds < 86400 then
        local hours = math.floor(seconds / 3600)
        local mins = math.floor((seconds % 3600) / 60)
        return string.format("%dh %dm", hours, mins)
    else
        local days = math.floor(seconds / 86400)
        local hours = math.floor((seconds % 86400) / 3600)
        return string.format("%dd %dh", days, hours)
    end
end

-- Create floating window with content
M._create_float = function(lines, title, opts)
    opts = opts or {}

    -- Create buffer
    local buf = vim.api.nvim_create_buf(false, true)
    vim.api.nvim_buf_set_lines(buf, 0, -1, false, lines)

    -- Buffer options
    vim.bo[buf].buftype = 'nofile'
    vim.bo[buf].bufhidden = 'wipe'
    vim.bo[buf].filetype = 'sharedserver'
    vim.bo[buf].modifiable = false

    -- Calculate dimensions
    local width = opts.width or 70
    local height = opts.height or math.min(#lines + 2, math.floor(vim.o.lines * 0.8))

    -- Center position
    local col = math.floor((vim.o.columns - width) / 2)
    local row = math.floor((vim.o.lines - height) / 2)

    -- Window options
    local win_opts = {
        relative = 'editor',
        width = width,
        height = height,
        col = col,
        row = row,
        style = 'minimal',
        border = 'rounded',
        title = title or '',
        title_pos = 'center',
    }

    -- Create window
    local win = vim.api.nvim_open_win(buf, true, win_opts)

    -- Window options
    vim.wo[win].cursorline = true
    vim.wo[win].number = false
    vim.wo[win].relativenumber = false

    -- Keymaps
    local close_win = function()
        if vim.api.nvim_win_is_valid(win) then
            vim.api.nvim_win_close(win, true)
        end
    end

    vim.keymap.set('n', 'q', close_win, { buffer = buf, nowait = true })
    vim.keymap.set('n', '<Esc>', close_win, { buffer = buf, nowait = true })

    return buf, win
end

-- Show status in floating window
M._show_status_float = function(server_name)
    local lines = {}

    if server_name then
        -- Show single server details
        local status = M.status(server_name)
        local server = M._servers[server_name]

        if status.error then
            lines = { status.error }
        else
            local icon = status.running and "●" or "○"
            local state = status.running and "running" or "stopped"
            local color = status.running and "Green" or "Gray"

            table.insert(lines, string.format("%s %s - %s", icon, server_name, state))
            table.insert(lines, "")

            if status.running then
                table.insert(lines, string.format("  PID:        %d", status.pid))
                table.insert(lines, string.format("  Refcount:   %d", status.refcount))
                table.insert(lines, string.format("  Attached:   %s", status.attached and "yes" or "no"))

                -- Calculate uptime from sharedserver info
                if status.started_at then
                    local uptime = os.time() - status.started_at
                    table.insert(lines, string.format("  Uptime:     %s", M._format_uptime(uptime)))
                end

                -- Show grace period state if applicable
                if status.state == "GRACE" then
                    table.insert(lines, "  State:      GRACE PERIOD (shutting down)")
                end
            end

            if server then
                table.insert(lines, "")
                table.insert(lines, string.format("  Command:    %s", server.config.command))
                if #server.config.args > 0 then
                    table.insert(lines, string.format("  Args:       %s", table.concat(server.config.args, " ")))
                end
                table.insert(lines, string.format("  Lazy:       %s", status.lazy and "yes" or "no"))
            end
        end

        M._create_float(lines, string.format(" Server: %s ", server_name))
    else
        -- Show all servers
        local statuses = M.status_all()
        local names = vim.tbl_keys(statuses)
        table.sort(names)

        if #names == 0 then
            lines = { "No servers registered" }
            M._create_float(lines, " Shared Servers ")
            return
        end

        -- Header
        table.insert(lines, string.format("%-20s %-12s %-8s %-8s %s", "NAME", "STATUS", "PID", "REFS", "UPTIME"))
        table.insert(lines, string.rep("─", 68))

        -- Server list
        for _, name in ipairs(names) do
            local status = statuses[name]
            local server = M._servers[name]

            local icon = status.running and "●" or "○"
            local state = status.running and "running" or "stopped"
            local pid_str = status.running and tostring(status.pid) or "-"
            local refs_str = status.running and tostring(status.refcount) or "-"
            local uptime_str = "-"

            if status.running and status.started_at then
                local uptime = os.time() - status.started_at
                uptime_str = M._format_uptime(uptime)
            end

            local lazy_indicator = status.lazy and " [lazy]" or ""
            local attached_indicator = (status.running and status.attached) and "" or " [detached]"

            table.insert(lines, string.format(
                "%s %-18s %-12s %-8s %-8s %s%s%s",
                icon,
                name,
                state,
                pid_str,
                refs_str,
                uptime_str,
                lazy_indicator,
                attached_indicator
            ))
        end

        table.insert(lines, "")
        table.insert(lines, "Press q or <Esc> to close")

        M._create_float(lines, " Shared Servers ", { width = 75 })
    end
end

-- Internal function to setup user commands
M._setup_commands = function()
    -- :ServerStart <name> - Start a named server
    vim.api.nvim_create_user_command("ServerStart", function(opts)
        local name = opts.args
        if name == "" then
            vim.notify("Usage: ServerStart <name>", vim.log.levels.ERROR)
            return
        end

        local success = M.start(name)
        if not success then
            vim.notify("Failed to start server '" .. name .. "'", vim.log.levels.ERROR)
        end
    end, {
        nargs = 1,
        complete = function()
            return M.list()
        end,
        desc = "Start a named server"
    })

    -- :ServerStop <name> - Stop a named server
    vim.api.nvim_create_user_command("ServerStop", function(opts)
        local name = opts.args
        if name == "" then
            vim.notify("Usage: ServerStop <name>", vim.log.levels.ERROR)
            return
        end

        M.stop(name)
    end, {
        nargs = 1,
        complete = function()
            return M.list()
        end,
        desc = "Stop a named server"
    })

    -- :ServerRestart <name> - Restart a named server
    vim.api.nvim_create_user_command("ServerRestart", function(opts)
        local name = opts.args
        if name == "" then
            vim.notify("Usage: ServerRestart <name>", vim.log.levels.ERROR)
            return
        end

        M.restart(name)
    end, {
        nargs = 1,
        complete = function()
            return M.list()
        end,
        desc = "Restart a named server"
    })

    -- :ServerStatus [name] - Show server status (all servers if no name given)
    vim.api.nvim_create_user_command("ServerStatus", function(opts)
        M._show_status_float(opts.args ~= "" and opts.args or nil)
    end, {
        nargs = "?",
        complete = function()
            return M.list()
        end,
        desc = "Show server status"
    })

    -- :ServerList - List all registered servers
    vim.api.nvim_create_user_command("ServerList", function()
        M._show_status_float(nil)
    end, {
        nargs = 0,
        desc = "List all registered servers"
    })

    -- :ServerStopAll - Stop all servers
    vim.api.nvim_create_user_command("ServerStopAll", function()
        M.stop_all()
        M._notify("Stopped all servers", vim.log.levels.INFO, "stop")
    end, {
        nargs = 0,
        desc = "Stop all servers"
    })
end

return M
