---
description: Workaround for avoiding being stuck after terminal finishes.
applyTo: '**'
---


## Prefer sync mode for long running task, instead of monitoring.
For tasks expected to run long, to reduce token usage, use sync mode for long running terminal commands instead of monitoring with running new terminals for it.