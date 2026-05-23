---
name: workflow-management
description: >
  Mandatory protocol for modifying VarDict-rs workflow infrastructure — agents, skills,
  instructions, test harness, CI workflows, or parity scripts. Use whenever the task
  involves creating, editing, renaming, or deleting any agent (.agent.md), skill
  (SKILL.md), instruction (.instructions.md), test file (tests/), CI workflow
  (.github/workflows/), parity script (scripts/), or test configuration (Cargo.toml
  dev-dependencies). Also trigger when adding tools to an agent, changing agent routing,
  modifying skill descriptions, or restructuring the test tier system. Do NOT use for
  porting Rust code, fixing parity mismatches, or running tests — those have their own
  skills. This skill exists because workflow changes have high blast radius: a single
  edit can break agent routing, stale cross-references, orphan skills, or silently
  disable test coverage. Also use when you need to understand, orient on, or inspect
  the VarDict-rs workflow structure (agents, skills, CI wiring, test tiers) without
  making any changes. In orientation mode, read `references/workflow-structure.md`
  directly — no phase protocol required.
---

# Workflow Management

Workflow infrastructure changes are rare but high-impact. A single stale agent name
breaks an orchestrator dispatch. A missing tool grant silently disables a capability.
A renamed test file drops coverage without warning. A stale description can keep an
otherwise-correct agent or skill from ever being invoked. This skill prevents those
failures by requiring full context before any edit.

In change mode, the protocol has five phases. No phase may be skipped or abbreviated.

## Mode Selection

This skill operates in two modes depending on what the user needs:

**Orientation Mode** — Use when the goal is to understand or inspect the workflow structure, not change it. Triggers include phrases like "understand the workflow", "what skills exist", "how does the workflow work", "show me the workflow structure", "what agents are there", "inspect workflow", "orient on the workflow". In orientation mode: read `references/workflow-structure.md` and answer from it. No phase protocol required.

**Change Mode** — Use when any infrastructure file will be created, modified, deleted, or renamed. This is the default when a modification is requested. Triggers are the same as the skill description's main use case. In change mode: run the full Phase 1–6 protocol below.

---

## Phase 1: Full Context Load

Before analyzing or proposing any change, read **every** infrastructure file listed
below. Not frontmatter. Not summaries. The full content. Agents are stateless — the
only way to reason about cross-cutting impact is to have the full picture in context.

### 1.1 Agent Files

Read every `.agent.md` under `.github/agents/`:

| File | What to extract |
|------|-----------------|
| Each agent file | Name, description, tools list, model, agents list, user-invocable, disable-model-invocation, any file-level purpose note, full body instructions |

> See `references/workflow-structure.md` → Agents for the current list.

### 1.2 Skill Files

Read every `SKILL.md` under `.github/skills/*/`:

| Skill | What to extract |
|-------|-----------------|
| Each skill | Name, description, trigger contexts, which agents reference it, any agent names mentioned in the body, any file paths referenced, workflow phases |

> See `references/workflow-structure.md` → Skills for the current list.

### 1.3 Instruction Files

Read every `.instructions.md` under `.github/instructions/`:

> See `references/workflow-structure.md` → Instructions for the current list.

For each instruction file, extract `description`, `applyTo`, and any file-level
purpose note in addition to the rule body.

### 1.4 Test Harness

Read every file under `tests/`:

> See `references/workflow-structure.md` → Test Harness for the current file inventory.

`parity_e2e_sweep` is the full-BAM E2E parity tier, cost-gated, with 3 `#[ignore]`
tests (one per BAM tag). `parity_e2e_sweep_somatic` is the full-pair somatic sweep
tier, also cost-gated, with 1 `#[ignore]` test for `wes_il_pair`.

Note module names, fixture paths, test function names, `#[ignore]` annotations, and
which modules each test covers.

### 1.5 CI Workflows

Read every `.yml` under `.github/workflows/`:

> See `references/workflow-structure.md` → CI Workflows for the current list.

Note triggers, job names, environment variables, test commands, and which test files
each workflow runs.

### 1.6 Scripts

Read every script under `scripts/`:

> See `references/workflow-structure.md` → Scripts for the current list.

### 1.7 Build Configuration

Read `Cargo.toml` — specifically `[dev-dependencies]`, `[profile.debug-release]`, and
any `[[test]]` or `[[bench]]` sections.

### 1.8 Mission Context (optional)

If the change is motivated by a project goal or active plan, read the relevant mission
files under `copilot-office/missions/` to understand the broader context:

