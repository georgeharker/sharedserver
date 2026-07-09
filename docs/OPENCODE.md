# OpenCode Integration

[`opencode-sharedserver`](https://github.com/georgeharker/opencode-sharedserver)
is an [OpenCode](https://opencode.ai) plugin that manages shared backend
processes through the `sharedserver` CLI documented in this repo. It is the
OpenCode counterpart to the [Neovim plugin](NEOVIM.md): same CLI underneath,
same reference-counted lifecycle, wired into a different editor.

The plugin is vendored into this repository as a git submodule under
[`plugins/opencode/`](../plugins/opencode/) and published to npm as
[`@geohar/opencode-sharedserver`](https://www.npmjs.com/package/@geohar/opencode-sharedserver).
Its own [README](../plugins/opencode/README.md) is the canonical reference for
options, diagnostics, and local development — this page is a short orientation.

For the Claude Code equivalent, see the [Claude Code Integration
guide](CLAUDE_CODE.md).

## How it works

When OpenCode starts, the plugin attaches to (or starts) each configured server
with `sharedserver use`. When OpenCode exits — on `exit`, `SIGINT`, `SIGTERM`,
or `SIGHUP` — it detaches with `sharedserver unuse`. Because `sharedserver` is
reference-counted, multiple OpenCode instances (and any shells, scripts, or
Neovim instances using the same name) share a single backend process. The
server survives OpenCode restarts inside its grace period and shuts down
automatically when the last client leaves.

The plugin only ever speaks to the `sharedserver` CLI; it does not manage
processes directly. Everything in the main [README](../README.md) about states,
the two-lockfile architecture, grace periods, and dead-client detection applies
unchanged.

## Requirements

- OpenCode (with plugin support)
- A `sharedserver` binary — install with `cargo install sharedserver`, or point
  the plugin's `binary` option / `SHAREDSERVER_BIN` env var at a build from this
  repo's `rust/` directory.

## Install

Add the plugin to your OpenCode config (`~/.config/opencode/config.json`).
OpenCode installs npm-published plugins automatically the first time it sees
them in the `plugin` list. The **tuple form** is required — the bare-string
form loads the plugin but passes no options, so no servers are managed.

```jsonc
{
    "plugin": [
        ["@geohar/opencode-sharedserver@latest", {
            "servers": {
                "chroma": {
                    "command": "chroma",
                    "args": ["run", "--path", "{env:HOME}/.local/share/chromadb"],
                    "env": { "ANONYMIZED_TELEMETRY": "False" },
                    "gracePeriod": "30m"
                }
            }
        }]
    ]
}
```

> **Note on `@latest`.** OpenCode caches the installed plugin and does not
> re-resolve dist-tags on every launch, so `@latest` refreshes only
> occasionally. Plugin developers and anyone who needs a specific build should
> pin an explicit version (e.g. `@geohar/opencode-sharedserver@0.1.4`). The
> submodule README has the full details.

## Working with the submodule

The submodule pins a specific commit of the OpenCode plugin. To get it:

```bash
# Clone this repo with the submodule in one step
git clone --recurse-submodules https://github.com/georgeharker/sharedserver

# Or initialize it after a plain clone
git submodule update --init
```

To bump the pinned commit to the plugin's latest `main`:

```bash
git submodule update --remote plugins/opencode
git add plugins/opencode && git commit -m "chore: bump opencode-sharedserver submodule"
```

## Reference

The submodule README covers the parts not repeated here:

- Full per-server option table (`command`, `args`, `env`, `gracePeriod`,
  `logFile`, `metadata`, `lazy`) and binary-resolution order.
- The exact `sharedserver use` / `unuse` invocations the plugin runs.
- TUI toast behavior and the post-attach health check.
- Structured-log line shapes for diagnosing startup problems.

See [`plugins/opencode/README.md`](../plugins/opencode/README.md).
