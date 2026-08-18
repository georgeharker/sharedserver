---
description: Look up one sharedserver server — its def and the profiles it's in
argument-hint: <server-name>
allowed-tools: Bash(sharedserver:*)
---
Look up the sharedserver server named `$ARGUMENTS` and report its definition and
which profiles include it, verbatim:

!`sharedserver config lookup "$ARGUMENTS"`
