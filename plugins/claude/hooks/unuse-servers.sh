#!/usr/bin/env bash
# SessionEnd hook: for each entry in the user's sharedserver config,
# run `sharedserver unuse <name> --pid $PPID`. Fast and best-effort —
# if it fails or never runs, sharedserver's dead-client poller will
# reap the refcount within ~5s.

set -u

config="${CLAUDE_SHAREDSERVER_CONFIG:-$HOME/.config/claude/sharedserver.json}"
[[ ! -f "$config" ]] && exit 0

ss_bin="${CLAUDE_PLUGIN_ROOT}/bin/sharedserver"

if ! command -v jq >/dev/null 2>&1; then
  exit 0
fi
if ! command -v envsubst >/dev/null 2>&1; then
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