- `copilot-project-plan.md` — the project's vision and workflow
- `copilot-active-plan.md` — the current active goal and gate cycle

This helps ensure that workflow changes align with the project's direction rather than
being made in isolation. Skip this step only if the change is purely mechanical (e.g.,
fixing a typo in an agent file) with no strategic dimension.

### Phase 1 Completion Gate

You may NOT proceed to Phase 2 until you have read every file listed above. If a file
is missing or inaccessible, note it in the impact report — do not silently skip it.

---

## Phase 2: Impact Analysis

With full context loaded, analyze the proposed change against the complete infrastructure.

### 2.1 Dependency Map

For the proposed change, identify:

1. **Direct targets** — Which files will be created, modified, or deleted?
2. **Upstream dependents** — What dispatches to, references, or routes through the targets?
   - Agent → agent routing (check `agents:` lists in frontmatter)
   - Agent → skill references (check skill names in agent bodies)
   - Skill → agent references (check agent names in skill bodies)
   - Skill → skill references (check skill cross-references)
   - CI workflow → test file mapping (check `cargo test` commands)
   - CI workflow → script invocations
   - Instruction → file pattern coverage (`applyTo` globs)
3. **Downstream consumers** — What does the target dispatch to, reference, or depend on?
4. **Cross-references** — Any name strings that would become stale (agent names, skill
   names, file paths, tool names)?

### 2.2 Test Coverage Impact

For the proposed change, assess:

- Does it add, remove, or rename test files?
- Does it change which tests are `#[ignore]`d?
- Does it affect the ignored tests allowlist?
- Does it change CI workflow triggers or test commands?
- Does it affect fixture generation scripts?
- Does it modify `Cargo.toml` test configuration?

### 2.3 Invocation & Reachability Analysis

Every workflow component exists to be used. A new agent nobody can dispatch, a skill no
description triggers, or a CI workflow with no matching event is dead on arrival. For
every component being created or modified, trace its activation path.

**Entry point** — How is this component triggered?

| Component type | Activation mechanism |
|---------------|---------------------|
| Agent | `user-invocable: true` for direct use; listed in another agent's `agents:` for dispatch; model-invoked unless `disable-model-invocation: true` |
| Skill | Description-match triggers loading; agent body may reference it explicitly |
| Mode | Registered in system prompt `modeInstructions`; user selects via mode switcher |
| Instruction | `applyTo` glob matches active file paths automatically |
| CI workflow | Push/PR/schedule/`workflow_dispatch` triggers in YAML `on:` block |
| Script | Called from CI workflow, from another script, or run manually |

**Reachability** — Can the component actually be reached?

- New agent → Is it in at least one dispatcher's `agents:` list? Does
  `disable-model-invocation` block intended callers?
- New skill → Does the description cover the intended trigger phrases? Is at least one
  agent able to load it (skill scope)?
- New mode → Is it wired into the mode selection mechanism? Can users switch to it?
- New CI workflow → Does the `on:` trigger match the intended event? Are required
  secrets/environments configured?
- Modified agent → Does removing a tool or changing routing leave any downstream skill
  or workflow unreachable?

**Consumer intent** — Who is the intended user?

| Consumer | Examples |
|----------|---------|
| User-facing | Human invokes agent directly, selects a mode, runs a script manually |
| Agent-facing | Orchestrator dispatches to agent; agent loads skill via description match |
| CI-facing | GitHub event triggers workflow; workflow calls script |
| Hybrid | Multiple activation paths (e.g., script used both by CI and manually) |

Flag any component that has **no reachable activation path** — it needs wiring before
the change is complete. Include reachability findings in the Impact Report (Phase 3).

### 2.4 Description & Docstring Contract

Treat frontmatter descriptions and file-level docstrings as part of the workflow
surface, not optional prose. A workflow change is incomplete if the activation or
purpose text becomes stale.

For every modified component, answer these questions:

1. Does it have a `description:` field, file-header summary, or purpose docstring?
2. Would the proposed change make that text stale, vague, or misleading?
3. Does the text explain both what the component does and how or when it is used?

Component-specific rules:

- Agent: update `description:` when role, routing, tools, or dispatch intent changes.
  It must state what the agent does and when another agent or the user should invoke it.
- Skill: update `description:` whenever trigger scope, supported tasks, or workflow
  shape changes. It must include concrete trigger contexts or phrases, not just a label.
