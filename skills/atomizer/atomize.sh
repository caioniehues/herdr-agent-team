#!/bin/sh
# Write a next action (and optionally a task) into the herdmates focus
# file, preserving its existing Task (unless overridden) and Decisions
# sections. Self-contained POSIX sh — no dependency on the herdmates Rust
# crate/binary; see skills/atomizer/SKILL.md and docs/focus-file.md for
# the contract this must produce.
set -eu

NEXT_ACTION="${1:?usage: atomize.sh <next-action> [task]}"
TASK_OVERRIDE="${2:-}"

FOCUS_FILE="$HOME/.local/share/herdmates/focus.md"
FOCUS_DIR=$(dirname "$FOCUS_FILE")
mkdir -p "$FOCUS_DIR"

TASK_BODY=""
DECISIONS_BODY=""

if [ -f "$FOCUS_FILE" ]; then
    TASK_BODY=$(awk '
        BEGIN { section = "" }
        /^##[[:space:]]*[Tt][Aa][Ss][Kk][[:space:]]*$/ { section = "task"; next }
        /^##/ { section = ""; next }
        section == "task" { print }
    ' "$FOCUS_FILE")

    DECISIONS_BODY=$(awk '
        BEGIN { section = "" }
        /^##[[:space:]]*[Dd][Ee][Cc][Ii][Ss][Ii][Oo][Nn][Ss][[:space:]]*$/ { section = "decisions"; next }
        /^##/ { section = ""; next }
        section == "decisions" && /^-[[:space:]]*\[[ xX]\]/ { print }
    ' "$FOCUS_FILE")
fi

if [ -n "$TASK_OVERRIDE" ]; then
    TASK_BODY="$TASK_OVERRIDE"
fi

TMP_FILE=$(mktemp "$FOCUS_DIR/.focus.XXXXXX")
trap 'rm -f "$TMP_FILE"' EXIT

{
    printf '# Focus\n\n'
    printf '## Task\n%s\n\n' "$TASK_BODY"
    printf '## Next Action\n%s\n\n' "$NEXT_ACTION"
    printf '## Decisions\n'
    if [ -n "$DECISIONS_BODY" ]; then
        printf '%s\n' "$DECISIONS_BODY"
    fi
} > "$TMP_FILE"

mv "$TMP_FILE" "$FOCUS_FILE"
trap - EXIT

echo "Wrote next action to $FOCUS_FILE"
