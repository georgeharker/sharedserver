# Pi Integration

[`pi-sharedserver`](../plugins/pi)
is a [Pi](https://github.com/badlogic/pi-mono) extension that manages shared
backend processes through the `sharedserver` CLI documented in this repo. It is
the Pi counterpart to the [Neovim plugin](NEOVIM.md), the [OpenCode
plugin](OPENCODE.md), and the [Claude Code plugin](CLAUDE_CODE.md): same CLI
underneath, same reference-counted lifecycle, wired into a different host.

The extension lives in this repository as a plain directory under `plugins/pi/`
and is published to npm as
[`@geohar/pi-sharedserver`](https://www.npmjs.com/package/@geohar/pi-sharedserver).
Its own
[README](https://github.com/georgeharker/sharedserver/blob/main/plugins/pi/README.md)
is the canonical reference for options, commands, and diagnostics — this page is
a short orientation.

## How it works

On `session_start`, the extension brings up this host's profile with
`sharedserver up --profile pi --json`; on `session_shutdown` (when `reason ===
"quit"`) it releases it with `sharedserver down --profile pi`. The binary reads
the config, expands `${VAR}`, and selects the profile's servers itself; the
extension health-checks what the JSON report says came up. A reload, resume, or
fork keeps the processes and re-attaches; process exit and
`SIGINT`/`SIGTERM`/`SIGHUP` also drain cleanly. Because `sharedserver` is
reference-counted, multiple Pi instances — and any shells, scripts, Neovim,
OpenCode, or Claude Code sessions using the same name — share a single backend
process, which survives session restarts inside its grace period and shuts down
when the last client leaves.

The extension only ever speaks to the `sharedserver` CLI; it does not manage
processes directly. Everything in the main [README](../README.md) about states,
the two-lockfile architecture, grace periods, and dead-client detection applies
unchanged.

## Requirements

- [pi](https://github.com/badlogic/pi-mono) coding agent
  (`@earendil-works/pi-coding-agent`)
- `curl` — the extension fetches a matching `sharedserver` on first use if one
  isn't already present, so **no Rust toolchain is required**. Any binary
  reachable via `PATH`, `SHAREDSERVER_BIN`, or a standard cargo/homebrew location
  is used instead of downloading; the fetched version matches this extension's
  own, so the pair stay in lockstep.
- Node.js 18+

## Install

```bash
pi install npm:@geohar/pi-sharedserver
```

Or straight from the repo (the git-install shim resolves `plugins/pi`):

```bash
pi install git:github.com/georgeharker/sharedserver
```

Then drop a config file at `~/.config/sharedserver/servers.json` (or set
`SHAREDSERVER_CONFIG`). `${VAR}` references are expanded throughout:

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

The `servers` schema is intentionally compatible with the OpenCode and Claude
Code plugins — a `servers` map copies across without changes, so one config
drives every client. An optional top-level `profiles` map groups servers so this
host brings up only its `pi` profile (plus universal, profile-less servers);
override the profile with `$PI_SHAREDSERVER_PROFILE`.

## Commands

Beyond the automatic host-profile lifecycle, the extension registers a
`/sharedserver` slash command for on-demand control and introspection:

| Command | Does |
| --------- | ------ |
| `/sharedserver status` | Show running servers |
| `/sharedserver up <profile>` | Bring up a (task) profile on demand |
| `/sharedserver down <profile>` | Release it |
| `/sharedserver config show` | Print the whole config |
| `/sharedserver config lookup <name>` | One server's def + the profiles it's in |

Verbs and the `config` sub-verbs autocomplete. Config *mutations*
(`register`/`unregister`) are intentionally not exposed as slash commands — those
are install-time edits, not in-session actions.

## Working with the extension source

The extension is a plain directory in this repo (`plugins/pi/`), so a plain clone
already contains its full source:

```bash
git clone https://github.com/georgeharker/sharedserver
```

To change the extension, edit its files under `plugins/pi/` directly and commit
as normal:

```bash
$EDITOR plugins/pi/src/index.ts
git add plugins/pi && git commit -m "feat(pi): ..."
```

## Reference

The extension README covers the parts not repeated here — the full per-server
option table (`command`, `args`, `env`, `gracePeriod`, `logFile`, `metadata`,
`lazy`, `skipIfEnv`), the environment overrides (`SHAREDSERVER_BIN`,
`SHAREDSERVER_LOCKDIR`, `SHAREDSERVER_CONFIG`, `PI_SHAREDSERVER_NOTIFY`,
`PI_SHAREDSERVER_PROFILE`), the config-discovery chain, and the exact
`sharedserver up` / `down` invocations.

See the [pi-sharedserver README](https://github.com/georgeharker/sharedserver/blob/main/plugins/pi/README.md).
</content>
</invoke>
