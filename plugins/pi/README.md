# pi-sharedserver

A [Pi](https://pi.dev) extension that manages shared backend processes through the
[`sharedserver`](https://github.com/georgeharker/sharedserver) CLI.

When a Pi session starts, the extension brings up this host's profile with
`sharedserver up --profile pi`; when the session quits it releases it with
`sharedserver down`. The `sharedserver` binary reads the config, expands `${VAR}`,
and selects the profile's servers itself. Because `sharedserver` is reference-counted,
multiple Pi instances — or other tools using the same name — share a single backend
process. The server survives Pi restarts inside its grace period and shuts down
automatically when the last client leaves.

It is the Pi counterpart of sharedserver's Claude Code and OpenCode plugins and reads
the **same** `servers.json`, so one config drives every client.

## About sharedserver

[`sharedserver`](https://github.com/georgeharker/sharedserver)
([crates.io](https://crates.io/crates/sharedserver)) is a small Rust CLI that runs a
long-lived process on behalf of several clients with reference counting, a configurable
grace period after the last client detaches, and a watcher that reaps dead clients. It
exposes a tiny verb surface — `use`, `unuse`, `up`, `down`, `list`, `info`, `check`
(`up`/`down` bring a whole **profile** up or down) — and stores
per-server state in lockfiles under `$XDG_RUNTIME_DIR/sharedserver/` (or
`/tmp/sharedserver/`). This extension only ever speaks to that CLI.

**You do not need to install it.** On first use the extension fetches a matching
`sharedserver` from GitHub releases if one isn't already present — prebuilt, so no Rust
toolchain is involved. Any `sharedserver` already on `PATH` (or in `~/.cargo/bin`,
`~/.local/bin`, `/opt/homebrew/bin`, `/usr/local/bin`) is used as-is, and an explicit
`$SHAREDSERVER_BIN` is always honoured. The version fetched matches this extension's own
version, so the pair stay in lockstep (identical to the Claude Code and OpenCode
plugins, with which it shares an install lock).

## Requirements

- [pi](https://github.com/badlogic/pi-mono) coding agent (`@earendil-works/pi-coding-agent`)
- `curl`, for the one-time fetch of `sharedserver` on first use — **nothing else**.
- Node.js 18+

## Install

```bash
pi install npm:@geohar/pi-sharedserver
```

Or from the repo (git-install shim resolves `plugins/pi`):

```bash
pi install git:github.com/georgeharker/sharedserver
```

## Configuration

The extension reads a `servers.json` via the shared discovery chain (first hit wins; a
per-project file *replaces* the global rather than merging):

1. `$SHAREDSERVER_CONFIG`, if set and present.
2. A per-project file walked **up** from the session cwd:
   `.sharedserver.json` or `.sharedserver/servers.json`.
3. The global `~/.config/sharedserver/servers.json`.

```json
{
  "servers": {
    "my-vector-db": {
      "command": "qdrant",
      "args": ["--config-path", "${HOME}/.config/qdrant/config.yaml"],
      "gracePeriod": "1h",
      "logFile": "${HOME}/.local/state/sharedserver/qdrant.log"
    }
  }
}
```

`${VAR}` references are expanded from the environment, matching the Claude hook's
`envsubst` pass, so one file behaves identically in every client.

### Per-server fields

| Field | Meaning |
|-------|---------|
| `command` | Binary to run (required unless `lazy`). |
| `args` | Arguments passed to `command`. |
| `env` | Extra env vars forwarded via `--env KEY=VALUE`. |
| `gracePeriod` | Grace period after the last client detaches, e.g. `30m`, `1h`, `2h30m`. |
| `logFile` | Capture the managed server's stdout/stderr to this path. |
| `metadata` | Optional metadata string forwarded to sharedserver. |
| `lazy` | Only attach if already running; never start it. |
| `skipIfEnv` | Env var name; when set (non-empty) this server is skipped entirely (another host already launched it). |

### Environment overrides

| Variable | Purpose |
|----------|---------|
| `SHAREDSERVER_BIN` | Explicit path to the `sharedserver` binary. |
| `SHAREDSERVER_LOCKDIR` | Override `SHAREDSERVER_LOCKDIR` for child invocations. |
| `SHAREDSERVER_CONFIG` | Explicit path to a `servers.json`, overriding discovery. |
| `PI_SHAREDSERVER_NOTIFY` | Set to `false` to silence TUI notifications. |
| `PI_SHAREDSERVER_PROFILE` | Profile this session brings up. Default `pi`. |

## Profiles

The extension brings up the **`pi`** profile (override with `$PI_SHAREDSERVER_PROFILE`).
Add an optional top-level `profiles` map to `servers.json` to give each host its own
slice:

```jsonc
{
  "servers": { "chroma": { "command": "chroma" }, "watchman": { "lazy": true } },
  "profiles": { "pi": ["chroma"] }
}
```

A server named by no profile (`watchman` above) is **universal** and comes up for every
host. A config with no `profiles` brings up every server, exactly as before — nothing you
already have needs to change.

## Lifecycle

- **`session_start`** → `sharedserver up --profile pi --json` selects and starts this
  host's servers (refcounted; shared across clients). A 2.5s health check then verifies
  each server the report said started/attached is still alive.
- **`session_shutdown` (`reason === "quit"`)** → `sharedserver down --profile pi`
  re-resolves the same selection and releases it. A reload/resume/fork keeps the processes
  and re-attaches. Process exit and `SIGINT`/`SIGTERM`/`SIGHUP` also drain cleanly.

No servers configured is a normal, quiet state — an unconfigured install starts cleanly.

## License

MIT
