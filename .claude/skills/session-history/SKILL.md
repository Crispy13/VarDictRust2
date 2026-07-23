---
name: session-history
description: >
  Maintain a durable per-session working log so nothing important is lost when
  the conversation is compacted. Use when: starting real work in a long session,
  after making a decision or locking a constraint, when task state changes, at a
  milestone, before a long/risky/irreversible operation, when context is getting
  large, or right before an expected /compact. Also use to read back the log
  after a compaction/resume. The paired SessionStart hook auto-reloads this log,
  so a freshly-compacted agent starts already holding it. This is within-session
  working state — distinct from MEMORY.md (durable cross-session facts).
---

# Session history

## Why this exists

Compaction replaces the live conversation with a summary. Anything important that
the summary omits — decisions, current task state, next steps, constraints,
gotchas — is silently lost, and the post-compaction agent resumes half-blind.

**Only files survive compaction.** So this skill keeps a curated log on disk, and
a `SessionStart` hook (`matcher: startup|resume|compact`, in `.claude/settings.json`
→ `.claude/hooks/session-history.sh`) re-injects it automatically after every
compaction/resume as a `[session-history]` system-reminder. You do not have to
remember to reload it — but you DO have to keep it current. That is your job here.

Two layers:
- **Discipline (this skill):** you append/update the log at the right moments.
- **Guarantee (the hook):** the log comes back on its own after compaction.

## Your session id and log path

Your session id is the **UUID directory in your scratchpad path** (shown in your
system prompt), e.g. `.../<session-id>/scratchpad`. The log lives at
`.claude/session-history/<session-id>.md`.

> **The id changes across resume (≈ daily in a long-lived chatroom).** Always
> re-derive it from the scratchpad path shown in your system prompt each time —
> never trust a path quoted in a compaction summary (that's how a log gets
> orphaned under a stale id). The load hook now auto-recovers a drifted log by
> adopting the newest log and migrating it to the current id, but writing to the
> right name yourself keeps exactly one clean file.

Get the exact path (creates the dir):

```bash
bash .claude/hooks/session-history.sh path <session-id>
```

Write/update the file with the normal Write/Edit tools. Logs are gitignored
(`.claude/session-history/`), so they never pollute `git status` or get committed.

## When to update (cadence)

**Primary trigger — flush right before every `/compact`.** When the user signals a
manual compaction is coming (or you're about to suggest one), bring the log fully
current FIRST. This is the main, deliberate flush point. If the user tends to
compact manually, proactively offer to flush when context is getting large.

**But manual flushing alone is not enough — auto-compaction gives no warning.**
Two kinds of compaction:
- **Manual `/compact`** — you see it coming → the primary flush above covers it.
- **Auto-compaction** (context fills up) — fires with **no warning**; you cannot
  flush "right before" it. Whatever isn't already in the log is lost from the
  curated view (only the raw summary survives).

So the log must ALSO be kept current as you go, so a surprise auto-compact loses
at most a few exchanges. Update whenever the log would otherwise go stale:
- A **decision** is made or a **constraint/goal** is stated by the user.
- **Task state** changes: something starts, finishes, blocks, or is verified.
- A **milestone** completes (e.g. a stage committed, a gate goes green).
- **Before** a long, risky, or irreversible operation, or a batch you might not
  finish before compaction.
- After a compaction/resume: the hook re-injected the log — **read it, act on
  'Next steps', and correct anything now stale.**

Keep it **curated, not a transcript.** It is the minimum a cold agent needs to
continue correctly. Overwrite stale sections in place; don't append endlessly.
Prefer concrete pointers (`file:line`, exact commands, commit hashes) over prose.
Convert relative dates to absolute.

## Template

Create the log (first time) with this shape, then keep it current:

```markdown
# Session log — <session-id>
_Last updated: <YYYY-MM-DD HH:MM>_

## Mission & constraints
- <the overarching goal + hard rules that MUST NOT be violated>

## Current focus
- <the one thing being worked on right now>

## State & progress
- DONE: <verified-complete items, with evidence/commit>
- IN PROGRESS: <what's underway + where it stands>
- BLOCKED: <blockers + what would unblock>

## Decisions (dated)
- <YYYY-MM-DD> <decision> — <why>

## Next steps
1. <the very next action a cold agent should take>
2. ...

## Gotchas & open questions
- <traps, surprises, things awaiting user input>

## Key pointers
- <file:line, commands, hashes, paths worth not re-deriving>
```

## Relationship to MEMORY.md

- **This log** = ephemeral working state for THIS session. Gitignored. Deleted/
  stale-able freely.
- **MEMORY.md** (`.../memory/`) = durable facts that matter across sessions.
  When something in the log proves worth keeping long-term, promote it into a
  memory file per that system's rules — don't leave it only in the session log.

## Verifying the guarantee (optional)

To confirm the hook would reload a log, simulate its payload:

```bash
echo '{"session_id":"<id>","source":"compact"}' | bash .claude/hooks/session-history.sh
```

It prints the `additionalContext` JSON if a non-empty log exists, nothing
otherwise. The real hook feeds the same shape on `startup|resume|compact`.
