# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

### Changed

### Deprecated

### Removed

### Fixed
- Fixed notification detection logic that incorrectly identified server starts as attaches, causing `on_start` notifications to not appear

### Security

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

[Unreleased]: https://github.com/yourusername/shareserver/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/yourusername/shareserver/compare/v0.1.0...v0.2.0
