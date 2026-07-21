#!/usr/bin/env bash
# session-history.sh — durable per-session working log that survives compaction.
#
# Two modes:
#   (no args, JSON on stdin)  "load"  — invoked by the SessionStart hook. Reads
#                                       session_id from the hook payload and, if a
#                                       log exists for it, prints it back as
#                                       additionalContext so the (possibly
#                                       just-compacted) agent starts already
#                                       holding its curated working notes.
#   path <session-id>                 — prints the log file path (creating the
#                                       dir). Used by the session-history skill so
#                                       the agent knows where to append.
#
# Only files survive compaction; this script is the "guarantee" layer that pairs
# with the session-history skill (the "discipline" layer). See
# .claude/skills/session-history/SKILL.md.
set -euo pipefail

PROJECT_DIR="${CLAUDE_PROJECT_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
HIST_DIR="$PROJECT_DIR/.claude/session-history"

case "${1:-load}" in
  path)
    sid="${2:?usage: session-history.sh path <session-id>}"
    mkdir -p "$HIST_DIR"
    printf '%s/%s.md\n' "$HIST_DIR" "$sid"
    ;;
  load)
    # Payload (JSON) arrives on stdin; the load logic lives in a sibling .py so
    # stdin stays free for the payload (a `python3 -` heredoc would consume it).
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    exec python3 "$SCRIPT_DIR/session-history-load.py" "$HIST_DIR"
    ;;
  *)
    echo "usage: session-history.sh [load | path <session-id>]" >&2
    exit 2
    ;;
esac