- Instruction: update `description:` and `applyTo:` together when rule scope changes.
- Workflow, script, or test helper: update any leading purpose comment, usage block,
  or docstring when inputs, outputs, prerequisites, or invocation path change. Add a
  short header only when the file's role would otherwise be unclear; do not add boilerplate.
- Rename or move: update embedded examples, paths, and quoted identifiers inside
  descriptions and docstrings, not just code references.

Flag any file where the implementation changed but the descriptive surface did not.

### 2.5 Risk Classification

Classify the change:

| Risk | Criteria |
|------|----------|
| **LOW** | Single file edit, no cross-references, no test impact, reachability unchanged |
| **MEDIUM** | Multiple files, has cross-references but all identifiable, no test regression risk |
| **HIGH** | Agent routing change, skill rename, test tier restructure, CI workflow modification, new component with no activation path yet |

---

## Phase 3: Impact Report

Present the following structured report to the requester **before making any edits**.
Do not proceed to implementation until the report is acknowledged.

```
## Workflow Change Impact Report

### Proposed Change
{One-paragraph description of what is being changed and why}

### Risk Classification: {LOW | MEDIUM | HIGH}

### Files to Modify
| File | Change Type | Description |
|------|-------------|-------------|
| path/to/file | create/modify/delete/rename | What changes |

### Cross-Reference Updates Required
| Source File | Reference | Current Value | New Value |
|-------------|-----------|---------------|-----------|
| (or "None identified") |

### Test Coverage Impact
- Tests added/removed/modified: {list or "None"}
- Ignored test changes: {list or "None"}
- Allowlist updates needed: {yes/no}
- CI workflow impact: {description or "None"}

### Description & Docstring Updates
| File | Surface | Update Needed | Reason |
|------|---------|---------------|--------|
| {path or "None identified"} | description/frontmatter/docstring/comment | {yes/no} | {why} |

### Invocation & Reachability
| Component | Type | Entry Point | Reachable? | Consumer |
|-----------|------|-------------|------------|----------|
| {name} | agent/skill/mode/workflow/script | {how triggered} | {yes/no — details} | {user/agent/CI/hybrid} |
{or "No new/modified components — reachability unchanged"}

### Dependency Chain
{Mermaid diagram or bullet list showing the routing/reference chain affected}

### Risks and Mitigations
| Risk | Mitigation |
|------|------------|
| {What could go wrong} | {How the implementation addresses it} |
```

---

## Phase 4: Implementation

After the impact report is acknowledged, implement the changes. Follow these rules:

### 4.1 Ordering

Apply changes in dependency order:
1. Leaf files first (files nothing depends on)
2. Then files that reference the leaves
3. Then routing/dispatch files last

This ensures that at no intermediate point do cross-references point at nonexistent
targets.

### 4.2 Cross-Reference Consistency

After every file edit, verify that:
- Every agent name referenced in skills matches an actual `.agent.md` `name:` field
- Every skill name referenced in agents matches an actual skill directory name
- Every `agents:` list entry in agent frontmatter matches an actual agent `name:` field
- Every tool in an agent's `tools:` list is intentional (not leftover from copy-paste)
- Every file path referenced in skills/instructions points to an existing file

### 4.3 Naming Conventions

When creating new files, follow existing conventions:
- Agents: `.github/agents/{kebab-case}.agent.md`
- Skills: `.github/skills/{kebab-case}/SKILL.md`
- Instructions: `.github/instructions/{kebab-case}.instructions.md`
- Tests: `tests/parity_suite/{module_name}.rs`, `tests/parity_sweep_suite/{module_name}_sweep.rs`
- Scripts: `scripts/{snake_case}.sh` or `scripts/{snake_case}.py`

### 4.4 Description and Docstring Hygiene

For each modified component:

- Keep frontmatter `description:` aligned with the current behavior, activation path,
  and intended consumer.
- Prefer descriptions that answer both "what does this do?" and "when should it be
  used or triggered?"
- If a file already has a purpose comment or docstring, update it when semantics,
  prerequisites, or outputs change.
- If no header text exists, add one only when the file's role or invocation path would
  otherwise be unclear.
- Remove stale names, old paths, and outdated trigger phrases from examples and
  narrative text.

---

## Phase 5: Validation

After implementation, verify that nothing is broken.

### 5.1 Build Check

```bash
source activate vdr
export LIBCLANG_PATH="$CONDA_PREFIX/lib"
cargo test --profile debug-release --no-run
```

