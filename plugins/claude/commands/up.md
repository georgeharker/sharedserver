---
description: Bring up a sharedserver profile for this session
argument-hint: <profile>
allowed-tools: Bash(sharedserver:*)
---
Bring up the sharedserver profile `$ARGUMENTS` and report the result, verbatim.
The reference is held by this Claude session (`$PPID`), so the servers stay up
until the session ends or `/…:down $ARGUMENTS` releases them:

!`sharedserver up --profile "$ARGUMENTS" --pid "$PPID" --profile-optional`
