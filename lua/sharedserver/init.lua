local Job = require("plenary.job")
local Path = require("plenary.path")

local M = {}

-- Storage for multiple server configurations
M._servers = {}

-- Default configuration
M._config = {
    commands = true,
    notify = {
        on_start = true,     -- Notify when starting a new server
        on_attach = false,   -- Notify when attaching to existing server
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

    -- Extract command name for default pidfile
    local cmd_name = opts.command:match("([^/]+)$") or opts.command

    local defaults = {
        command = nil,  -- required
        args = {},
        pidfile = vim.fn.stdpath("cache") .. "/" .. cmd_name .. ".lock.json",
        working_dir = nil,  -- optional, defaults to cwd
        lazy = false,  -- if true, only attach if already running, don't start
        on_start = nil,  -- optional callback(pid)
        on_exit = nil,   -- optional callback(exit_code)
    }

    opts = vim.tbl_extend("force", defaults, opts)
    opts.pidfile = vim.fs.normalize(Path:new(opts.pidfile):normalize())

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

-- Attach to server only if already running (for lazy mode)
M.attach_if_running = function(name)
    local server = M._servers[name]
    if not server then
        M._notify("sharedserver: unknown server '" .. name .. "'", vim.log.levels.ERROR, "error")
        return
    end

    local lockdata = M._read_lock(server.config.pidfile)
    if lockdata ~= nil then
        -- Server is running, attach to it
        lockdata.refcount = lockdata.refcount + 1
        M._write_lock(server.config.pidfile, lockdata)
        server.attached = true
        M._notify("sharedserver: attached to existing '" .. name .. "' (pid " .. lockdata.pid .. ")", vim.log.levels.INFO, "attach")
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

    -- Check if server is already running
    local lockdata = M._read_lock(config.pidfile)
    if lockdata ~= nil then
        -- Increment refcount for this neovim instance
        lockdata.refcount = lockdata.refcount + 1
        M._write_lock(config.pidfile, lockdata)
        server.attached = true
        M._notify("sharedserver: attached to existing '" .. name .. "' (pid " .. lockdata.pid .. ")", vim.log.levels.INFO, "attach")
        return true
    end

    -- Create working directory if specified and doesn't exist
    if config.working_dir then
        local work_path = Path:new(config.working_dir)
        if not work_path:exists() then
            work_path:mkdir({parents = true})
        end
    end

    -- Start the server
    server.job = Job:new({
        command = config.command,
        args = config.args,
        cwd = config.working_dir,
        detached = true,
        on_exit = function(job, exit_code)
            -- Only notify on non-zero exit (unexpected)
            if exit_code ~= 0 then
                M._notify("sharedserver: '" .. name .. "' exited with code " .. exit_code, vim.log.levels.WARN, "error")
            end
            M._remove_lock(config.pidfile)
            server.job = nil
            server.attached = false

            if config.on_exit then
                config.on_exit(exit_code)
            end
        end,
    })

    if server.job ~= nil then
        server.job:start()
        local pid = server.job.pid

        if pid then
            M._notify("sharedserver: started '" .. name .. "' (pid " .. pid .. ")", vim.log.levels.INFO, "start")
            M._write_lock(config.pidfile, {refcount = 1, pid = pid, started_at = os.time()})
            server.attached = true

            if config.on_start then
                config.on_start(pid)
            end
            return true
        else
            M._notify("sharedserver: failed to start '" .. name .. "'", vim.log.levels.ERROR, "error")
            return false
        end
    end
    return false
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

    local config = server.config
    local lockdata = M._read_lock(config.pidfile)
    if lockdata == nil then
        server.attached = false
        return
    end

    -- Decrement refcount
    lockdata.refcount = lockdata.refcount - 1

    if lockdata.refcount == 0 then
        -- Last neovim instance, kill the server
        if server.job and server.job.handle then
            server.job.handle:kill(15)  -- SIGTERM
            M._notify("sharedserver: stopped '" .. name .. "' (pid " .. lockdata.pid .. ")", vim.log.levels.INFO, "stop")
        end
        M._remove_lock(config.pidfile)
        server.attached = false
    else
        -- Other instances still using the server
        M._write_lock(config.pidfile, lockdata)
        server.attached = false
    end
end

-- Stop all servers
M.stop_all = function()
    for name, _ in pairs(M._servers) do
        M.stop(name)
    end
end

-- Internal lock file functions
M._read_lock = function(pidfile)
    if Path:new(pidfile):exists() then
        local file = io.open(pidfile, "r")
        if file ~= nil then
            local content_str = file:read("*a")
            file:close()
            local ok, decoded = pcall(vim.fn.json_decode, content_str)
            if ok then
                return decoded
            else
                M._notify("sharedserver: failed to decode lockfile " .. pidfile, vim.log.levels.WARN, "error")
            end
        else
            M._notify("sharedserver: failed to read lockfile " .. pidfile, vim.log.levels.WARN, "error")
        end
    end
    return nil
end

M._write_lock = function(pidfile, lockdata)
    local content_str = vim.fn.json_encode(lockdata)
    local file = io.open(pidfile, "w")
    if file ~= nil then
        file:write(content_str)
        file:close()
    else
        M._notify("sharedserver: failed to write lockfile " .. pidfile, vim.log.levels.ERROR, "error")
    end
end

M._remove_lock = function(pidfile)
    if Path:new(pidfile):exists() then
        Path:new(pidfile):rm()
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

    local lockdata = M._read_lock(server.config.pidfile)
    if lockdata then
        return {
            name = name,
            running = true,
            attached = server.attached,
            pid = lockdata.pid,
            refcount = lockdata.refcount,
            lazy = server.config.lazy,
        }
    else
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

                -- Calculate uptime
                local lockdata = M._read_lock(server.config.pidfile)
                if lockdata and lockdata.started_at then
                    local uptime = os.time() - lockdata.started_at
                    table.insert(lines, string.format("  Uptime:     %s", M._format_uptime(uptime)))
                end
            end

            if server then
                table.insert(lines, "")
                table.insert(lines, string.format("  Command:    %s", server.config.command))
                if #server.config.args > 0 then
                    table.insert(lines, string.format("  Args:       %s", table.concat(server.config.args, " ")))
                end
                table.insert(lines, string.format("  Lazy:       %s", status.lazy and "yes" or "no"))
                table.insert(lines, string.format("  Pidfile:    %s", server.config.pidfile))
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

            if status.running then
                local lockdata = M._read_lock(server.config.pidfile)
                if lockdata and lockdata.started_at then
                    local uptime = os.time() - lockdata.started_at
                    uptime_str = M._format_uptime(uptime)
                end
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
