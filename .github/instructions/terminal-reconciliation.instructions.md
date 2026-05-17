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