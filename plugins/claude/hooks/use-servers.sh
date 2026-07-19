#!/usr/bin/env bash
# SessionStart hook: for each entry in the user's sharedserver config,
# run `sharedserver use <name> --pid $PPID [...] -- <command> [args ...]`.
#
# $PPID is the Claude Code session process. sharedserver's dead-client
# detection polls every ~5s, so even if SessionEnd never fires (crash,
# kill -9), the refcount eventually self-corrects.

set -u

# Config resolution, highest precedence first: an explicit override, then a
# per-project config, then the global one. First file that exists wins — the
# per-project file REPLACES the global rather than merging with it.
# unuse-servers.sh resolves identically; keep the two in sync or `unuse` will
# not match what `use` attached to.
_resolve_config() {
  local c d
  # 1. Explicit override.
  if [[ -n "${CLAUDE_SHAREDSERVER_CONFIG:-}" && -f "${CLAUDE_SHAREDSERVER_CONFIG}" ]]; then
    printf '%s' "$CLAUDE_SHAREDSERVER_CONFIG"; return 0
  fi
  # 2. Per-project, WALKED UP from the project dir — mirrors how
  #    mcp-companion discovers .mcp-companion.json.
  d="$(cd "${CLAUDE_PROJECT_DIR:-$PWD}" 2>/dev/null && pwd -P)" || d=""
  while [[ -n "$d" ]]; do
    for c in "$d/.sharedserver.json" "$d/.sharedserver/servers.json"; do
      [[ -f "$c" ]] && { printf '%s' "$c"; return 0; }
    done
    [[ "$d" == "/" ]] && break
    d="$(dirname "$d")"
  done
  # 3. Global fallback.
  c="$HOME/.config/sharedserver/servers.json"
  [[ -f "$c" ]] && { printf '%s' "$c"; return 0; }
  return 1
}

# No config anywhere simply means no configured servers — a normal state, not an
# error. Exit quietly so an unconfigured install launches cleanly.
config="$(_resolve_config)" || exit 0

ss_bin="${CLAUDE_PLUGIN_ROOT}/bin/sharedserver"

# Expand ${VAR} env references throughout the config before parsing.
# Requires envsubst (gettext) — install with `brew install gettext` on macOS.
if ! command -v envsubst >/dev/null 2>&1; then
  echo "sharedserver: envsubst not found (install gettext)" >&2
  exit 0
fi
if ! command -v jq >/dev/null 2>&1; then
  echo "sharedserver: jq not found" >&2
  exit 0
fi

expanded="$(envsubst <"$config")"

# Iterate over .servers entries.
while IFS= read -r entry; do
  [[ -z "$entry" ]] && continue
  name="$(jq -r '.key' <<<"$entry")"
  spec="$(jq -c '.value' <<<"$entry")"

  command="$(jq -r '.command // empty' <<<"$spec")"
  grace="$(jq -r '.gracePeriod // empty' <<<"$spec")"
  log_file="$(jq -r '.logFile // empty' <<<"$spec")"
  metadata="$(jq -r '.metadata // empty' <<<"$spec")"
  lazy="$(jq -r '.lazy // false' <<<"$spec")"
  skip_if_env="$(jq -r '.skipIfEnv // empty' <<<"$spec")"

  # When skipIfEnv names an env var that is set (non-empty), this server has
  # been launched for us by another host — e.g. CodeCompanion / mcp-companion
  # injects MCP_COMPANION_COMBINER_URL, the same var the combiner's MCP client
  # config expands. Don't launch (or attach to) it ourselves in that context.
  if [[ -n "$skip_if_env" && -n "${!skip_if_env:-}" ]]; then
    continue
  fi

  ss_args=(use "$name" --pid "$PPID")
  [[ -n "$grace" ]] && ss_args+=(--grace-period "$grace")
  [[ -n "$log_file" ]] && ss_args+=(--log-file "$log_file")
  [[ -n "$metadata" ]] && ss_args+=(--metadata "$metadata")

  # env entries (additive)
  while IFS= read -r kv; do
    [[ -n "$kv" ]] && ss_args+=(--env "$kv")
  done < <(jq -r '.env // {} | to_entries[] | "\(.key)=\(.value)"' <<<"$spec")

  if [[ "$lazy" == "true" ]]; then
    : # attach-only: no -- command tail
  else
    if [[ -z "$command" ]]; then
      echo "sharedserver: server '$name' missing 'command' and is not lazy; skipping" >&2
      continue
    fi
    cmd_args=()
    while IFS= read -r a; do
      cmd_args+=("$a")
    done < <(jq -r '.args // [] | .[]' <<<"$spec")
    if [[ ${#cmd_args[@]} -gt 0 ]]; then
      ss_args+=(-- "$command" "${cmd_args[@]}")
    else
      ss_args+=(-- "$command")
    fi
  fi

  if ! out="$("$ss_bin" "${ss_args[@]}" 2>&1)"; then
    echo "sharedserver: 'sharedserver use $name' failed (exit $?):" >&2
    [[ -n "$out" ]] && echo "$out" | sed "s/^/  [$name] /" >&2
  elif [[ -n "$out" ]]; then
    echo "$out" | sed "s/^/  [$name] /" >&2
  fi
done < <(jq -c '.servers // {} | to_entries[]' <<<"$expanded")

exit 0
