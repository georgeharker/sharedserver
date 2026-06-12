# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

### Changed
- Documentation is now published to a docs site via a reusable GitHub Actions workflow
- CI tests now use a proper vusted setup
- GitHub Actions workflows can be triggered manually

### Deprecated

### Removed

### Fixed
- Fixed the CI test workflow
- Corrected changelog repository links to point at the actual GitHub repository

### Security

## [0.4.10] - 2026-04-13

### Changed
- Lua test suite rewritten as busted specs, with a Makefile target and a GitHub Actions test workflow

### Removed
- Removed the plenary.nvim dependency — the Lua plugin and its tests no longer require plenary

## [0.4.9] - 2026-03-29

### Added
- New `is_registered(name)` Lua API to check whether a server has been configured

### Fixed
- Lazy servers are no longer preemptively attached at startup; they now start only on first use

## [0.4.8] - 2026-03-24

### Fixed
- Servers are now started in their own process group, and `admin stop`/`admin kill` (and the watcher) terminate the whole process group — ensuring the entire server stack, including child processes, is reliably killed

## [0.4.7] - 2026-03-22

### Changed
- Maintenance release (version bump only, no functional changes)

## [0.4.6] - 2026-03-22

### Changed
- Internal cleanup: lint fixes and release script improvements (no functional changes)

## [0.4.5] - 2026-03-22

### Added
- **JSON output for `list`**: `sharedserver list --json` emits machine-readable server status

### Changed
- Lua plugin now queries server status via the JSON output for more robust parsing

## [0.4.4] - 2026-02-12

### Added
- **Admin doctor command**: Validate server state and automatically clean up issues
    - Check all servers: `sharedserver admin doctor`
    - Check specific server: `sharedserver admin doctor <name>`
    - Validates server/watcher processes are alive
    - Checks all client PIDs are valid processes
    - Verifies refcount matches actual client count
    - Validates state constraints (Active has clients, Grace has none)
    - Automatically removes stale lockfiles for stopped servers
    - Color-coded output with ✓/⚠ indicators
- **Admin kill command**: Force kill unresponsive servers
    - Force kill: `sharedserver admin kill <name>`
    - Sends SIGKILL immediately (no grace period)
    - Also kills watcher process if it exists
    - Cleans up all lockfiles (server and clients)
    - More aggressive than `admin stop --force`
    - Useful when servers won't stop normally

### Changed
- **BREAKING**: Lua API restructured to single-parameter format
    - **Old**: `setup(servers_table, options_table)`
    - **New**: `setup({ servers = {...}, commands = true, notify = {...} })`
    - All server configurations must now be in a `servers` table
    - Configuration options (commands, notify) are direct fields in opts
    - No backward compatibility - clean break for cleaner API
    - See README.md and EXAMPLES.md for migration examples

### Removed
- Single-server mode removed from Lua plugin (use `servers` table instead)

### Fixed
- Hardened the new commands against crashes and added test coverage

## [0.4.3] - 2026-02-12

### Added
- **Environment variable support**: Pass custom environment variables to server processes
    - New `env` configuration option in Lua (table format: `{KEY = "value"}`)
    - New `--env KEY=VALUE` CLI argument (repeatable)
    - Environment variables are inherited and extended, not replaced
    - Useful for API keys, debug flags, custom paths, and feature toggles
    - Example: `env = {DEBUG = "1", API_KEY = "secret"}` in server config
- **Server logging support**: Capture server stdout/stderr to a file
    - New `log_file` option in Lua server config
    - New `--log-file` CLI argument
    - Server output is appended to the log file (otherwise discarded to /dev/null)
- **Health check on server start**: Detect servers that die immediately after starting
    - Checks the server a few seconds after launch and shows an error notification if it died unexpectedly
    - Respects the `notify.on_error` configuration setting
    - Only runs for newly started servers, not attachments
- **`:checkhealth sharedserver` support**: Verify your setup from inside Neovim
    - Verifies the `sharedserver` binary installation and version
    - Checks lock directory accessibility and permissions
    - Validates the plugin API is loaded correctly
    - Shows status of all configured servers
- New debugging guide (`docs/DEBUGGING.md`) covering the health check system and how to capture server output
- Rust integration test suite and expanded shell test coverage

### Changed
- Expanded README and examples; added build badge

## [0.4.1] - 2026-02-10

### Changed
- Rust code consolidated into a single `sharedserver` crate (previously split into `sharedserver-cli` and `sharedserver-core`), simplifying installation via `cargo install sharedserver`
- README: clarified installation instructions

## [0.3.7] - 2026-02-10

### Changed
- Improved the release/versioning script (internal, no functional changes)

