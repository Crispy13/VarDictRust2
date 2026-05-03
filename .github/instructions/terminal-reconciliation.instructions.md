---
description: Require grounded handling before and after terminal execution so terminal calls do not drift into stale analysis or opaque stalls.
applyTo: '**'
---

# Terminal Execution Grounding

## Purpose

Prevent two terminal failure modes:

- false explanations after a completed terminal tool call
- opaque stalls caused by long-running or dropped terminal dispatches being analyzed without live grounding

## Rules

- Before any non-trivial `run_in_terminal` call, choose the execution mode deliberately:
	- Prefer `mode=async` for commands that can run longer than a quick check, can emit substantial output, or mainly need progress monitoring.
	- Use `mode=sync` only for short bounded checks or commands that genuinely need immediate blocking results.
- Before dispatching a non-trivial terminal command, state at least one recovery anchor: the expected process, artifact path, or success/failure marker that can be checked later.
- After any `run_in_terminal` call, do not explain what happened until the terminal tool result has been received.
- If the command is still running, use only terminal-state tools tied to that same execution, such as `get_terminal_output`, `send_to_terminal`, or `kill_terminal` when appropriate.
- The first reasoning step after a terminal call returns must explicitly restate the observed state before any further analysis:
	- completed command: exit condition and key terminal output
	- async/running command: active status, terminal id, and the key live output already observed
- Do not search logs, infer missing execution, or claim there was no terminal result unless that absence has been explicitly verified from the actual tool record.
- If the user later says a session got stuck after a terminal call, first re-ground on live terminal state, current process/artifact state, and recent tool records before making any theory about what happened.
- If a planned terminal command has no provable tool-execution record, treat it as a dropped dispatch boundary rather than as a command result:
	- do not describe it as if it ran
	- rerun the pending command in a fresh terminal when feasible, preferably with `mode=async` and a recovery anchor
- When the user asks about current state, artifact location, or whether something worked after a terminal call, re-ground on the live terminal result or current filesystem state before making any claim.

## Anti-patterns

- Using `mode=sync` for long-running commands that should be monitored asynchronously.
- Launching a non-trivial terminal command without any recovery anchor.
- Saying a command did not run when a completed `run_in_terminal` result already exists.
- Beginning a new troubleshooting theory before consuming the terminal output that just arrived.
- Explaining a dropped or missing terminal-dispatch boundary as if it were the command's result.
- Answering a current-state question from memory of an older validation run instead of the current path or tool result.