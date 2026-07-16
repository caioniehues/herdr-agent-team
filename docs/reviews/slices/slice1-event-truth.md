# Slice 1 (#53) — event → durable truth: `src/hook.rs` + `src/reconcile.rs`

READ-ONLY two-axis review on `integrate/program-wave1`
(v1.0.0 + five loop/fix merges). Sources: `docs/spec.md` §5/§7/§11 (the brief
cites §7–8; the normative event/report/outbox contract lives in §5, §7 and
§11 — reviewed against those), ADR-0002, ADR-0010, CONTEXT.md. Reviewer:
loop49 worker, 2026-07-15.

## Verdict summary

The slice's core design is sound and the wave-1 fixes hold: `reconcile.rs`
is a genuinely deep, pure module (injected clock, injected report set, no
I/O) and `hook.rs` is its imperative shell behind the `HerdrApi` seam. All
seven manifest event subscriptions from spec §5 are present
(`herdr-plugin.toml:57–83`) — the brief's "unsubscribed events" stale-board
class is closed at the subscription level. The four hosted fixes
(reports_present gate, attention observe-not-consume, task metadata,
atomic-claim drain #59) are each present with a regression test through the
public interface. Remaining findings are second-order gaps the fixes still
miss — mostly crash-window and error-isolation truth leaks.

## Findings (ranked)

### F1 — MEDIUM-HIGH: crash between claim and delivery silently orphans a queued message

- **Where:** `src/hook.rs:286–287` (claim rename), `src/hook.rs:350–383`
  (`queued_message_paths` lists only `*.msg`).
- **Violates:** spec §11 "Failed delivery leaves the file in place (retried
  on the next flip) and logs a `delivery_failed` event."
- **Scenario:** the #59 fix renames `<seq>.msg → <seq>.claim` before
  delivering. If the hook process dies in that window (herdr shutdown, kill,
  OOM), the `.claim` file is never re-listed by any future drain — the
  message is lost with no `delivery_failed` event and no retry. The
  in-process failure paths requeue best-effort (`let _ = rename` at
  `hook.rs:306,321`), but a requeue failure or a crash defeats them.
- **Depth note:** the atomic-claim fix is correct for the race it targets
  (double-drain — regression test
  `duplicate_drains_never_record_delivery_failed_after_delivered` is good),
  but it traded the crash-durability property the spec promises.
- **Disposition:** fix-ticket — sweep stale `*.claim` files back to `*.msg`
  at the top of `drain_outbox` (age-gated or unconditionally: rename is
  atomic and the claim winner holds the file for milliseconds), plus a test
  that a pre-existing `.claim` is recovered.

### F2 — MEDIUM: one failed side effect aborts the hook, starving other runs of the event

- **Where:** `src/hook.rs:127` (`inject_pointer(...)?`), `:168`
  (`pane_report_metadata(...)?`), `:173` (`notification_show(...)?`), all
  inside the `for listed_run in run::list_active_runs(...)` loop
  (`hook.rs:67`).
- **Violates:** spec §5 reconciliation-per-active-run; the slice's own
  purpose (durable truth survives external failure — same class as loop 3's
  teardown fix, but on the event path).
- **Scenario:** two active runs; run A's god pane is gone, so
  `inject_pointer` errors → `on_agent_status` returns → run B never
  reconciles the event. Events do not refire, so run B's board is
  permanently stale (e.g. a missed `workspace_closed`). Within run A the
  damage compounds: state was already persisted inside the lock
  (`hook.rs:80–110`) before side effects, so the consumed
  `working→idle/done` transition never re-triggers — the pointer is lost.
  Spec §5 explicitly makes pointer injection at-most-once, so the per-run
  pointer loss is semi-sanctioned; the cross-run event starvation is not.
- **Disposition:** fix-ticket — isolate errors per run (log a
  `delivery_failed`-style event, continue the loop) and per action.

### F3 — MEDIUM: the god's outbox is never drained

- **Where:** `src/reconcile.rs:196–208` — `AgentStatusChanged` with
  `Target::God` early-returns after the blocked sweep; no code path anywhere
  emits `DrainOutbox` for target `god`.
- **Violates:** spec §11 "on **any team member** flipping to `idle` or
  `done`, drains **that member's** outbox."
