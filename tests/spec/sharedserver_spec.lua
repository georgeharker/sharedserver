-- Unit tests for sharedserver plugin
-- Run with: make test

local sharedserver = require("sharedserver")

-- Helper: temporarily set an environment variable for a test
local function with_env(vars, fn)
    local saved = {}
    for k, v in pairs(vars) do
        saved[k] = os.getenv(k)
        -- vim.fn.setenv is the only portable way to affect os.getenv in Neovim
        vim.fn.setenv(k, v)
    end
    local ok, err = pcall(fn)
    for k, v in pairs(saved) do
        if v == nil then
            vim.fn.setenv(k, vim.NIL)
        else
            vim.fn.setenv(k, v)
        end
    end
    if not ok then error(err, 2) end
end

-- Helper: reset plugin server state between tests
local function reset_servers()
    sharedserver._servers = {}
end

-- ============================================================================
-- _get_lockdir
-- ============================================================================

describe("sharedserver._get_lockdir", function()
    before_each(function()
        vim.fn.setenv("SHAREDSERVER_LOCKDIR", vim.NIL)
        vim.fn.setenv("XDG_RUNTIME_DIR", vim.NIL)
    end)

    it("returns SHAREDSERVER_LOCKDIR when set", function()
        with_env({ SHAREDSERVER_LOCKDIR = "/custom/lockdir" }, function()
            assert.equals("/custom/lockdir", sharedserver._get_lockdir())
        end)
    end)

    it("appends /sharedserver to XDG_RUNTIME_DIR when set", function()
        with_env({ XDG_RUNTIME_DIR = "/run/user/1000" }, function()
            assert.equals("/run/user/1000/sharedserver", sharedserver._get_lockdir())
        end)
    end)

    it("SHAREDSERVER_LOCKDIR takes priority over XDG_RUNTIME_DIR", function()
        with_env({
            SHAREDSERVER_LOCKDIR = "/explicit/lockdir",
            XDG_RUNTIME_DIR = "/run/user/1000",
        }, function()
            assert.equals("/explicit/lockdir", sharedserver._get_lockdir())
        end)
    end)

    it("falls back to /tmp/sharedserver when no env vars set", function()
        assert.equals("/tmp/sharedserver", sharedserver._get_lockdir())
    end)
end)

-- ============================================================================
-- _parse_sharedserver_info
-- ============================================================================

describe("sharedserver._parse_sharedserver_info", function()
    it("returns nil and error for nil input", function()
        local result, err = sharedserver._parse_sharedserver_info(nil)
        assert.is_nil(result)
        assert.equals("empty response", err)
    end)

    it("returns nil and error for empty string", function()
        local result, err = sharedserver._parse_sharedserver_info("")
        assert.is_nil(result)
        assert.equals("empty response", err)
    end)

    it("returns nil and error message for invalid JSON", function()
        local result, err = sharedserver._parse_sharedserver_info("not json {{{")
        assert.is_nil(result)
        assert.is_not_nil(err)
        assert.is_truthy(err:match("failed to parse JSON"))
    end)

    it("returns parsed table for valid JSON object", function()
        local json = vim.fn.json_encode({ name = "test", pid = 42, status = "running" })
        local result, err = sharedserver._parse_sharedserver_info(json)
        assert.is_nil(err)
        assert.is_not_nil(result)
        assert.equals("test", result.name)
        assert.equals(42, result.pid)
        assert.equals("running", result.status)
    end)

    it("returns parsed table for valid JSON array", function()
        local json = vim.fn.json_encode({ { name = "a" }, { name = "b" } })
        local result, err = sharedserver._parse_sharedserver_info(json)
        assert.is_nil(err)
        assert.is_not_nil(result)
        assert.equals(2, #result)
        assert.equals("a", result[1].name)
    end)
end)

-- ============================================================================
-- register
-- ============================================================================

describe("sharedserver.register", function()
    before_each(reset_servers)

    it("requires command option", function()
        local notified = false
        local orig = vim.notify
        vim.notify = function(msg, _level)
            if msg:match("command.*required") or msg:match("required.*command") then
                notified = true
            end
        end
        sharedserver.register("nocommand", {})
        vim.notify = orig
        assert.is_true(notified)
        assert.is_nil(sharedserver._servers["nocommand"])
    end)

    it("stores server config in _servers", function()
        sharedserver.register("myserver", {
            command = "sleep",
            args = { "3600" },
            lazy = true,
        })
        local entry = sharedserver._servers["myserver"]
        assert.is_not_nil(entry)
        assert.equals("sleep", entry.config.command)
        assert.same({ "3600" }, entry.config.args)
        assert.is_false(entry.attached)
    end)

    it("normalizes working_dir to an absolute path", function()
        sharedserver.register("dirtest", {
            command = "sleep",
            working_dir = "~/testdir",
            lazy = true,
        })
        local entry = sharedserver._servers["dirtest"]
        assert.is_not_nil(entry)
        -- vim.fs.normalize expands ~ to absolute path
        local wd = entry.config.working_dir
        assert.is_truthy(wd:sub(1, 1) == "/", "working_dir should be absolute, got: " .. tostring(wd))
        assert.is_falsy(wd:match("~"), "working_dir should not contain ~")
    end)

    it("leaves working_dir nil when not provided", function()
        sharedserver.register("nodirtest", {
            command = "sleep",
            lazy = true,
        })
        local entry = sharedserver._servers["nodirtest"]
        assert.is_not_nil(entry)
        assert.is_nil(entry.config.working_dir)
    end)

    it("applies default values for unspecified options", function()
        sharedserver.register("defaults_test", {
            command = "echo",
            lazy = true,
        })
        local cfg = sharedserver._servers["defaults_test"].config
        assert.same({}, cfg.args)
        assert.same({}, cfg.env)
        assert.is_nil(cfg.idle_timeout)
        assert.is_nil(cfg.log_file)
        assert.is_nil(cfg.on_start)
        assert.is_nil(cfg.on_exit)
    end)
end)
