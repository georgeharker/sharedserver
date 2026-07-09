# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Vendored the sibling editor plugins as git submodules under `plugins/`:
  [`opencode-sharedserver`](https://github.com/georgeharker/opencode-sharedserver)
  at `plugins/opencode` and
  [`claude-sharedserver`](https://github.com/georgeharker/claude-sharedserver) at
  `plugins/claude`, with new
  [OpenCode](docs/OPENCODE.md) and [Claude Code](docs/CLAUDE_CODE.md)
  integration guides and a README section.

### Changed
- Replaced the remaining ASCII diagrams with rendered SVG/PNG (editable SVG
  sources kept in `docs/`): the state machine and lifecycle timeline in the
  README, and the `:ServerStatus` window mockup in `docs/NEOVIM.md`.

### Deprecated

### Removed

### Fixed

### Security

## [0.5.0] - 2026-06-27

### Added
- `admin stop` now takes `--timeout <DUR>` (default `10s`) bounding how long it
  waits for teardown to converge.
- New process liveness primitive `process_liveness()` returning `Alive` /
  `Zombie` / `Gone`, and a corresponding **`defunct`** server state (server
  process died but lockfiles not yet removed). `check` now exits `3` for defunct.
- `info --json` now includes the `start_time` and `watcher_start_time` process
  start stamps used by the PID-reuse guard.

### Changed
- **`admin incref` / `admin decref` now require `--pid`.** These low-level
  commands previously defaulted the client PID to the (immediately-exiting) CLI
  process, registering a dead client. They are internal plumbing — `use` /
  `unuse` and the Neovim plugin already always pass `--pid` — so this only
  affects anyone invoking the raw admin commands without it.
- **`admin start` / `use` no longer leave an orphaned watcher/server on a
  start-confirmation timeout.** The wait is now gated on watcher *liveness*: it
  fails fast when the watcher dies without publishing, tolerates a slow-but-alive
  watcher up to a generous cap (so slow lockfile I/O or load doesn't kill a
  healthy start), and on genuine failure tears the whole tree down before
  returning — so a retry can't accidentally start a second instance.
- **Teardown model reworked so the watcher is the single owner of the server
  lifecycle.** It now reaps the server (`waitpid`) so no zombie lingers, polls
  every 500 ms (was 5 s), and deletes lockfiles pid-guarded so a stale watcher
  can't clobber a restarted instance reusing the same name.
- **`admin stop` / `stop --force` now signal and then wait for full teardown**
  (server reaped, lockfiles removed, watcher exited) before returning, instead
  of deleting lockfiles themselves. `stop` errors without escalating; `--force`
  escalates to SIGKILL and, on failure, reports exactly what survived.
- **`admin kill` is now the watcher-independent "floor"**: it SIGKILLs the
  watcher first, then the server's process group, then cleans up itself. Works
  even when the watcher is wedged.
- `admin doctor` now defers to a live watcher instead of racing it, and only
  removes a lockfile when the server is dead **and** no watcher is alive.
- **`clients.json` is now kept for the whole life of the server** instead of
  being deleted when the refcount reaches zero. Grace is now signalled by
  `refcount == 0` (the file stays with an empty client map), not by the file's
  absence. This keeps the lockfile inode stable so its `flock` is a real mutex.
  `refcount` is always derived from the number of distinct client PIDs.
- Documentation is now published to a docs site via a reusable GitHub Actions workflow
- CI tests now use a proper vusted setup
- GitHub Actions workflows can be triggered manually

### Deprecated

### Removed

### Fixed
- **A corrupt or mid-teardown-deleted lockfile no longer turns every command
  into a hard error.** `get_server_state` now reports `stopped` for an
  unreadable/empty server lock (a normal teardown race or corruption) instead of
  propagating an error, and `admin doctor` cleans such locks instead of aborting
  the whole run on the first bad one.
- **Refcount no longer inflates when the same client PID attaches twice.** A
  repeat `incref`/`use` from one PID used to bump the count while the client map
  (keyed by PID) stayed at one entry, so a single `unuse` could then drop the
  server while it was still in use. The refcount is now derived from the client
  set, making repeat attaches idempotent.
- **A recycled PID can no longer be mistaken for the server or signalled by
  mistake.** The server's process start stamp is recorded and verified, so if the
  OS reuses the old server's PID for an unrelated process, `stop`/`kill`/state
  checks treat the server as gone instead of acting on the stranger. The same
  guard now also covers the watcher PID.
- **`parse_duration` rejects overflowing values** (e.g. an absurd
  `--grace-period`/`--timeout`) instead of panicking (debug) or silently
  wrapping to a tiny duration (release).
- **`admin doctor` now also discovers orphaned `clients.json` files** (with no
  matching `server.json`) and cleans them, instead of only scanning for servers.
- **The invocation log is written atomically** (whole line, under a lock), so
  concurrent writers can't interleave partial lines into the audit log.
- **Fixed a lock-correctness bug from deleting and recreating `clients.json`.**
  Because `flock` binds to the inode, deleting the file at refcount 0 and
  recreating it on the next attach let two processes lock different inodes for
  the same path, corrupting the refcount and occasionally dropping a just-attached
  client into grace. The file is no longer deleted mid-life (see Changed).
- **`stop --force` no longer falsely reports "Failed to kill server process"**
  when the kill in fact succeeded: a zombie (post-kill, awaiting reap) is no
  longer treated as a live process. This also unwedges the watcher, which
  previously could loop forever treating the zombie as alive.
- A fast `stop`/`start` cycle with the same name no longer risks the old watcher
  deleting the new instance's lockfiles (the stale-watcher restart race).
- **The daemon no longer aborts on startup under a debug build.** The fd-redirect
  setup closed a borrowed descriptor that the owning `File` then closed again on
  drop; this double-close is a no-op in release but trips std's debug-mode I/O
  safety guard, killing the watcher before it published its lock. The descriptor
  is now owned via `into_raw_fd()` so it is closed exactly once.
- **macOS process start stamps are now microsecond-resolution** (folding in
  `pbi_start_tvusec` alongside `pbi_start_tvsec`), so two processes that reuse a
  PID within the same second still get distinct stamps for the reuse guard.
- Integration tests are now isolated to a dedicated `SHAREDSERVER_LOCKDIR` so
  they no longer touch the user's real lockdir or assert against the wrong path.
- Integration tests select the daemon binary by build profile, so both
  `cargo test` and `cargo test --release` exercise the binary they just built
  rather than a stale one from the other profile.
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

[Unreleased]: https://github.com/georgeharker/sharedserver/compare/v0.5.0...HEAD
[0.5.0]: https://github.com/georgeharker/sharedserver/compare/v0.4.10...v0.5.0
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
