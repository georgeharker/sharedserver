-- Test configuration for shareserver/process-wrapper integration
-- Save this file and run with: nvim -u test-sharedserver-integration.lua

-- Setup runtimepath to include shareserver
local plugin_dir = vim.fn.getcwd()
vim.opt.runtimepath:prepend(plugin_dir)

-- Load plenary (required dependency)
vim.opt.runtimepath:prepend(vim.fn.expand("~/.local/share/nvim/lazy/plenary.nvim"))

-- Load the plugin
local sharedserver = require("sharedserver")

-- Configure the plugin
sharedserver.setup({
    opencode = {
        command = "sleep",
        args = {"3600"},  -- Sleep for 1 hour (simulates a long-running server)
        idle_timeout = "30s",  -- Grace period of 30 seconds
    },
}, {
    notify = {
        on_start = true,
        on_attach = true,
        on_stop = true,
        on_error = true,
    },
})

-- Print instructions
vim.defer_fn(function()
    print("\n=== shareserver sharedserver integration test ===")
    print("Commands available:")
    print("  :ServerStatus        - Show all server status")
    print("  :ServerStatus opencode  - Show opencode server details")
    print("  :ServerStart opencode   - Start server (if stopped)")
    print("  :ServerStop opencode    - Stop server manually")
    print("")
    print("Testing workflow:")
    print("1. Open this nvim instance (should start 'sleep 3600')")
    print("2. Run: :ServerStatus opencode")
    print("3. Open another nvim with same config in another terminal")
    print("4. Check refcount increased: :ServerStatus opencode")
    print("5. Close one nvim, check refcount decreased")
    print("6. Close last nvim, wait 30s, server should stop (grace period)")
    print("")
    print("Debug commands:")
    print("  :lua print(vim.inspect(require('sharedserver').status('opencode')))")
    print("  :!$PLUGIN_DIR/bin/sharedserver info opencode")
    print("")
    print("Environment:")
    print("  SHAREDSERVER_LOCKDIR=" .. vim.fn.stdpath("cache") .. "/sharedserver")
    print("")
end, 500)

-- Helpful debug command
vim.api.nvim_create_user_command("DebugServer", function()
    local status = sharedserver.status("opencode")
    print(vim.inspect(status))
end, { desc = "Debug server status" })
