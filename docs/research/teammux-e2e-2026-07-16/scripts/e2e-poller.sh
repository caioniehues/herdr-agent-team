#!/bin/bash
# E2E evidence poller: capture non-empty inbox files mid-flight (they drain
# on read) + task-file status transitions, for teams created after START.
EV="$HOME/Projects/herdr-agent-team/docs/research/teammux-e2e-2026-07-16/captures"
mkdir -p "$EV"
START=$(date +%s)
echo "$START" > "$EV/poll-start-epoch.txt"
while [ $(( $(date +%s) - START )) -lt 900 ]; do
  for f in "$HOME"/.claude/teams/*/inboxes/*.json; do
    [ -e "$f" ] || continue
    [ "$(stat -c %Y "$f")" -ge "$START" ] || continue
    s=$(cat "$f" 2>/dev/null)
    if [ -n "$s" ] && [ "$s" != "[]" ]; then
      h=$(printf '%s' "$s" | md5sum | cut -c1-8)
      team=$(basename "$(dirname "$(dirname "$f")")")
      out="$EV/inbox-$team-$(basename "$f" .json)-$h.json"
      [ -e "$out" ] || printf '%s' "$s" > "$out"
    fi
  done
  for d in "$HOME"/.claude/tasks/*/; do
    [ -d "$d" ] || continue
    [ "$(stat -c %Y "$d")" -ge "$START" ] || continue
    team=$(basename "$d")
    mkdir -p "$EV/tasks-$team"
    cp -u "$d"[0-9]*.json "$EV/tasks-$team/" 2>/dev/null
    for tf in "$d"[0-9]*.json; do
      [ -e "$tf" ] || continue
      jq -c --arg t "$(date +%s)" '{t:$t,id,status,owner,blockedBy}' "$tf" 2>/dev/null
    done >> "$EV/task-status-log.jsonl"
  done
  sleep 0.2
done
echo POLLER-TIMEOUT >> "$EV/poller-exit.txt"
