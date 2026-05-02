---
description: Require terminal-result reconciliation before any later analysis of terminal behavior.
applyTo: '**'
---

# Terminal Result Reconciliation

## Purpose

Prevent false explanations about terminal behavior after a completed terminal tool call.

## Rules

- After any `run_in_terminal` call, do not explain what happened until the terminal tool result has been received.
- If the command is still running, use only terminal-state tools tied to that same execution, such as `get_terminal_output`, `send_to_terminal`, or `kill_terminal` when appropriate.
- The first reasoning step after a completed terminal call must explicitly restate the observed exit condition and the key terminal output before any further analysis.
- Do not search logs, infer missing execution, or claim there was no terminal result unless that absence has been explicitly verified from the actual tool record.
- When the user asks about current state, artifact location, or whether something worked after a terminal call, re-ground on the live terminal result or current filesystem state before making any claim.

## Anti-patterns

- Saying a command did not run when a completed `run_in_terminal` result already exists.
- Beginning a new troubleshooting theory before consuming the terminal output that just arrived.
- Answering a current-state question from memory of an older validation run instead of the current path or tool result.