# Task Plan — Issue #84 (feat/84-pivot-d1)

## Goal

Execute BRIEF.md exactly: issue #84 steps 1–6 (rename to herdmates + D1
sidebar-token agent board foundation), each step leaving fmt/clippy/tests
green, pausing at each step boundary for the coordinator's commit.
Extended by the coordinator to include step 7 (sidebar-rows doc) once the
prototype verdict landed. Step 8 (release) remains the coordinator's.

## Contract

- Workspace: `/home/caio/Projects/herdmates-issue84` (worktree, branch
  `feat/84-pivot-d1`).
- Never commit/push/mutate git — coordinator commits between steps.
- Gates every step: `cargo fmt --check`, `cargo clippy --all-targets -- -D
  warnings`, `cargo test`.
- Step boundary protocol lives in `PROGRESS.md` (BRIEF's own format) — this
  file is the planning-with-files layer, additive, does not replace it.
- Standing addition (mid-step-5): every READY/BLOCKED line in PROGRESS.md
  is followed by `herdr pane run w1A:p1 'BUILD:84 STEP n
  READY|BLOCKED — read .../PROGRESS.md'` to notify the coordinator pane.

## Phases

### Phase 1 — rename crate + binary
Status: complete
- Cargo.toml, herdr-plugin.toml, src/main.rs, src/paths.rs renamed
  herdr-agent-team → herdmates. Committed by coordinator.

### Phase 2 — mark legacy surface frozen
Status: complete
- Freeze doc comments on 7 legacy modules; README rewritten (herdmates
  pitch + labeled legacy section). Committed.

### Phase 3 — teamfiles module (pure logic)
Status: complete
- `src/teamfiles.rs`: parse team config.json + inbox JSON → TeamConfig/
  Member/InboxMessage/Teammate. 12 tests, 6 fixture dirs. Committed.

### Phase 4 — tokens module (pure logic)
Status: complete
- `src/tokens.rs`: Teammate → TokenSet (task/status/model), 80-char
  truncation, 16-token budget. 7 tests. Committed.

### Phase 5 — board pump subcommand
Status: complete
- Discover team files under `~/.claude/teams/`, resolve teammate panes,
  emit `pane report-metadata --token` calls via HERDR_BIN_PATH.
- Process-spawning behind HerdrApi trait (existing pattern), recording
  FakeHerdr fixture for tests.
- See findings.md for live-verified CLI/schema facts this design rests on.
- Subtasks:
  - [x] Verify live `herdr pane report-metadata` flags (`--token` exists,
        repeatable, distinct from `--state-label`)
  - [x] Verify live schema `tokens` field (maxProperties 16, name pattern)
  - [x] Verify pane→session resolution path (`agent_list().agent_session.value`
        matches `leadSessionId`)
  - [x] Re-snapshot docs/herdr-api-schema.snapshot.json (was stale — missing
        `tokens` field entirely)
  - [x] Add `pane_report_tokens` to HerdrApi trait + HerdrClient + FakeHerdr
  - [x] Write `src/pump.rs`: discover_team_dirs, pump_once, resolve_lead_pane
  - [x] Wire `pump-board` subcommand into main.rs
  - [x] Unit tests + one integration smoke (stable argv snapshot)
  - [x] Live smoke test against real ~/.claude/teams/ (3 leads resolved,
        non-destructive token publish confirmed)
  - [x] Run gates, log PROGRESS.md step 5 boundary, pause

### Phase 6 — wire pump into manifest event handlers
Status: complete
- Wired at `hook_command()` (the untested, real-env entrypoint shared by
  ALL `on-agent-status` manifest events), not inside the heavily-tested
  `on_agent_status` — avoids coupling 20+ existing tests to a live
  `~/.claude/teams` filesystem read. herdr-plugin.toml needed NO changes:
  the existing `[[events]]` entries already route to the binary command
  that now also runs the debounced pump.
- Debounce via a marker file under `state_dir` (2000ms window); 4 new
  tests (maybe_pump_at) + a live simulated hook invocation confirming
  first-call-runs / immediate-second-call-debounced.

### Phase 7 — sidebar-rows doc (scope extension)
Status: complete — coordinator confirmed scope (steps 1-7) done, all
committed. Standing by idle for review-fix requests before merge.
- Coordinator unblocked this at the prototype-verdict landing, extending
  BRIEF's original 1–6 scope. Folded in 5 verified findings supplied by
  the coordinator (also on issue #84 comment): builtin is `state_text`
  not `state_label`; invalid token → reload-config `"partial"` + silent
  stale UI; token values must be telegraphic (~20 visible chars,
  sidebar_width default 26/max 36); agent-less panes never appear;
  rows config is one global table, not per-agent-id.
- `docs/sidebar-rows.toml` written to match the exact live syntax found in
  `~/.config/herdr/config.toml` (a prototype-spike snippet already present
  on this machine, comment-tagged "herdmates D1 prototype") — not guessed.
- README: new "Agent board setup (D1)" section + doc-map entry.
- Docs-only step; gates unaffected (229 tests, unchanged count).

## Decisions Log

- Step 4: token names `$task/$step` from issue text — no `step` concept
  exists in the domain model, substituted `model`. Logged in PROGRESS.md.
- Step 5: resolution scope limited to the team LEAD only. Only the lead's
  session ID is recorded in config.json (`leadSessionId`); teammates carry
  no independently resolvable session ID pre-shim, so non-lead members are
  always skipped (matches ADR-0012's "skip, never error" degrade policy).
  See findings.md for the live evidence this rests on.
- Step 5: `docs/herdr-api-schema.snapshot.json` was stale (missing `tokens`
  entirely — CLAUDE.md mandates re-snapshot after any herdr update).
  Re-snapshotting as part of this step since D1 depends directly on the
  `tokens` field being accurately documented.
