# sharedserver

A shared process manager with reference counting, grace periods, and
dead-client detection: one server process shared across shells, scripts, and
any number of Neovim, OpenCode, or Claude Code sessions, started when the first
client needs it and shut down automatically after the last one leaves.

Start with the [overview and quick start](README.md), or jump in:

| I want to… | Document |
|------------|----------|
| Set up servers in Neovim, with ready-made configs | [Configuration Examples](EXAMPLES.md) |
| Go deeper on the Neovim plugin (API, status UI, lazy loading) | [Neovim Integration Guide](docs/NEOVIM.md) |
| Share servers across OpenCode sessions | [OpenCode Integration Guide](docs/OPENCODE.md) |
| Share servers across Claude Code sessions | [Claude Code Integration Guide](docs/CLAUDE_CODE.md) |
| Debug a server that won't start | [Debugging Guide](docs/DEBUGGING.md) |
| See what changed between releases | [Changelog](CHANGELOG.md) |

The CLI works standalone too — see the
[Standalone CLI section](README.md#standalone-cli) of the README.