- **Scenario:** `msg.rs:401–414` enqueues to `outbox/god/` whenever the
  god's launcher declares `queues_midturn = false` and the god pane is
  working/blocked (launcher table is user-editable data, spec §3). Those
  messages then sit forever — the only drain trigger is the hook, and god
  flips never drain. Mitigation: the default god (claude) has
  `queues_midturn = true`, so the enqueue branch is unreachable in the
  shipped configuration.
- **Disposition:** fix-ticket (emit `DrainOutbox { "god" }` on god
  idle/done) or document a hard constraint "god launcher entries must
  declare `queues_midturn = true`" in spec §3 and reject others at spawn.

### F4 — MEDIUM: blocked notification fires once per run lifetime, not once per blocked episode

- **Where:** `src/reconcile.rs:382–395` — `notify_once` gates on
  `aggregate_notifications["blocked:<worker>"]`, which is never removed;
  `blocked_since_ms` IS cleared on unblock (`reconcile.rs:255`) but the
  notification gate isn't.
- **Violates:** spec §11 attention lifecycle sets the precedent that gates
  are cycle-scoped — `msg --ack` clears `attention:<w>` exactly so "a later
  raise notifies again" (`src/msg.rs:333–341`). The blocked gate has no
  analogous clear.
- **Scenario:** worker blocks (notified), god unblocks it, worker blocks
  again hours later past the threshold → silence. The user learned to trust
  the first notification; the second episode is invisible.
- **Disposition:** fix-ticket — remove `blocked:<w>` from
  `aggregate_notifications` on any non-blocked status flip (the same branch
  that clears `blocked_since_ms`), plus a re-block regression test (current
  tests cover only the first episode).

### F5 — LOW-MEDIUM: team-complete gate uses report *existence*, not sentinel *readiness*

- **Where:** `src/hook.rs:79–87` (`reports_present` via `.is_file()`), fed
  into `reconcile.rs:258–261`.
- **Violates:** CONTEXT.md "Result ready — … only when it carries the
  completion sentinel as its final non-empty line; **file existence alone is
  not readiness**."
- **Scenario:** worker opens `inbox/<w>.md`, writes a partial report, goes
  idle without the sentinel → "Team complete" notification fires on a report
  the god's own `wait report:<w>` (which checks the sentinel,
  `src/god_cli.rs:376 report_ready`) would still refuse. Two readiness
  definitions now coexist in the codebase — a divergence seam.
- **Depth note:** the loop-1 fix (reports_present gate) is directionally
  right but shallow: it should reuse `report_ready` rather than re-deciding
  readiness with a weaker predicate.
- **Disposition:** fix-ticket — share `report_ready` (move it to a neutral
  module) and use it for `reports_present`.

### F6 — LOW-MEDIUM: team-complete is unreachable once any worker orphans/ends

- **Where:** `src/reconcile.rs:258–272` — the all-workers check iterates
  `state.workers.keys()` with no lifecycle filter.
