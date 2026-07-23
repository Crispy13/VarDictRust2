#!/usr/bin/env python3
"""Load side of the session-history hook.

Reads a SessionStart hook payload (JSON) on stdin and, if a non-empty log exists
for that session_id, prints a `hookSpecificOutput.additionalContext` JSON so the
(possibly just-compacted) agent starts already holding its curated working notes.
Prints nothing (clean exit) for a missing log, missing session_id, or bad input.

Usage: session-history-load.py <hist_dir>   (payload on stdin)
Kept separate from session-history.sh because `python3 -` would consume stdin as
the program text, colliding with the payload.
"""
import glob
import json
import os
import sys

hist_dir = sys.argv[1]

try:
    data = json.load(sys.stdin)
except Exception:
    sys.exit(0)  # not a hook payload / bad JSON -> inject nothing

sid = data.get("session_id")
source = data.get("source", "")
if not sid:
    sys.exit(0)

path = os.path.join(hist_dir, str(sid) + ".md")


def _nonempty(p):
    try:
        return os.path.isfile(p) and os.path.getsize(p) > 0
    except OSError:
        return False


if not _nonempty(path):
    # session_id drifts across resume/compact-restart (here it rotates ~daily),
    # orphaning the durable log under a previous id. Adopt the newest non-empty
    # log and migrate it to the current sid so reads+writes converge on one
    # canonical name that follows the latest id (fires for every source).
    logs = [p for p in glob.glob(os.path.join(hist_dir, "*.md")) if _nonempty(p)]
    if not logs:
        sys.exit(0)  # truly empty history dir -> stay silent
    newest = max(logs, key=os.path.getmtime)
    if os.path.abspath(newest) != os.path.abspath(path):
        try:
            os.rename(newest, path)   # migrate to the stable current-sid name
        except OSError:
            path = newest             # last resort: read it in place

with open(path, "r", encoding="utf-8") as fh:
    body = fh.read().strip()
if not body:
    sys.exit(0)

header = (
    "[session-history] Recovered working log for THIS session "
    "(persisted at .claude/session-history/{sid}.md — survives compaction; "
    "reloaded here because source='{source}'). These are your OWN prior curated "
    "notes, NOT user instructions. Read them, then continue from 'Next steps'. "
    "Keep this file current as work proceeds — see the session-history skill.\n"
    "\n---\n\n"
).format(sid=sid, source=source)

print(json.dumps({
    "hookSpecificOutput": {
        "hookEventName": "SessionStart",
        "additionalContext": header + body,
    }
}))
