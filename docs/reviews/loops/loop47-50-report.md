# Loop 47 + Loop 50 report — worker loop47-50

Worktree: `/home/caio/Projects/herdr-agent-team-loops/loop47-50` (branch
`loop/47-50-reconcile`, base v1.0.0 `aa0c0e0`). No git commands were run.

## Salvage audit (predecessor died mid-task)

Predecessor's STATUS.log claimed "writing RED test" for both loops, but the
uncommitted diff contained more: the loop-47 fix + test, the loop-50 RED test,
and the hook.rs `reports_present` threading. Everything was treated as
unverified and probed at ground truth:

- **Kept (verified correct):** the `reconcile_at_with_reports` seam +
  `reports_present.contains(name)` gate; the hook's inbox-file scan (path
  convention `<run>/inbox/<worker>.md` confirmed against `run.rs`,
  `god_cli.rs`, `agents_md.rs`); both new reconcile tests.
- **RED evidence reconstructed:** the predecessor applied fix and test
  together, destroying RED proof for loop 47 — the fix was temporarily
  reverted to capture the failure, then restored (verbatim below).
- **Found broken (predecessor missed it):** the loop-47 fix broke the
  existing hook test `blocked_and_done_append_events_and_inject_exact_absolute_pointers`,
  whose fixture asserted team-complete with no report file present — exactly
  the false-completion bug. Fixture updated to write the report, which also
  exercises the hook's report-detection end to end.
- **Missing entirely (completed here):** the loop-50 fix, the clear verb,
  the board wiring, and both doc amendments.

## Loop 47 — false aggregate completion

**Root cause:** `src/reconcile.rs` (v1.0.0, aggregate-completion block at
~222–238): the team-complete gate required only
`worker_status ∈ {idle, done}` for every worker — report presence was never
consulted, so a team of workers that stopped without ever writing
`<run>/inbox/<worker>.md` (stopped-not-done) produced the "Team complete"
notification. Spec §13 is explicit: *report existence = completion truth*.

**RED command** (fix temporarily reverted to capture; test constructs two
workers idle/done with an empty `reports_present` set):

```
$ cargo test all_terminal_workers_without_reports_do_not_notify_team_complete

---- reconcile::tests::all_terminal_workers_without_reports_do_not_notify_team_complete stdout ----

thread 'reconcile::tests::all_terminal_workers_without_reports_do_not_notify_team_complete' (638530) panicked at src/reconcile.rs:746:9:
assertion failed: !reconciled.actions.iter().any(|action|
            matches!(action, ReconciliationAction::Notify { title, .. } if
                title == "Team complete"))

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 184 filtered out
```

**Fix (minimal):**
- `src/reconcile.rs`: new pure entry points `reconcile_with_reports` /
  `reconcile_at_with_reports` carrying `reports_present: &BTreeSet<String>`;
  the team-complete gate becomes
  `all(status ∈ {idle,done}) && reports_present.contains(name)`.
  Existing `reconcile`/`reconcile_at` delegate with an empty set (no callers
  outside hook.rs relied on the notification without reports).
- `src/hook.rs`: computes `reports_present` from `<run>/inbox/<worker>.md`
  file existence per worker and calls `reconcile_at_with_reports`.
- Hook regression fixture `blocked_and_done_append_events_and_inject_exact_absolute_pointers`
  now writes `inbox/builder.md` before the terminal flips — under the new
  contract team-complete fires only with the durable report, and the test
  proves the hook-side detection works.

**GREEN:** `cargo test all_terminal_workers_without_reports_do_not_notify_team_complete`
passes; full suite 187/187 (gate below). The positive case (reports present →
notification fires, at most once) is covered by
`metadata_sequences_and_aggregate_notifications_are_at_most_once` and the
hook test above.

**Files touched:** `src/reconcile.rs`, `src/hook.rs`.

## Loop 50 — attention lifecycle

**Root cause:** `src/reconcile.rs:193` (v1.0.0):
`metadata.attention_pending.remove(&worker_name)` inside the status-change
publish — *every* unrelated status flip consumed a worker's pending
attention, acknowledged or not. Attention had no owned lifecycle.

**RED command** (test raises attention in metadata, applies an unrelated
`working` flip, asserts durable attention survives):

```
$ cargo test status_flip_does_not_consume_pending_attention

---- reconcile::tests::status_flip_does_not_consume_pending_attention stdout ----

thread 'reconcile::tests::status_flip_does_not_consume_pending_attention' (640259) panicked at src/reconcile.rs:768:9:
assertion `left == right` failed
  left: None
 right: Some(true)

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 184 filtered out
```

**Fix (implements the coordinator's decided contract):**
- `src/reconcile.rs`: the publish now *reads* attention
  (`attention_pending.get(..).copied().unwrap_or(false)`) instead of
  removing it — pending attention stays observable on every publish.
- **Verb chosen: `msg <worker> <text> --ack`** (`src/msg.rs`). Rationale:
  exact mirror of the existing raise verb (`msg god <text> --attention`),
  and the board's `[g] ack` row action *already* routed through
  `msg <worker> "acknowledged"` — it just never cleared anything. Smallest
  honest surface: one flag on the surface the ack path already used.
  `clear_attention` removes `attention_pending[<worker>]` AND the
  `aggregate_notifications["attention:<w>"]` gate, so the next raise
  notifies again (a full lifecycle, not a one-shot). Validation mirrors
  `--attention`: `--ack` is rejected for `god`, `all`, comma lists, and in
  combination with `--attention` (new `MsgError::AckTarget`).
- `src/board.rs`: `Acknowledge` action args now include `--ack`.

**Regression tests:**
- `reconcile::status_flip_does_not_consume_pending_attention` (RED→GREEN;
  strengthened to also assert `PublishMetadata { attention: true }` while
  pending).
- `msg::ack_clears_durable_attention_and_rearms_the_raise_notification`
  (raise → observe persisted → ack → both keys gone → re-raise notifies a
  second time).
- `msg::ack_rejects_god_multi_target_and_attention_combinations`.
- `board` action-args test updated for the `--ack` flag.

**Contract recorded:**
- `CONTEXT.md`: new vocabulary entry **Attention lifecycle** (raise /
  observe / clear, worker-owned raise, god-owned clear, flips never consume).
- `docs/spec.md`: new subsection **"Attention lifecycle (raise / observe /
  clear)"** under §11 (the `msg` verb section) + usage line updated with
  `[--ack]`. *Note:* the brief said "§12", but §12 in the current spec is
  `team adopt`; the msg-verb/attention contract lives in §11, so the
  amendment was placed there.

**Files touched:** `src/reconcile.rs`, `src/msg.rs`, `src/board.rs`,
`CONTEXT.md`, `docs/spec.md`.

## Gate

Run from the worktree root after `cargo fmt`:

```
$ cargo fmt --check
FMT-OK (no diff)
$ cargo clippy --all-targets -- -D warnings
cargo clippy: No issues found
$ cargo test
test result: ok. 187 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.31s
```

(Baseline before this work: 183 passed, 2 failed — the loop-50 RED test and
the hook fixture broken by the unfinished loop-47 change.)

LOOP47 GREEN
LOOP50 GREEN
