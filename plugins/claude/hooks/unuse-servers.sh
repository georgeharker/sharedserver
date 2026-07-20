#!/usr/bin/env bash
# SessionEnd hook: for each entry in the user's sharedserver config,
# run `sharedserver unuse <name> --pid $PPID`. Fast and best-effort —
# if it fails or never runs, sharedserver's dead-client poller will
# reap the refcount within ~5s.

set -u

# Must resolve exactly as use-servers.sh does, or we would `unuse` a different
# server set than we attached to. Highest precedence first: explicit override,
# per-project, then global; first existing file wins (no merging).
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

# Quiet on teardown: if there is no config we never attached anything, and
# SessionEnd is not the place to nag (use-servers.sh reports it on start).
config="$(_resolve_config)" || exit 0

ss_bin="${CLAUDE_PLUGIN_ROOT}/bin/sharedserver"

# Exiting silently here leaks a reference: `use` attached this session and only
# `unuse` detaches it, so a missing tool leaves the refcount high and the server alive
# past its grace period. sharedserver's dead-client poller reclaims it within ~5s of
# the session dying, so this self-corrects — but it should still be traceable, hence
# stderr. No systemMessage: SessionEnd has no payload channel to carry one.
if ! command -v jq >/dev/null 2>&1; then
  echo "sharedserver: jq not found; cannot detach this session (the dead-client poller will reclaim it)" >&2
  exit 0
fi
if ! command -v envsubst >/dev/null 2>&1; then
  echo "sharedserver: envsubst not found; cannot detach this session (the dead-client poller will reclaim it)" >&2
  exit 0
fi

while IFS= read -r entry; do
  [[ -z "$entry" ]] && continue
  name="$(jq -r '.key' <<<"$entry")"
  # Mirror use-servers.sh: never attached a skipIfEnv server when its env var
  # was set, so there is nothing to detach.
  skip_if_env="$(jq -r '.value.skipIfEnv // empty' <<<"$entry")"
  if [[ -n "$skip_if_env" && -n "${!skip_if_env:-}" ]]; then
    continue
  fi
  "$ss_bin" unuse "$name" --pid "$PPID" >/dev/null 2>&1 || true
done < <(envsubst <"$config" | jq -c '.servers // {} | to_entries[]')

exit 0