## [0.3.6] - 2026-02-10

### Fixed
- Fixed crate README references in `Cargo.toml` so packages render correctly on crates.io

## [0.3.5] - 2026-02-10

### Changed
- Publish workflow now takes the crates.io token from the environment (CI only, no functional changes)

## [0.3.4] - 2026-02-10

### Changed
- Maintenance release while iterating on crates.io publishing (version bump only, no functional changes)

## [0.3.3] - 2026-02-10

### Changed
- Maintenance release while iterating on crates.io publishing (version bump only, no functional changes)

## [0.3.2] - 2026-02-10

### Changed
- Maintenance release while iterating on crates.io publishing (version bump only, no functional changes)

## [0.3.1] - 2026-02-10

First tagged release, built on the new Rust CLI.

### Added
- **Rust `sharedserver` CLI**: Complete rewrite of the shell wrapper in Rust
    - User commands: `use`, `unuse`, `list`, `info`, `check`, `completion`
    - Admin namespace: `admin start`, `admin stop`, `admin incref`, `admin decref`, `admin debug`
    - Watcher process manages grace-period shutdown after the last client detaches
- Monitoring and recovery test suite covering server lifecycle edge cases
- crates.io publish workflows and a release script

### Changed
- Project renamed from `sharedserver.nvim` to `sharedserver`
- CLI binary is now `sharedserver`, replacing the `serverctl` shell wrapper
- Hardened reference counting and start/stop/watcher handling based on the new monitoring tests

### Fixed
- Fixed notification detection logic that incorrectly identified server starts as attaches, causing `on_start` notifications to not appear

## [0.2.0] - 2026-02-08

### Added
- New user-friendly `serverctl unuse` command for detaching from servers
- Separate admin command namespace (`serverctl admin`) for low-level operations
- Improved help text showing everyday commands vs admin commands
- Shell completion support for both user and admin commands

### Changed
- **BREAKING**: Restructured CLI - `incref`/`decref` moved to `admin incref`/`admin decref`
- **BREAKING**: `--pid` in user commands (`use`, `unuse`) now defaults to **parent process** instead of current process
- Lua plugin now calls `admin incref`/`admin decref` explicitly
- `serverctl use` command no longer requires `--pid` parameter (defaults to parent/caller)
- Updated README with new command structure and usage examples

### Migration Guide
If you were calling `serverctl incref` or `serverctl decref` directly:
- Change `serverctl incref <name> --pid <pid>` → `serverctl admin incref <name> --pid <pid>`
- Change `serverctl decref <name> --pid <pid>` → `serverctl admin decref <name> --pid <pid>`

For most users, prefer the new high-level commands:
- Use `serverctl use <name> -- <command>` to start/attach
- Use `serverctl unuse <name>` to detach

[Unreleased]: https://github.com/georgeharker/sharedserver/compare/v0.4.10...HEAD
[0.4.10]: https://github.com/georgeharker/sharedserver/compare/v0.4.9...v0.4.10
[0.4.9]: https://github.com/georgeharker/sharedserver/compare/v0.4.8...v0.4.9
[0.4.8]: https://github.com/georgeharker/sharedserver/compare/v0.4.7...v0.4.8
[0.4.7]: https://github.com/georgeharker/sharedserver/compare/v0.4.6...v0.4.7
[0.4.6]: https://github.com/georgeharker/sharedserver/compare/v0.4.5...v0.4.6
[0.4.5]: https://github.com/georgeharker/sharedserver/compare/v0.4.4...v0.4.5
[0.4.4]: https://github.com/georgeharker/sharedserver/compare/v0.4.3...v0.4.4
[0.4.3]: https://github.com/georgeharker/sharedserver/compare/v0.4.1...v0.4.3
[0.4.1]: https://github.com/georgeharker/sharedserver/compare/v0.3.7...v0.4.1
[0.3.7]: https://github.com/georgeharker/sharedserver/compare/v0.3.6...v0.3.7
[0.3.6]: https://github.com/georgeharker/sharedserver/compare/v0.3.5...v0.3.6
[0.3.5]: https://github.com/georgeharker/sharedserver/compare/v0.3.4...v0.3.5
[0.3.4]: https://github.com/georgeharker/sharedserver/compare/v0.3.3...v0.3.4
[0.3.3]: https://github.com/georgeharker/sharedserver/compare/v0.3.2...v0.3.3
[0.3.2]: https://github.com/georgeharker/sharedserver/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/georgeharker/sharedserver/releases/tag/v0.3.1
[0.2.0]: https://github.com/georgeharker/sharedserver/commit/f91769b1c79899895bf70df8491a831d532cfb30
