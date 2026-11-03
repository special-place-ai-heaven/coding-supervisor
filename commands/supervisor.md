---
description: Show or set the coding-supervisor mode for this project (off, audit, enforce, strict)
argument-hint: "[status|off|audit|enforce|strict]"
disable-model-invocation: true
allowed-tools: Bash(supervisor level:*), Bash(supervisor status:*)
---

!`supervisor level $ARGUMENTS`

!`supervisor status`

Show the two outputs above to the user exactly as they are, then stop. Change nothing, run
nothing else, and do not set a mode yourself: only the person who typed this command chooses it.
