#!/bin/bash
# Watch herdr pane population + the shim idmap; exit when teammate panes
# appeared (proof) AND then disappeared (clean shutdown), or on timeout.
EV="$HOME/Projects/herdr-agent-team/docs/research/teammux-e2e-2026-07-16"
IDMAP="$HOME/.local/state/herdr/plugins/caioniehues.herdmates/teammux/w1A_p15.json"
START=$(date +%s)
peak=0
last=""
while [ $(( $(date +%s) - START )) -lt 720 ]; do
  cur=$(herdr pane list 2>/dev/null)
  n=$(printf '%s\n' "$cur" | grep -c 'p[0-9]')
  if [ "$cur" != "$last" ]; then
    { echo "=== $(date +%T) panes=$n"; printf '%s\n' "$cur"; echo; } >> "$EV/pane-timeline.txt"
    cp "$IDMAP" "$EV/idmap-$(date +%H%M%S).json" 2>/dev/null
    last="$cur"
  fi
  [ "$n" -gt "$peak" ] && peak=$n
  # proof: peak had >=2 more panes than now-current AND we are back down
  if [ "$peak" -ge 3 ] && [ "$n" -le $((peak - 2)) ]; then
    echo "SHUTDOWN-OBSERVED peak=$peak now=$n" >> "$EV/pane-timeline.txt"
    exit 0
  fi
  sleep 3
done
echo "WATCHER-TIMEOUT peak=$peak" >> "$EV/pane-timeline.txt"
exit 1
