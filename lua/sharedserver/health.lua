-- Health check module for :checkhealth sharedserver
local M = {}

-- Resolve the binary exactly the way the plugin does (plugin rust/target,
-- then the fixed install locations) — health must never report a binary the
-- plugin won't actually use.
local function get_sharedserver_path()
    local ok, sharedserver = pcall(require, "sharedserver")
    if not ok then
        return nil, "Plugin not loaded"
    end
    return sharedserver._find_sharedserver()
end

-- Check if sharedserver binary exists and is executable
local function check_binary()
    local path, err = get_sharedserver_path()
    if not path then
        return false, err or "Binary not found in any plugin search path"
    end
    return true, path
end

-- Check if lockdir is accessible — same resolution as the plugin/binary
-- (SHAREDSERVER_LOCKDIR, then XDG_RUNTIME_DIR, then /tmp).
local function check_lockdir()
    local ok, sharedserver = pcall(require, "sharedserver")
    if not ok then
        return false, "Plugin not loaded"
    end

    local lockdir = sharedserver._get_lockdir()

    -- Check if directory exists or can be created
    local stat = vim.loop.fs_stat(lockdir)
    if stat then
        if stat.type == "directory" then
            -- Check if writable
            local test_file = lockdir .. "/.health_check_test"
            local f = io.open(test_file, "w")
            if f then
                f:close()
                os.remove(test_file)
                return true, lockdir
            else
                return false, "Directory exists but is not writable: " .. lockdir
            end
        else
            return false, "Path exists but is not a directory: " .. lockdir
        end
    else
        -- Try to create it
        local success = vim.fn.mkdir(lockdir, "p")
        if success == 1 then
            return true, lockdir
        else
            return false, "Could not create directory: " .. lockdir
        end
    end
end

-- Test basic start/stop cycle
local function test_lifecycle()
    local ok, sharedserver = pcall(require, "sharedserver")
    if not ok then
        return false, "Plugin not loaded"
    end

    -- Check if we have any configured servers
    if not sharedserver._servers or vim.tbl_isempty(sharedserver._servers) then
        return nil, "No servers configured (this is OK, just means no setup() called yet)"
    end

    -- Just verify the API is available
    if
        type(sharedserver.start) ~= "function"
        or type(sharedserver.stop) ~= "function"
        or type(sharedserver.status) ~= "function"
    then
        return false, "API functions missing"
    end

    return true, "API functions available"
end

-- Check version
local function check_version()
    local path = get_sharedserver_path()
    if not path then
        return false, "Could not execute binary"
    end
    local result = vim.fn.system({ path, "--version" })
    if vim.v.shell_error == 0 and result and result:match("sharedserver") then
        return true, vim.trim(result)
    end

    return false, "Could not get version"
end

-- Main health check function
M.check = function()
    local health = vim.health or require("health")

    health.start("sharedserver")

    -- Check binary
    local binary_ok, binary_info = check_binary()
    if binary_ok then
        health.ok("sharedserver binary found: " .. binary_info)

        -- Check version
        local version_ok, version_info = check_version()
        if version_ok then
            health.ok(version_info)
        else
            health.warn("Could not get version: " .. version_info)
        end
    else
        health.error("sharedserver binary not found: " .. binary_info)
        health.info("Searched: <plugin>/rust/target/release, ~/.local/bin, /usr/local/bin, /opt/homebrew/bin")
        health.info("Build in the plugin dir: cargo install --path rust --force")
        health.info("(plain `cargo install sharedserver` lands in ~/.cargo/bin, which the plugin does not search)")
    end

    -- Check lockdir
    local lockdir_ok, lockdir_info = check_lockdir()
    if lockdir_ok then
        health.ok("Lock directory accessible: " .. lockdir_info)
    else
        health.error("Lock directory issue: " .. lockdir_info)
    end

    -- Check API
    local api_ok, api_info = test_lifecycle()
    if api_ok == nil then
        health.info(api_info)
    elseif api_ok then
        health.ok("Plugin API: " .. api_info)
    else
        health.error("Plugin API issue: " .. api_info)
    end

    -- Show current server status
    local ok, sharedserver = pcall(require, "sharedserver")
    if ok and sharedserver._servers and not vim.tbl_isempty(sharedserver._servers) then
        health.info("Configured servers:")
        for name, config in pairs(sharedserver._servers) do
            local info = sharedserver._sharedserver_info(name)
            if info and info.state == "running" then
                health.info("  • " .. name .. ": running (pid " .. info.pid .. ", refs " .. info.refcount .. ")")
            else
                health.info("  • " .. name .. ": stopped")
            end
        end
    end

    -- Health check feature
    health.info("Features:")
    health.info("  • Health check notifications (detects server death after 3s)")
    health.info("  • Reference counting with grace periods")
    health.info("  • Multi-instance server sharing")
    health.info("  • Lazy loading support")
end

return M
