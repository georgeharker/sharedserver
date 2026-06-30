-- Example configuration for debugging the goog_ws server issue
-- Add this to your Neovim config to capture server output

require("sharedserver").setup({
    servers = {
        goog_ws = {
            command = "uvx",
            args = { "workspace-mcp", "--transport", "streamable-http" },
            env = {
                GOOGLE_CLIENT_SECRET_PATH = vim.fn.expand("$HOME")
                    .. "/.cache/secrets/"
                    .. vim.fn.expand("$USER")
                    .. ".gcp-oauth.keys.json",
                WORKSPACE_MCP_PORT = "8002",
            },
            lazy = true,

            -- DEBUGGING: Enable this line to capture the server's stdout/stderr
            log_file = "/tmp/sharedserver-debug.log",
        },
    },
    commands = true,
    notify = {
        on_start = true,
        on_attach = true,
        on_stop = false,
        on_error = true,
    },
})

-- Instructions:
-- 1. Reload your Neovim config with the log_file line enabled
-- 2. Run: :ServerStart goog_ws
-- 3. If the server dies immediately:
--    a. You'll see an error notification after 3 seconds
--    b. Check the log: tail -f /tmp/sharedserver-debug.log
-- 4. Compare the command in the log with running it manually in your shell

-- For testing the health check notification:
-- You can test with a server that exits immediately:
--
-- require("sharedserver").setup({
--     servers = {
--         test_exit = {
--             command = "bash",
--             args = { "-c", "exit 1" },
--             log_file = "/tmp/sharedserver-debug.log",
--         },
--     },
-- })
--
-- Then: :ServerStart test_exit
-- Expected: Error notification immediately (command fails)
--
-- Or test with delayed exit:
-- require("sharedserver").setup({
--     servers = {
--         test_delayed_exit = {
--             command = "bash",
--             args = { "-c", "sleep 2; exit 1" },
--             log_file = "/tmp/sharedserver-debug.log",
--         },
--     },
-- })
--
-- Then: :ServerStart test_delayed_exit
-- Expected: Success notification, then error notification after 3 seconds
