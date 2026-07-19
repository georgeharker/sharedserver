# Claude Code Integration

[`claude-sharedserver`](../plugins/claude)
is a [Claude Code](https://docs.claude.com/en/docs/claude-code/overview) plugin
that manages shared backend processes through the `sharedserver` CLI documented
in this repo. It is the Claude Code counterpart to the
[Neovim plugin](NEOVIM.md) and the [OpenCode plugin](OPENCODE.md): same CLI
underneath, same reference-counted lifecycle, wired into a different host.

The plugin lives in this repository as a plain directory under
`plugins/claude/`. Its own
[README](../plugins/claude/README.md)
is the canonical reference for install, configuration, and diagnostics — this
page is a short orientation.

## How it works

On `SessionStart`, the plugin attaches to (or starts) each configured server
with `sharedserver use`; on `SessionEnd`, it detaches with `sharedserver unuse`.
The client PID is the Claude Code session (`$PPID` of the hook process), so the
refcount tracks sessions rather than the ephemeral hook invocations. Because
`sharedserver` is reference-counted, multiple Claude Code sessions — and any
shells, scripts, Neovim, or OpenCode instances using the same name — share a
single backend process, which survives session restarts inside its grace period
and shuts down when the last client leaves.

Everything in the main [README](../README.md) about states, the two-lockfile
architecture, grace periods, and dead-client detection applies unchanged.

## Requirements

- Claude Code with plugin support
- A `sharedserver` binary reachable via `PATH`, `SHAREDSERVER_BIN`, or a
  standard cargo/homebrew location (install with `cargo install sharedserver`,
  or build from this repo's `rust/` directory)
- `jq` and `envsubst` on `PATH` — the hooks parse the config with `jq` and
  expand `${VAR}` references with `envsubst` (`brew install gettext` on macOS)

## Install

The plugin is a directory you point Claude Code at via the marketplace. The
marketplace manifest lives at the repo root, so add it straight from GitHub:

```sh
claude plugin marketplace add georgeharker/sharedserver
claude plugin install sharedserver@sharedserver
```

Then drop a config file at `~/.config/claude/sharedserver.json` (or set
`CLAUDE_SHAREDSERVER_CONFIG`). `${VAR}` references are expanded throughout:

```json
{
  "servers": {
    "chroma": {
      "command": "chroma",
      "args": ["run", "--path", "${HOME}/.local/share/chromadb"],
      "gracePeriod": "1h"
    },
    "watchman": { "lazy": true }
  }
}
```

The `servers` schema is intentionally compatible with the OpenCode plugin — a
`servers` map copies across without changes. The Claude plugin additionally
supports **`skipIfEnv`**: name an env var and the entry is skipped whenever that
var is non-empty, for when another host already launched the process for this
session.

## Working with the plugin source

The plugin is a plain directory in this repo (`plugins/claude/`), so a plain
clone already contains its full source:

```bash
git clone https://github.com/georgeharker/sharedserver
```

To change the plugin, edit its files under `plugins/claude/` directly and commit
as normal:

```bash
$EDITOR plugins/claude/claude-sharedserver/hooks/use-servers.sh
git add plugins/claude && git commit -m "feat(claude): ..."
```

## Reference

The plugin README covers the parts not repeated here — the full per-server
option table, the exact `sharedserver use` / `unuse` invocations, the
`skipIfEnv` / mcp-companion pairing, and diagnostics.

See the [claude-sharedserver README](../plugins/claude/README.md).
