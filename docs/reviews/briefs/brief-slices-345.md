# Worker briefs — Stage 2 slices 1–3 (#53, #54, #55): READ-ONLY code review

Target: the INTEGRATION branch worktree
`/home/caio/Projects/herdr-agent-team-loops/integration`
(branch `integrate/program-wave1` = v1.0.0 + the five reviewed loop/fix
merges). Review THIS tree — the fixes are part of the reviewed surface.
Absolute paths everywhere. **READ-ONLY: modify nothing except your own report
file. NEVER run `git` (beyond read-only log/show/diff) or `gh`.**

## Shared method (identical to slices 5/6)

Two-axis review: **Standards** (Rust quality, error handling, tests through
the interface) + **Spec** (conformance to the named sources; every claim
cites code file:line AND spec/ADR section). Deep-module lens — answer
explicitly: deletion test; caller leverage through a small interface; seams
where behavior genuinely varies; outcomes tested through the interface.

Findings ranked by severity with file:line, violated source, one-line failure
scenario, suggested disposition (fix-ticket / document / wontfix). NO inline
fixes. The coordinator files issues.

## Slice 1 (#53) — event → durable truth: `src/hook.rs`, `src/reconcile.rs`

Spec sources: `docs/spec.md` §7–8, ADR-0002, ADR-0010.
Context: this slice hosted loops 1/4/5 and defect #59 — all fixed on this
branch (reports_present gate, attention observe-not-consume, task metadata,
atomic-claim drain). Review the fixes IN CONTEXT for depth/locality and for
what they still miss (e.g. hook events the plugin does not subscribe to:
pane.moved/pane.exited/pane.closed/workspace.closed/worktree.removed — the
known stale-board defect class).
Report: `docs/reviews/slices/slice1-event-truth.md` (in the integration worktree).
Sentinel: `SLICE1 DONE` or `SLICE1 BLOCKED: <reason>`.

## Slice 2 (#54) — durable truth read/wait: `src/god_cli.rs`, `src/run.rs`

Spec sources: `docs/spec.md` §10 + god-tools/wait sections.
Context: hosted loop 2 (sentinel readiness — fixed here). The Stage 0
narrowed socket claim lives in slice 6, already reviewed; focus on wait
semantics, inbox snapshot construction, read-marks, dead-worker handling.
Report: `docs/reviews/slices/slice2-read-wait.md`.
Sentinel: `SLICE2 DONE` or `SLICE2 BLOCKED: <reason>`.

## Slice 3 (#55) — teardown: `src/status_kill.rs`

Spec sources: `docs/spec.md` §6, ADR-0009.
Context: hosted loop 3 (failed-teardown persistence — fixed here). Review
kill/release ordering, dirty-worktree refusal, adopted-vs-owned asymmetry,
and whether the new failed-lifecycle threading is total (any swallowed
backend failure left?).
Report: `docs/reviews/slices/slice3-teardown.md`.
Sentinel: `SLICE3 DONE` or `SLICE3 BLOCKED: <reason>`.
