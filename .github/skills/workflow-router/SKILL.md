---
name: workflow-router
description: >
  Route the Parity Orchestrator to the correct workflow based on project state
  and user request. Use when: starting a new orchestrator cycle, user makes a
  request that doesn't match the current workflow, active plan state changes,
  deciding between porting, E2E gate, targeted fix, parity check, or audit.
---

# Workflow Router

## When to Use

The orchestrator invokes this skill at Step 1 (Route) of every cycle, after reading
the active plan. The skill analyzes project state and user intent, then returns a
workflow destination with a confidence level.

## Decision Inputs

1. **Active plan progress tracker** — which modules are ✅ (done) vs ⬜ (pending)
2. **Active plan phase/milestone** — which milestone is active (M3 porting, M4 output, M5 modes, M6 validation)
3. **User's request text** — natural language from the user (if any)

## Workflow Catalog

| Destination | Trigger | Description |
|-------------|---------|-------------|
| `per-module-gate-cycle` | Incomplete modules in tracker | 7-Step Cycle: Risk → Implement → Validate → Audit → Expand → Review |
| `e2e-config-diagnosis` | All modules ✅ + E2E gate ⬜ | Final Gate: artifact-backed evidence intake from the existing full-scope red report → rerun the same full declared scope only if the diagnosis-ready/full-scope contract fails → reviewed diagnosis plan file → diagnosis + repair handoff → reviewed repair plan file → repair → rerun the same full scope |
| `targeted-fix` | User says "fix mismatch" / "fix parity" / names a specific bug | Dispatch shard-diagnosis → mismatch-repair → verify |
| `parity-check` | User says "run parity" / "test module" / names a module to test | Dispatch module-parity-test for specified module |
| `audit` | User says "audit" / "logic review" / names a module to audit | Dispatch logic-parity-audit for specified module |

The `e2e-config-diagnosis` route is the canonical full-scope final gate. User-approved diagnostic reruns may happen inside that workflow, but they do not replace the canonical full-scope route. The route uses `plan-duck` only before the combined Phase 2/3 diagnosis-handoff dispatch and before the repair dispatch. It is not inserted before mechanical verification reruns.

## Decision Logic (Priority Order)

Apply these rules in order. Stop at the first match.

### Priority 1: Explicit User Request
If the user's request explicitly names a workflow or action:
- "fix mismatch", "fix parity", "repair" → `targeted-fix` → **HIGH**
- "run parity", "test module", "check parity" → `parity-check` → **HIGH**
- "audit", "logic review", "compare methods" → `audit` → **HIGH**
- "run E2E", "config gate", "final gate" → `e2e-config-diagnosis` → **HIGH**
- "port module", "implement", "next module" → `per-module-gate-cycle` → **HIGH**

### Priority 2: All Modules Done + E2E Pending
If the active plan shows ALL pipeline modules as ✅ AND the E2E config gate is ⬜:
- → `e2e-config-diagnosis` → **HIGH**

### Priority 3: Incomplete Modules
If the active plan has any pipeline modules still ⬜:
- → `per-module-gate-cycle` (for the next incomplete module) → **HIGH**

### Priority 4: Later Phase (M4/M5/M6)
If the active plan milestone is M4, M5, or M6:
- → **MEDIUM** confidence (workflows not yet defined for these phases)
- Present the user with the milestone description and ask what they want to do

### Priority 5: Ambiguous
If none of the above match:
- → **LOW** confidence
- Present all available workflow options and let the user choose

## Confidence Model

### HIGH Confidence
- **Action:** Log the routing decision as a brief notification in chat, then proceed automatically.
- **Format:** `> Routing: {destination} — {reason}`
- **Example:** `> Routing: per-module-gate-cycle — Active plan shows sv_processor pending`

### MEDIUM Confidence
- **Action:** Confirm with user via `vscode_askQuestions` using a single question with the recommended option pre-selected.
- **Format:** One question with 2-3 options, recommended option marked.

### LOW Confidence
- **Action:** Present full option list via `vscode_askQuestions`, let user choose.
- **Format:** One question listing all available workflows with descriptions.

## Agent References

This skill is read by:
- **🥖Parity-Orchestrator** — at Step 1 of every cycle

The workflow destinations dispatch to:
- **Parity Verifier** — module-parity-test, logic-parity-audit, shard-diagnosis, config-e2e-diagnosis
- **Port Engineer** — faithful-port, mismatch-repair
- **Module Analyst** — risk assessment (within per-module-gate-cycle)
- **Review Gate** — change-impact-review (within per-module-gate-cycle)
- **Gerneral-Purpose Agent** — ad-hoc tasks

## Extending This Skill

When M4/M5/M6 workflows are defined:
1. Add the new workflow to the Catalog table
2. Add trigger conditions to the Decision Logic (at the appropriate priority level)
3. Add the workflow block to the orchestrator agent file
4. No changes needed to the confidence model or agent references