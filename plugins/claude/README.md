# claude-sharedserver

A [Claude Code](https://docs.claude.com/en/docs/claude-code/overview) plugin that manages shared backend processes through the [`sharedserver`](https://github.com/georgeharker/sharedserver) CLI.

On `SessionStart`, the plugin attaches to (or starts) each configured server with `sharedserver use`. On `SessionEnd`, it detaches with `sharedserver unuse`. Because `sharedserver` is reference-counted, multiple Claude Code sessions — or other tools using the same name — share a single backend process. The server survives session restarts inside its grace period and shuts down automatically when the last client leaves.

This plugin is the Claude Code counterpart to [`opencode-sharedserver`](https://github.com/georgeharker/sharedserver/tree/main/plugins/opencode); see its README for the OpenCode equivalent.

## About sharedserver

[`sharedserver`](https://github.com/georgeharker/sharedserver) ([crates.io](https://crates.io/crates/sharedserver)) is a small Rust CLI that runs a long-lived process on behalf of several clients with reference counting, a configurable grace period after the last client detaches, and a watcher that reaps dead clients automatically. Verbs: `use`, `unuse`, `list`, `info`, `check`. State lives in lockfiles under `$XDG_RUNTIME_DIR/sharedserver/` (or `/tmp/sharedserver/`). This plugin only ever speaks to that CLI; it doesn't manage processes directly.

Install with cargo:

```sh
cargo install sharedserver
sharedserver --version
sharedserver list           # "(no servers)" on a fresh install
```

## Why

Sharedserver is useful for long-lived development services that several clients want to share: an MCP bridge that aggregates many MCP servers, vector DBs, language servers behind a wrapper, model inference servers, dev HTTP servers. This plugin wires those services to Claude Code's session lifecycle so they come up with Claude and tear down cleanly when the last session exits, without you starting them manually.

## Requirements

- Claude Code with plugin support
- [`sharedserver`](https://crates.io/crates/sharedserver) reachable via `PATH`, `SHAREDSERVER_BIN`, or one of the standard cargo/homebrew locations (`bin/sharedserver` wrapper handles resolution)
- `jq` on PATH (hooks parse the config with it)
- `envsubst` on PATH (env-var expansion inside the config) — `brew install gettext` on macOS

## Install

The plugin is just a directory you point Claude Code at via marketplace or `--plugin-dir`.
It ships inside the [`sharedserver`](https://github.com/georgeharker/sharedserver)
repo at `plugins/claude`; the marketplace name and plugin name are both `sharedserver`:

```sh
# From GitHub (the marketplace manifest is at the repo root)
claude plugin marketplace add georgeharker/sharedserver
claude plugin install sharedserver@sharedserver

# Or per-session, pointing at the plugin directory directly
claude --plugin-dir /path/to/sharedserver/plugins/claude
```

Once enabled, drop a config file at `~/.config/claude/sharedserver.json` (or set `CLAUDE_SHAREDSERVER_CONFIG`).

## Configuration

The config is a single JSON file describing one or more servers. `${VAR}` env references are expanded throughout. Example:

```json
{
  "servers": {
    "mcp-bridge": {
      "command": "mcp-bridge",
      "args": [
        "--config", "${HOME}/.cache/secrets/${USER}.mcpservers.json",
        "--port", "9741"
      ],
      "gracePeriod": "30m",
      "logFile": "${HOME}/Library/Logs/mcp-bridge.log"
    },
    "chroma": {
      "command": "chroma",
      "args": ["run", "--path", "${HOME}/.local/share/chromadb"],
      "gracePeriod": "1h"
    },
    "watchman": {
      "lazy": true
    }
  }
}
```

> The `mcp-bridge` example assumes the bridge's console script is on your `PATH`.
> Install it without hardcoding a checkout path via `uv tool install <mcp-companion>/bridge`
> (isolated venv, `mcp-bridge` on PATH), or point `command` at a specific interpreter
> (e.g. `"${MY_BRIDGE_VENV}/bin/python"` with `args: ["-m", "mcp_bridge", …]`). Don't
> hardcode a personal checkout location here — it won't be portable.

Per-server fields (matches the opencode plugin):

| Field         | Type         | Description                                                                              |
|:--------------|:-------------|:-----------------------------------------------------------------------------------------|
| `command`     | `string`     | Binary to run as the shared server. Required unless `lazy` is true.                      |
| `args`        | `string[]`   | Arguments passed to `command`.                                                           |
| `env`         | `object`     | Extra environment variables forwarded via `sharedserver --env KEY=VALUE`.                |
| `gracePeriod` | `string`     | Duration: `30s`, `5m`, `1h`, `2h30m`. Time the server stays alive with no clients.       |
| `logFile`     | `string`     | Capture server stdout/stderr to this path.                                               |
| `metadata`    | `string`     | Optional metadata string forwarded to sharedserver.                                      |
| `lazy`        | `boolean`    | Attach only if the server is already running; never start it.                            |
| `skipIfEnv`   | `string`     | Name of an env var; when it is set (non-empty) this server is skipped entirely — neither started nor attached. Use it when another host already launched the process for this session. |

The whole file passes through `envsubst` before parsing, so `${HOME}`, `${USER}`, `${PATH}`, etc. work in any string value.

### `skipIfEnv` — when something else already launched the server

If Claude Code is started by another host that has already brought the shared
process up and points Claude at it via an env var, you don't want this plugin
launching (or refcounting) a second one. Set `skipIfEnv` to that var's name and
the entry is skipped whenever the var is non-empty.

The canonical case is the MCP bridge under
[mcp-companion](https://github.com/georgeharker/mcp-companion): CodeCompanion
spawns `claude` with `MCP_COMPANION_BRIDGE_URL=…` (the same var the bridge's MCP
client config expands as `${MCP_COMPANION_BRIDGE_URL:-…}`). With
`"skipIfEnv": "MCP_COMPANION_BRIDGE_URL"` the bridge entry no-ops in that
context and Claude simply connects to the bridge the editor owns; run standalone
(var unset) it launches the bridge as usual.

## What it runs

For each configured server, on `SessionStart`:

```
sharedserver use <name> --pid <claude-session-pid> \
    [--grace-period <gracePeriod>] \
    [--metadata <metadata>] \
    [--log-file <logFile>] \
    [--env K=V ...] \
    -- <command> [args ...]
```

The `--` and trailing command are omitted when `lazy: true`.

On `SessionEnd`:

```
sharedserver unuse <name> --pid <claude-session-pid>
```

`<claude-session-pid>` is `$PPID` of the hook process. That's the Claude Code session itself, so the refcount tracks Claude sessions and not the (ephemeral) hook invocations.

## Behavior

- Any failure (missing binary, bad config, `sharedserver use` non-zero exit) is logged to stderr and ignored. The plugin never blocks a Claude session from starting.
- `sharedserver` polls every ~5s for dead clients, so even if `SessionEnd` never fires (hard crash, `kill -9`) the refcount eventually self-corrects.
- Multiple Claude Code sessions pointing at the same server name share one process. The first session starts it; subsequent ones increment the refcount; the last one out triggers the grace period.

## Example: mcp-bridge for mcp-companion

The motivating use case. Write `~/.config/claude/sharedserver.json` as shown in [Configuration](#configuration) above, then register the bridge as an MCP server so Claude Code knows where to dial:

```sh
claude mcp add --transport http --scope user mcp-companion http://127.0.0.1:9741/mcp
```

That's two layers: this plugin owns the lifecycle (start on SessionStart, stop on SessionEnd with grace), and `claude mcp` registration owns the connection. Both pieces are independent — you can pull the lifecycle plugin and run the bridge yourself; or pull the MCP entry and the bridge stays running for other clients (nvim, OpenCode).

## Pairing with other tools

If you also use `mcp-companion` from Neovim, both editors can share a single bridge process: each registers as a `sharedserver` client on its own lifecycle, the bridge stays warm until both exit, and within the grace period a restart re-attaches instantly.

For the OpenCode side, see [opencode-sharedserver](https://github.com/georgeharker/sharedserver/tree/main/plugins/opencode). The config format here is intentionally compatible — copy a `servers` map across without changes.

## Diagnostics

Hook stderr is captured by Claude Code; failures from this plugin show up there. To inspect sharedserver itself:

```sh
sharedserver list
sharedserver info <name>            # add --json for machine-readable
sharedserver admin doctor           # validate state, clean stale lockfiles
```

Common issues:

- **Bridge not reachable from Claude Code**: confirm `sharedserver info mcp-bridge` shows it ACTIVE; `curl http://127.0.0.1:9741/health`; check `claude mcp list` for the registration.
- **Hook fires but server doesn't start**: check `logFile` if set; otherwise run the command standalone to see what it complains about.
- **Stale lockfiles after a crash**: `sharedserver admin doctor` to validate, `sharedserver admin kill <name>` as a last resort.

## License

MIT
