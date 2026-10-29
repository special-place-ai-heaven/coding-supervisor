---
description: Set how strictly the completion gate judges "done" (off, lite, full, ultra), or show its status
argument-hint: off|lite|full|ultra|status
disable-model-invocation: true
allowed-tools: Bash(node "${CLAUDE_PLUGIN_ROOT}/hooks/supervisor-hook.js" slash*)
---
The gate answered:

!`node "${CLAUDE_PLUGIN_ROOT}/hooks/supervisor-hook.js" slash $ARGUMENTS`

Show the answer above to the user as it is and do nothing else. Do not change the level yourself.
