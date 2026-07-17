# E2E attempt 2 — PASS (2026-07-17)

Re-run after the `list_pane_ids` lazy-registration fix (2c0a36b). Lead
launched via `herdmates teammux-launch --model sonnet` → pane `w1A:p16`,
lead session `5a22f79a`, team `session-5a22f79a`.

## Verdict: PASS

- Teammate spawn through the shim worked: pane count 2 → 4 → 2
  (`pane-timeline.txt`, `SHUTDOWN-OBSERVED peak=4`). Teammates alpha/beta
  were REAL herdr panes `w1A:p17` / `w1A:p18`.
- Idmap growth proves the mapping: `idmap-1444*.json` go from
  `{%0: w1A:p16}` to `{%0: p16, %1: p1, %2: p17, %3: p18}`. `%1` is the
  coordinator's foreign pane, lazily minted — the exact attempt-1 blocker
  code path, exercised live.
- Task DAG ran: 1 (alpha) + 2 (beta) completed, then 3 (lead,
  blockedBy [1,2]) completed (`captures/task-status-log.jsonl`;
  transitions pending→completed observed).
- Lead printed `E2E-COMPLETE` (lead transcript
  `~/.claude/projects/.../5a22f79a-*.jsonl`; only other "E2E-FAILED"
  occurrence is the prompt text itself). Both teammates dismissed cleanly.

## New schema data (feed #92)

- **Inbox entry schema captured** (the #88/#91 gap), e.g.
  `captures/inbox-session-5a22f79a-alpha-3c7938bd.json`:
  `{from, text, timestamp (ISO), msgV: 1, msg_id (uuid), type: "message",
  read: false}`.
- **Dismissal prunes members from `config.json`** — final config
  (`captures/team-config-final-5a22f79a.json`) lists only `team-lead`
  (its `tmuxPaneId: "leader"`, `backendType: "in-process"`). Teammate
  `tmuxPaneId` values were NOT captured mid-flight (poller doesn't
  snapshot config) — open datum; idmap covers the pane binding anyway.
- Team creation is implicit: spawning a named teammate joins/creates the
  session team; no explicit create-team verb (lead's own observation).

## Watcher fix

`scripts/e2e-watcher.sh` counted panes with `grep -c` over single-line
JSON (always 1); now `jq '.result.panes | length'`, and IDMAP path
updated per-run.
