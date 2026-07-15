# Loop #49 report — teardown truth after external failure

Branch `loop/49-teardown-truth`, base v1.0.0 (`aa0c0e0`). Worktree
`/home/caio/Projects/herdr-agent-team-loops/loop49`.

**Provenance note:** a previous worker died mid-task with the RED test AND the
fix already uncommitted in `src/status_kill.rs`. This report verifies both at
ground truth rather than trusting its STATUS.log: RED was reconstructed by
restoring HEAD production code with the new test kept, GREEN and the gate were
re-run on the fixed tree.

## RED — command + verbatim failure (reconstructed against HEAD `aa0c0e0`)

Production code reverted to `git show HEAD:src/status_kill.rs`, new test kept:

```
$ cargo test kill_persists_failed_lifecycle_when_workspace_close_fails

test status_kill::tests::kill_persists_failed_lifecycle_when_workspace_close_fails ... FAILED

---- status_kill::tests::kill_persists_failed_lifecycle_when_workspace_close_fails stdout ----
note: team kill could not close workspace 'workspace-builder' (workspace_not_found); continuing

thread 'status_kill::tests::kill_persists_failed_lifecycle_when_workspace_close_fails' (625663) panicked at src/status_kill.rs:869:9:
assertion `left == right` failed
  left: Ended
 right: Failed

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 183 filtered out
```

The persisted run recorded worker `builder` as `ended` even though the
backend `workspace_close` failed — a clean end-state hiding the incomplete
teardown.

## Root cause

`src/status_kill.rs` at v1.0.0:

- `kill_run_with_backend` (~line 392–394): a failing `backend.workspace_close`
  was only logged via `log_teardown_note` — the failure never reached the
  persistence path.
- `kill_worker_with_backend` (~line 462–470): same swallow for both the
  adopted-pane `pane_run` release notice and the owned `workspace_close`.
- `release_adopted_workers` (~line 545): stamped `Released` unconditionally,
  even when the release-notice `pane_run` had just failed.
- `persist_kill_state` → `end_worker_lifecycles` (~line 566): stamped every
  worker `Ended`/`Released`, so the durable run board always showed a clean
  teardown.

## Fix summary

Thread teardown failures into persistence as a `BTreeSet<String>` of worker
names:

- `release_adopted_workers` returns the set of workers whose release notice
  failed and stamps them `failed` instead of `released`.
- `kill_run_with_backend` extends the set with every non-adopted worker whose
  workspace failed to close.
- `kill_worker_with_backend` collects its own failure (pane or workspace) the
  same way.
- `persist_kill_state(run_dir, worker, end_run, &failed_workers)` stamps
  `WorkerLifecycle::Failed` for every failed worker inside the persisted-state
  hook, and only applies the clean terminal lifecycle to a kill target that
  did NOT fail. `end_worker_lifecycles` already skipped `Failed` workers
  (pre-existing), so a later re-kill cannot launder `failed` back to `ended`.

**Lifecycle vocabulary: no addition needed.** `WorkerLifecycle::Failed`
already exists in `src/types.rs` (spec §4 partial-failure diagnosis); the fix
reuses it for teardown failures — the smallest possible change, consistent
with spec §6. The run itself still ends (`RunLifecycle::Ended`) per ADR-0009
best-effort teardown; honesty lives in the per-worker `failed` states.

## GREEN evidence

Fix restored, full suite:

```
$ cargo test
cargo test: 184 passed (1 suite, 0.31s)
```

Regression tests retained:

- `kill_persists_failed_lifecycle_when_workspace_close_fails` — failed close
  persists `builder: failed` while unaffected `reviewer` still ends cleanly.
- `kill_persists_failed_lifecycles_when_close_or_release_notice_fails`
  (renamed from `kill_with_a_closed_adopted_pane_releases_every_worker_and_ends_the_run`)
  — both a failed workspace close and a failed adopted-pane release notice
  persist `failed`; kill still completes without aborting.

## Gate output tail

```
$ cargo fmt --check
FMT OK
$ cargo clippy --all-targets -- -D warnings
cargo clippy: No issues found
$ cargo test
cargo test: 184 passed (1 suite, 0.31s)
```

## Files touched

- `src/status_kill.rs` (+70 −13) — fix + 1 new test + 1 hardened existing test.
- `docs/reviews/loops/loop49-report.md` (this report).
- `STATUS.log` (status pings; not for commit).

LOOP49 GREEN
