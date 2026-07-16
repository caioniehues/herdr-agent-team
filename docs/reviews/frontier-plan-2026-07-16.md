# Frontier execution plan — #66–#83 (drafted 2026-07-16, not yet dispatched)

> **SUPERSEDED 2026-07-16 by ADR-0012 (pivot to herdmates).** All tickets
> #66–#83 closed wontfix; batch worktrees deleted. Kept as record only.

State when this was written: the implementation-review program (#46–#65) is
fully executed and merged to `integrate/program-wave1` (197 tests, fmt/clippy
clean, **not pushed** — merge to main is a release and waits for Caio). The
slice reviews filed frontier tickets #66–#83. Four batch worktrees were
prepared off `integrate/program-wave1` but **no work was dispatched** — all
worker panes were closed on Caio's instruction. The worktrees are empty
(HEAD = 18931b0, no commits):

| Worktree | Branch | Tickets |
|---|---|---|
| `~/Projects/herdr-agent-team-loops/fix-teardown` | `fix/teardown-batch` | #74, #75, #76, #78 |
| `~/Projects/herdr-agent-team-loops/fix-hook` | `fix/hook-batch` | #66, #67, #68, #70 |
| `~/Projects/herdr-agent-team-loops/fix-godcli` | `fix/godcli-batch` | #71, #72 |
| `~/Projects/herdr-agent-team-loops/fix-msg` | `fix/msg-batch` | #80, #81, #82, #83 |

## Phase A batches + coordinator-decided precedents

These precedents were decided by the coordinator during batching; encode them
verbatim into worker briefs when dispatching. RED-first applies to every
behavior fix.

### fix-teardown (#74, #75, #76, #78) — HIGH priority batch

- **#74 (HIGH):** worktree removal failures currently escape the
  failed-lifecycle threading added by loop #49 — thread them through
  `WorkerLifecycle::Failed` like other teardown failures.
- **#75 (HIGH):** dirty-worktree refusal is a dead end. Precedent: the
  ended-run path honors `--remove-worktrees` for recorded paths; the
  `DirtyWorktrees` error text documents the manual path; spec §6 documents
  close-before-dirty ordering.
- **#76:** as filed.
- **#78:** S-8 finding = comment only (wontfix that sub-item).

### fix-hook (#66, #67, #68, #70)

- **#67 precedent:** emit `DrainOutbox{god}` on god idle/done flips —
  symmetric with workers, consistent with ADR-0002 pointer injection.
- **#66, #68, #70:** as filed.

### fix-godcli (#71, #72)

- **#71 precedent:** keep the attention ⊇ blocked superset; document it in
  spec §13 and test it through the production `row()` path.
- **#72:** as filed.

### fix-msg (#80, #81, #82, #83)

- **#80 precedent:** minimal fix only — `next_sequence` accounts for `.claim`
  files, plus a test. Skip the single-outbox-module refactor (that stays a
  separate structural ticket if ever prioritized).
- **#81 precedent:** append the delivered event on the direct path too
  (symmetric with the queued path).
- **#82 precedent:** god gets a conservative adopted-style launcher fallback,
  with test.
- **#83 precedents:** F6 sanitize the drain path; F7 reject terminal targets
  with a candidates-style error; F8a stderr warning on multiple active runs;
  F8b add a blocked-enqueue test.

## Phase B (after all Phase A branches merge, on fresh integration)

Sequenced after Phase A to avoid cross-module conflicts:

- **#69:** share the report-ready predicate in a neutral module so hook's
  `reports_present` and god_cli's `report_ready` use one definition.
- **#73:** docs alignment batch.

## Open decisions for Caio (do not implement without a call)

- **#77:** release-notice vs launcher policy.
- **#79:** blocked non-queueing worker cannot receive its unblocking answer
  (deadlock). Needs a design decision, not a patch.
- **Release:** merge `integrate/program-wave1` → main + version bump
  (recommendation: 1.1.0 — behavior changes include `msg --ack`,
  sentinel-gated report readiness, atomic-claim drain, failed teardown
  lifecycles, cfg(unix) socket gating).

## Dispatch contract reminders (from program-wave learnings)

- Claude workers only (codex banned by Caio 2026-07-15). New claude sessions
  crash intermittently at startup (config-borne, unsolved) — prefer reusing
  living sessions; if spawning fresh, verify the session survives before
  briefing.
- Briefs: absolute paths, RED-first, exact gate commands
  (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`),
  report to `docs/reviews/loops/<batch>-report.md` with per-ticket sentinels
  (`FIXNN GREEN`), STATUS.log pings, **never run git/gh** — coordinator
  commits and merges.
- Enter-swallow: after `pane run`, check
  `herdr wait agent-status <pane> --status working`; resend Enter once if
  needed.
