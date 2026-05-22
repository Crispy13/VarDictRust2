---
description: Workaround for avoiding being stuck after terminal finishes.
applyTo: '**'
---

## Run All Command in subshell (Mandatory)
Copilot seems to have a bug being stuck after terminal finishes. To avoid this, run all your command in a subshell. For example:

```bash
( your_command_here )
retcode=$?
echo "retcode=$retcode"
```
You don't need to follow the above format strictly, but your command must be wrapped in a subshell.
Never run your command directly without subshell. This is **mandatory**.

## Prefer sync mode for long running task, instead of monitoring.
For tasks expected to run long, to reduce token usage, use sync mode for long running terminal commands instead of monitoring with running new terminals for it.