This compiles all test targets without running them. If it fails, the change broke
something — fix before proceeding.

### 5.2 Cross-Reference Scan

Run a grep-based scan for stale references:

1. Extract all agent `name:` values from `.github/agents/*.agent.md`
2. Search all `.github/` files for agent name strings — flag any that don't match a
   known agent name
3. Extract all skill directory names from `.github/skills/*/`
4. Search all `.github/` files for skill name strings — flag any that don't match a
   known skill

Report findings. Fix any stale references before declaring the change complete.

### 5.3 Description / Docstring Sanity Check

For every modified agent, skill, or instruction:

- verify `description:` still matches the actual behavior and activation path
- verify renamed files or components are reflected in examples and narrative text

For modified workflows, scripts, or tests that already carry a purpose comment or
usage block:

- verify prerequisites, inputs, outputs, and invocation notes are still accurate

### 5.4 CI YAML Validation

If any workflow YAML was modified, validate syntax:
```bash
python -c "import yaml; yaml.safe_load(open('.github/workflows/{file}.yml'))"
```

### 5.5 Test Execution (if tests were affected)

If the change modified test files, test configuration, or fixture scripts:
```bash
cargo test --profile debug-release -- --include-ignored --skip parity_config_e2e_cell_
```

### 5.6 Reference File Sync

After all other validation steps, verify that `references/workflow-structure.md` accurately reflects the change just made:

- If the change added, removed, or renamed any agent, skill, instruction, test file, CI workflow, or script: update `references/workflow-structure.md` accordingly.
- If the reference is already current: explicitly confirm in the Validation Report — "Reference file verified, no update needed."

This step is mandatory even when no update is needed. A change-mode run that omits the reference-sync check leaves the reference file unverified.

### 5.7 Validation Report

```
## Validation Results

- Build check: {PASS | FAIL — details}
- Cross-reference scan: {CLEAN | {N} stale refs found — details}
- Description/docstring sanity: {PASS | FAIL — details}
- CI YAML validation: {PASS | FAIL | N/A}
- Test execution: {PASS | FAIL | N/A}
- Reference file sync: {UPDATED | VERIFIED-NO-CHANGE | FAIL — details}
- Ignored tests allowlist: {up-to-date | needs update — details}
```

---

## Phase 6: Work Report

After Phase 5 passes, write a work report so that other agents can understand what
changed and why without re-reading the entire infrastructure.

### 6.1 File Location

Write the report in workspace memory with file name: `wf-change-{short-slug}-{YYYYMMDD}.md`

where `{short-slug}` is a 2–4 word kebab-case summary of the change (e.g.
`add-workflow-inspector-skill`, `rename-shard-debugger`, `add-somatic-parity-test`).

### 6.2 Report Template

```markdown
# Workflow Change: {title}

**Date:** {YYYY-MM-DD}
**Risk:** {LOW | MEDIUM | HIGH}
**Status:** COMPLETE

## What Changed

{One or two sentence summary of the change.}

## Files Modified

| File | Change | Notes |
|------|--------|-------|
| {path} | created / modified / deleted / renamed | {brief reason} |

## Cross-References Updated

{List of files where a reference was updated, or "None."}

## Reachability After Change

{For each new or modified component: how is it now activated?
Or "No reachability changes." if no new components were added.}

## Why This Change Was Made

{Context sentence or two: what prompted this, what it unlocks or fixes.}

## Impact on Existing Agents

{Call out any agent whose behavior or invocation path is affected.
Or "No impact on existing agents." if none.}

## Known Gaps or Follow-Ups

{Optional. List anything deferred or intentionally left out.
Or "None." if the change is self-contained.}
```

### 6.3 Rules

- Write the report even for LOW-risk changes. Other agents read these to stay current.
- Keep every section concise — bullet points preferred over prose.
- Do not copy-paste the Phase 3 impact report verbatim; the work report is a
  post-implementation summary for other agents, not a pre-approval checklist.
- Date the file with the actual implementation date, not the request date.
- Announce the report path in chat after writing it so the user and any supervising
  orchestrator can find it.

---

## Maintaining This Skill

This skill references specific files and counts that exist in the repository today.
When infrastructure changes (new agents, new skills, new test tiers), update the
`references/workflow-structure.md` to match reality. The reference file is the single
authoritative inventory — if it's stale, the whole point of the skill (complete
context) is undermined. The SKILL.md procedural steps rarely need updating for
routine infrastructure changes, but may need edits for new component types or
structural process changes.
