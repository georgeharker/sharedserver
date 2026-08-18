#!/usr/bin/env bash
# SessionStart hook: bring up this host's sharedserver profile.
#
# The binary now owns config parsing — discovery, ${VAR} expansion, and profile
# selection all happen inside `sharedserver up`. So this hook is just one call:
# it no longer parses the config itself and needs neither jq nor envsubst.
#
# $PPID is the Claude Code session process. sharedserver's dead-client detection
# polls every ~5s, so even if SessionEnd never fires (crash, kill -9) the
# refcount eventually self-corrects.

set -u

ss_bin="${CLAUDE_PLUGIN_ROOT}/bin/sharedserver"

# The profile this session brings up. A "host" is just a reserved profile name;
# default to "claude", overridable for advanced multi-profile setups.
profile="${CLAUDE_SHAREDSERVER_PROFILE:-claude}"

# `up` discovers the config itself (explicit override -> per-project file walked
# UP from --cwd -> global). Pass the Claude-specific override var and project dir
# so discovery matches what this plugin has always used. --profile-optional keeps
# a user who has not defined a "claude" profile from seeing a warning every
# launch; their universal (profile-less) servers still come up, exactly as before.
args=(up --profile "$profile" --pid "$PPID" --profile-optional --cwd "${CLAUDE_PROJECT_DIR:-$PWD}")
[[ -n "${CLAUDE_SHAREDSERVER_CONFIG:-}" ]] && args+=(--config "$CLAUDE_SHAREDSERVER_CONFIG")

# This hook's stdout is the payload channel (a single JSON object), so `up`'s
# human-readable output must never leak onto it — capture BOTH streams and route
# them to stderr instead. On failure add a systemMessage, since SessionStart
# stderr is invisible at exit 0 and a silent failure to start would be baffling.
if ! out="$("$ss_bin" "${args[@]}" 2>&1)"; then
  printf '{"systemMessage":"sharedserver: could not bring up profile %s (see stderr)"}\n' "$profile"
  [[ -n "$out" ]] && printf '%s\n' "$out" | sed 's/^/  sharedserver: /' >&2
  exit 0
fi
[[ -n "$out" ]] && printf '%s\n' "$out" | sed 's/^/  sharedserver: /' >&2
exit 0
