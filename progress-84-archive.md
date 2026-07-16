# Progress Log — Issue #84 (planning-with-files layer)

This file supplements `PROGRESS.md` (BRIEF's own step-boundary protocol,
which remains the authoritative READY FOR COMMIT record). This file logs
session narration for the planning-with-files skill.

## Session 2026-07-16

- Steps 1–4 complete and committed prior to this skill being invoked
  (mid-turn addition to the contract during step 5). Backfilled task_plan.md
  phases 1–4 as complete from PROGRESS.md's existing record.
- Step 5 (board pump) in progress. Spent significant tool-call budget
  verifying live herdr CLI/schema facts before writing any code — findings
  logged in findings.md. Key discovery: docs/herdr-api-schema.snapshot.json
  was stale (missing the `tokens` field entirely), which would have led to
  a wrong design (e.g. reusing legacy `state_labels` single-tuple instead
  of the real `--token NAME=VALUE` repeatable flag) had it not been
  re-verified against the live installed herdr 0.7.4 binary.
- Re-snapshotted docs/herdr-api-schema.snapshot.json against live herdr
  0.7.4 (556 diff lines vs. the committed version — genuinely stale).
- Added `HerdrApi::pane_report_tokens` (+ HerdrClient/FakeHerdr impls),
  `src/pump.rs` (discover_team_dirs, resolve_lead_pane, pump_once, plus
  the `pump-board` CLI wrapper), wired into main.rs dispatch.
- 9 new tests (216 → 225), all pass. fmt/clippy clean (one clippy fix
  earlier in step 4, none needed here).
- Live smoke test: built target/release/herdmates, ran `pump-board`
  against the real ~/.claude/teams/ directory (15 teams on disk). 3 leads
  resolved to live herdr panes via session-id match, including the pane
  running this very session. Verified via `herdr pane get` that the
  published `status=idle` token landed without disturbing a separate,
  pre-existing publisher's `task`/`step` tokens on the same pane —
  confirms the per-source token model is additive/non-destructive.
- Step 5 complete. Logged PROGRESS.md step 5 boundary, paused for commit.
- Mid-step-5: user added standing protocol — every READY/BLOCKED line also
  triggers `herdr pane run w1A:p1 'BUILD:84 STEP n ...'`. Ran it for step 5
  retroactively, recorded the rule in PROGRESS.md/task_plan.md.

## Session 2026-07-16 (continued) — Step 6

- Wired `pump::maybe_pump` into `hook::hook_command()` (not into the
  tested `on_agent_status` — would have coupled 20+ existing tests to a
  live filesystem read of the real `~/.claude/teams`). herdr-plugin.toml
  needed zero changes — existing `[[events]]` entries already route every
  listed event to the binary command that now also runs the pump.
- Added debounce (2000ms window, marker file under state_dir) —
  maybe_pump/maybe_pump_at in pump.rs. 4 new tests (225 → 229), all pass.
  fmt/clippy clean.
- Rebuilt release binary, live-simulated a real `on-agent-status` hook
  invocation via env vars (HERDR_PLUGIN_STATE_DIR pointed at a scratch
  temp dir, synthetic event JSON). Confirmed: first call writes the
  marker, an immediate second call is debounced (marker timestamp
  unchanged). Sent the STEP 6 pane notification.

## Session 2026-07-16 (continued) — Step 7 (scope extension)

- Coordinator unblocked step 7 (prototype verdict landed) and supplied 5
  verified findings to fold in (also on issue #84 comment). Found a live
  `[ui.sidebar.agents]` config already on this machine tagged "herdmates
  D1 prototype" — used its exact syntax as ground truth rather than
  guessing.
- Wrote `docs/sidebar-rows.toml` (copy-paste snippet, heavily commented
  with all 5 findings) + README "Agent board setup (D1)" section.
- Did not independently reproduce the reload-config-partial claim (would
  have required swapping the coordinator's live sidebar config
  mid-session for a claim already given as verified) — took it as
  authoritative per findings.md.
- Docs-only step; gates re-run for contract compliance, unaffected
  (229 tests, same count as step 6).
- STEP 7 committed. Coordinator confirmed the full #84 scope (steps 1-7)
  is done. Standing by idle for possible review-fix requests before
  merge — no further action until the coordinator sends one.
