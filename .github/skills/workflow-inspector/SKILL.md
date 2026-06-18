---
name: workflow-inspector
description: >
  Inspect the current VarDict-rs workflow state without changing it. Use when the
  user asks how the workflow is going, what step is active, whether the designed
  workflow is being followed, what evidence exists, what checkpoints are missing,
  what is blocked, or what the next required action is. Prefer this skill over
  ad-hoc status summaries whenever the request is about workflow state,
  compliance, checkpoints, progress, blockers, or next-step inspection.
---

# Workflow Inspector

Report the current workflow state from repository artifacts and session evidence.
This skill is read-only. It does not route work, edit files, or continue execution.

## Purpose

Use this skill to answer workflow-state questions with evidence instead of
impressionistic summaries. The output should tell the caller what workflow is
active, what step is currently in progress, which artifacts support that claim,
where the designed workflow appears to be drifting, and what action is required
next.

This skill is intentionally separate from `workflow-router`:

- `workflow-router` decides what workflow to enter.
- `workflow-inspector` reports where the workflow currently stands.

Do not turn a workflow inspection request into a routing or execution action.

## When To Use

Use this skill whenever the user or a supervising agent explicitly wants a status
check such as:

- "what workflow are we in"
- "how is the workflow going"
- "show me the current step"
- "are we following the designed workflow"
- "what checkpoint is missing"
- "what is blocked"
- "what happens next"
- "give me a workflow report"

## When Not To Use

Do not use this skill for:

- Choosing the next workflow destination. Use `workflow-router` for that.
- Fixing a mismatch, running tests, or dispatching work.
- General code review or parity analysis.

## Evidence Order

Read sources in this order and stop only when you have enough evidence for an
honest report. If a higher-priority source is missing, say so explicitly and
continue to the next source.

1. Chat logs and your context: Probably you have been working based on user directions and workflow files(skills, agent mode, prompt, etc.). Refer to your activity history and context window.
2. Current-session artifacts if they exist:
   - the current CLI session-state artifact path

If the current CLI session-state artifact path does not exist or contains no relevant files, report that as
an evidence gap. Do not fabricate a session state.

## Compliance Checks

Look for evidence-backed gaps only. Good examples:

- Workspace has the defined workflow skills. Read "workflow-management" skill and understand the workspace workflows. There should be one more workflow file or skill you have been referred to.
- Typically, your workflow is set by combining workspace workflow and user's directions. 

## Blocker Checks
You maybe have encountered some blockers while you were trying to follow the workflows. Report all the blockers, including what you solved yourself or have been blocked by.

## Report Format

Make a new Markdown file under the current CLI session-state artifact path and show the path to chat.
The following is the base structure. You can expand but can't ignore it:

```markdown
## Workflow Snapshot

**Current workflow:** <name or best evidence-backed description>
**Active step:** <current step, phase, or addendum task>
**Confidence:** <high | medium | low>

### Workflow 
- <Diagram to explain the workflow (if possible). Prefer Mermaid.>
- <Detailed explantion of the workflow you actually have been used>

### Compliance gaps
- <gap, or "None found from current evidence">

### Blockers or unknowns
- <blocker or unknown, or "None">

### Evidence used
- <artifact path and what it established>
- <artifact path and what it established>

### Required action
- <next actions based on the evidence, to follow the defined workflow faithfully, or improve them>
```

Keep the report concise. Prefer a short evidence-backed answer over a long summary.

## Behavior Constraints

- Stay read-only.
- Do not dispatch subagents just to continue the workflow.
- Do not suggest that the workflow is healthy if the evidence is stale or missing.
- Do not hide uncertainty. Say exactly what is unknown.
- If the request is only for inspection, do not append an execution plan.

## Example Outcomes

### Example 1

Input: "Inspect the current workflow and tell me what step is active."

Output shape:
- Current workflow: full HG002 E2E parity closure
- Active step: refresh or verify chr12 cache, then relaunch the full gate
- Confidence: medium
- Compliance gaps: no fresh rerun artifact proving the next gate has started

### Example 2

Input: "Are we following the designed workflow correctly?"

Output shape:
- Identify the governing workflow from the active plan
- Cite the latest decision-log entry
- Call out any missing review, verification, or evidence artifact
- State the next required action without executing it