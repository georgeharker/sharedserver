-- Test script for shareserver plugin
-- Run with: nvim -u test_plugin.lua

-- Add current directory to package path
vim.opt.runtimepath:append(vim.fn.getcwd())

-- Setup the plugin with a test server configuration
require('sharedserver').setup({
  servers = {
    testserver = {
      command = 'sleep',
      args = {'3600'},
      idle_timeout = '30s'
    }
  }
})

print("=== Starting test server ===")
require('sharedserver').start('testserver')

-- Wait a bit for server to start
vim.defer_fn(function()
  print("\n=== Checking server status ===")
  -- Run sharedserver info to check state
  local handle = io.popen('./rust/target/release/sharedserver info testserver')
  if handle then
    local result = handle:read("*a")
    handle:close()
    print(result)
  end
  
  print("\n=== Test complete! ===")
  print("Neovim PID: " .. vim.fn.getpid())
  print("You can now:")
  print("  1. Check server status: ./rust/target/release/sharedserver info testserver")
  print("  2. Quit Neovim (refcount should decrement)")
  print("  3. Wait 30s to see if server shuts down (grace period)")
end, 2000)

-- Keep Neovim open
print("\nNeovim will stay open for testing. Press :q to quit and test decref.")