- **Scenario:** one worker's pane exits (`Orphaned`, last status
  `working`) or the god removes a misbehaving worker via
  `kill --worker` (`Ended`); the survivors all finish with sentinel-complete
  reports → the completion notification never fires, because the dead
  worker can never reach `idle|done` + report. Possibly intended ("the team
  did not complete"), but the kill-one-worker case reads as a false
  negative.
- **Disposition:** document the intended semantics in spec §11 (and if
  partial completion should notify, exclude terminal-lifecycle workers from
  the check).

### F7 — LOW: `api_schema()` subprocess/socket probe runs while holding the run lock

- **Where:** `src/hook.rs:100–106` — inside the `update_run_with_hook`
  closure, which holds the `.run.toml.lock` flock (`src/run.rs:163–191`).
- **Scenario:** first metadata publish of a run calls out to herdr for the
  schema while every other writer (`msg`, concurrent hooks, kill) blocks on
  the same lock. One-time per run (capabilities are persisted), so impact is
  a single stall — but a hung herdr socket turns it into a run-wide
  deadlock-until-timeout.
- **Disposition:** fix-ticket (small) — probe before entering the closure
  when `metadata_capabilities` is absent; recheck inside.

### F8 — LOW: unknown event name is a hard hook error

- **Where:** `src/hook.rs` `parse_event` fallthrough
  (`_ => Err(HookError::UnexpectedEvent ...)`).
- **Scenario:** manifest (`herdr-plugin.toml`) and `parse_event` must move
  in lockstep; adding a subscription without a parser arm turns every firing
  into a plugin error. Not a spec violation — the CLAUDE.md tolerance rule
  covers unknown *fields* (which are handled correctly, test
  `captured_payload_and_optional_fields_are_preserved_...`), not event
  names, and the strict `dot_form_event_types_are_rejected` test is
  deliberate.
- **Disposition:** document (a comment in `parse_event` pointing at the
  manifest, or a lenient-ignore with a logged note).

### F9 — NIT: per-iteration env/clock reads and a spurious blocked sweep for dead workers

- `src/hook.rs:68–77`: `SystemTime::now()` and the
  `HERDR_AGENT_TEAM_BLOCKED_THRESHOLD_MS` env parse re-run per active run —
  hoist above the loop.
- `src/reconcile.rs`: a worker orphaned *while blocked* keeps its
  `blocked_since_ms` entry (only status flips clear it), so a later sweep
  can emit one "Worker blocked" notification for an already-dead worker
  (right after the "Worker exited" one).
- **Disposition:** wontfix / batch with adjacent work.

## Deep-module assessment (per brief)

- **Deletion test:** `reconcile.rs` passes decisively — deleting it would
  re-scatter all event policy (targeting, transition rules, gating,
  run-ending) into the hook. It is the module that hides the most.
- **Caller leverage / small interface:** `reconcile_at_with_reports(event,
  state, metadata, reports, now, threshold) → Reconciliation` is one
  function absorbing seven event kinds and all policy — strong leverage.
  One flag: 8 of the 13 `ReconciliationAction` variants are no-ops in the
  only consumer (`hook.rs:175–183` bundles them into an empty match arm) —
  they serve as audit markers for tests. That widens the interface beyond
  caller need; consider collapsing informational variants into
  `Reconciliation` fields, or documenting them as test-observable audit
  surface.
- **Seams where behavior genuinely varies:** `HerdrApi` (fake vs socket vs
  CLI backend) and the injected clock/report-set are real variation seams,
  used exactly as seams should be. `drain_outbox` taking a delivery closure
  is a clean third seam that made the #59 race testable in-process.
- **Outcomes tested through the interface:** yes — hook tests drive
  `on_agent_status` with raw JSON and assert on durable files + fake-herdr
  argv; reconcile tests are pure. Gaps: no test for a pre-orphaned `.claim`
  file (F1), a second blocked episode (F4), multi-run error isolation (F2),
  or a god-targeted outbox (F3).

## Wave-1 fixes in context (brief's explicit ask)

| Fix | Present | Regression test | Depth verdict |
|---|---|---|---|
| reports_present gate (loop 1) | `hook.rs:79–87` → `reconcile.rs:261` | `all_terminal_workers_without_reports_do_not_notify_team_complete` | Shallow predicate — see F5 |
| attention observe-not-consume (loop 4) | `reconcile.rs:225–232` + msg.rs raise/clear | `status_flip_does_not_consume_pending_attention` | Deep and local; gate lifecycle exemplary (F4 should copy it) |
| task metadata (loop 5) | `hook.rs:140–170` | `metadata_payload_includes_a_workers_task_when_titles_are_supported` | Good; schema-gated per ADR-0010 preview rule |
| atomic-claim drain (#59) | `hook.rs:281–345` | `duplicate_drains_never_record_delivery_failed_after_delivered` | Correct for the race; introduced crash-window F1 |
| event subscriptions (stale-board class) | `herdr-plugin.toml:57–83`, `parse_event` all 7 | pane_moved/exited/closed/workspace_closed/worktree_removed/agent_detected fixture tests | Closed at subscription level; residual risk is F2 (error starvation), not missing hooks |

## Standards axis (Rust quality)

Clean overall: `thiserror` error taxonomy with context-preserving `Io`
variants; no `unwrap` on external data; the `expect("reconciled worker is in
spec")` at `hook.rs:145` is invariant-safe (adopt pushes the spec entry,
`adopt.rs:632`) but would panic the hook on a hand-corrupted `run.toml` —
acceptable, worth a comment. Locking is correct and documented
(stable lock-file inode across atomic replaces, `run.rs:163`). Tests are
outcome-oriented, not mock-choreography. `cargo fmt`/clippy posture not
re-run here (read-only review; the integration gate owns it).

SLICE1 DONE
