#!/usr/bin/env bash
# SessionEnd hook: release this host's sharedserver profile.
#
# Mirrors use-servers.sh exactly — same profile, same discovery inputs — so
# `down` re-resolves the identical selection and releases precisely what `up`
# brought up. Best-effort: if this never runs (hard crash, kill -9), sharedserver's
# dead-client poller reclaims the refcount within ~5s.

set -u

ss_bin="${CLAUDE_PLUGIN_ROOT}/bin/sharedserver"
profile="${CLAUDE_SHAREDSERVER_PROFILE:-claude}"

args=(down --profile "$profile" --pid "$PPID" --profile-optional --cwd "${CLAUDE_PROJECT_DIR:-$PWD}")
[[ -n "${CLAUDE_SHAREDSERVER_CONFIG:-}" ]] && args+=(--config "$CLAUDE_SHAREDSERVER_CONFIG")

# SessionEnd has no payload channel, so everything goes to stderr. A leaked ref
# self-corrects via the dead-client poller, but still trace a failure.
if ! out="$("$ss_bin" "${args[@]}" 2>&1)"; then
  echo "sharedserver: releasing profile '$profile' failed (the dead-client poller will reclaim it): ${out:-unknown}" >&2
fi
exit 0
