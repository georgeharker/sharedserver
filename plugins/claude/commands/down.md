---
description: Release a sharedserver profile for this session
argument-hint: <profile>
allowed-tools: Bash(sharedserver:*)
---
Release the sharedserver profile `$ARGUMENTS` (this session's reference, `$PPID`)
and report the result, verbatim:

!`sharedserver down --profile "$ARGUMENTS" --pid "$PPID" --profile-optional`
