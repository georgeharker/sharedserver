local Job = require("plenary.job")
local Path = require("plenary.path")

local M = {}

-- Storage for multiple server configurations
M._servers = {}

-- Setup multiple servers at once
M.setup = function(servers, opts)
    servers = servers or {}
    opts = opts or { commands = true }

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
    if opts.commands ~= false then
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
        vim.notify("sharedserver: unknown server '" .. name .. "'", vim.log.levels.ERROR)
        return
    end

    local lockdata = M._read_lock(server.config.pidfile)
    if lockdata ~= nil then
        -- Server is running, attach to it
        lockdata.refcount = lockdata.refcount + 1
        M._write_lock(server.config.pidfile, lockdata)
        server.attached = true
        vim.notify("sharedserver: attached to existing '" .. name .. "' (pid " .. lockdata.pid .. ")")
    end
    -- If not running, do nothing (lazy mode)
end

-- Start a named server
M.start = function(name)
    local server = M._servers[name]
    if not server then
        vim.notify("sharedserver: unknown server '" .. name .. "'", vim.log.levels.ERROR)
        return false
    end

    local config = server.config

    if not vim.fn.executable(config.command) then
        vim.notify("sharedserver: command '" .. config.command .. "' is not executable", vim.log.levels.WARN)
        return false
    end

    -- Check if server is already running
    local lockdata = M._read_lock(config.pidfile)
    if lockdata ~= nil then
        -- Increment refcount for this neovim instance
        lockdata.refcount = lockdata.refcount + 1
        M._write_lock(config.pidfile, lockdata)
        server.attached = true
        vim.notify("sharedserver: attached to existing '" .. name .. "' (pid " .. lockdata.pid .. ")")
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
            vim.notify("sharedserver: '" .. name .. "' exited with code " .. exit_code)
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
            vim.notify("sharedserver: started '" .. name .. "' (pid " .. pid .. ")")
            M._write_lock(config.pidfile, {refcount = 1, pid = pid})
            server.attached = true

            if config.on_start then
                config.on_start(pid)
            end
            return true
        else
            vim.notify("sharedserver: failed to start '" .. name .. "'", vim.log.levels.ERROR)
            return false
        end
    end
    return false
end

-- Stop a named server
M.stop = function(name)
    local server = M._servers[name]
    if not server then
        vim.notify("sharedserver: unknown server '" .. name .. "'", vim.log.levels.ERROR)
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
            vim.notify("sharedserver: stopped '" .. name .. "' (pid " .. lockdata.pid .. ")")
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
                vim.notify("sharedserver: failed to decode lockfile " .. pidfile, vim.log.levels.WARN)
            end
        else
            vim.notify("sharedserver: failed to read lockfile " .. pidfile, vim.log.levels.WARN)
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
        vim.notify("sharedserver: failed to write lockfile " .. pidfile, vim.log.levels.ERROR)
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
        if opts.args == "" then
            -- Show all servers
            local statuses = M.status_all()
            local names = vim.tbl_keys(statuses)
            table.sort(names)

            if #names == 0 then
                print("No servers registered")
                return
            end

            print("Registered servers:")
            print("---")
            for _, name in ipairs(names) do
                local status = statuses[name]
                local state
                if status.running then
                    state = string.format("running (pid: %d, refs: %d%s)",
                        status.pid,
                        status.refcount,
                        status.attached and ", attached" or ", not attached")
                else
                    state = "stopped"
                end
                local lazy_indicator = status.lazy and " [lazy]" or ""
                print(string.format("  %s: %s%s", name, state, lazy_indicator))
            end
        else
            -- Show specific server
            local status = M.status(opts.args)
            if status.error then
                print(status.error)
            else
                local state
                if status.running then
                    state = string.format("running (pid: %d, refs: %d%s)",
                        status.pid,
                        status.refcount,
                        status.attached and ", attached" or ", not attached")
                else
                    state = "stopped"
                end
                local lazy_indicator = status.lazy and " [lazy]" or ""
                print(string.format("%s: %s%s", opts.args, state, lazy_indicator))
            end
        end
    end, {
        nargs = "?",
        complete = function()
            return M.list()
        end,
        desc = "Show server status"
    })

    -- :ServerList - List all registered servers
    vim.api.nvim_create_user_command("ServerList", function()
        local servers = M.list()
        table.sort(servers)

        if #servers == 0 then
            print("No servers registered")
            return
        end

        print("Registered servers:")
        for _, name in ipairs(servers) do
            local status = M.status(name)
            local state_icon = status.running and "●" or "○"
            local lazy_indicator = status.lazy and " [lazy]" or ""
            print(string.format("  %s %s%s", state_icon, name, lazy_indicator))
        end
    end, {
        nargs = 0,
        desc = "List all registered servers"
    })

    -- :ServerStopAll - Stop all servers
    vim.api.nvim_create_user_command("ServerStopAll", function()
        M.stop_all()
        vim.notify("Stopped all servers")
    end, {
        nargs = 0,
        desc = "Stop all servers"
    })
end

return M
