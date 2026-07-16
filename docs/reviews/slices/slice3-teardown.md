# Slice 3 (#55) — teardown review: `src/status_kill.rs`

READ-ONLY review of the integration worktree
(`/home/caio/Projects/herdr-agent-team-loops/integration`, branch
`integrate/program-wave1`). Sources: `docs/spec.md` §6, ADR-0009; loop 3
(failed-teardown persistence) reviewed in context. No code modified.

## Deep-module lens (explicit answers)

- **Deletion test:** mostly passes. One failure: `release_adopted_workers`
  (status_kill.rs:552–595) writes `Released`/`Failed` lifecycles into its
  local in-memory `run`, but `persist_kill_state` (509) re-derives the same
  terminal lifecycles from the *freshly loaded* run via `end_worker_lifecycles`
  (602) + the `failed_workers` set. The in-memory lifecycle writes at 582–588
  are dead state duplicating a decision made once, correctly, at persist time
  — deletable without behavior change (finding S-7).
- **Caller leverage:** good. Both kill paths converge on one persistence
  seam (`persist_kill_state`) that owns the Failed-precedence rule and the
  all-terminal ⇒ run-Ended rule; `status` exposes one `status_run` entry with
  table/JSON as pure renderers over one snapshot builder (260).
- **Genuine seams:** `TeardownBackend` (338) is a real behavior-varying seam
  — `SystemTeardown` binds HerdrApi + a live `git status --porcelain`
  boundary (worktree_is_dirty, 621), `FakeTeardown` drives every teardown
  test. The git boundary itself gets one real-git test
  (`git_status_boundary_detects_untracked_evidence`, 1142).
- **Outcomes through the interface:** yes — teardown tests call
  `kill_*_with_backend` and assert via `load_run` + backend call logs, not
  internals. Coverage of the failure matrix is genuinely good (failed close,
  failed notice, failed+adopted precedence, dirty refusal, idempotent rerun,
  stale-lifecycle repair).

## Findings (ranked)

### S-1 · HIGH · worktree failures escape the failed-lifecycle threading — the loop-3 fix is not total

- **Where:** status_kill.rs:417–427 (`kill_run_with_backend`), 480–488
  (`kill_worker_with_backend`).
- **Violates:** loop 3's own contract (failed teardown must persist as
  durable `Failed` state, not vanish into stderr); spec §6 `--remove-worktrees`.
- **What:** `workspace_close` and release-notice `pane_run` failures are
  threaded into `failed_workers` → persisted `WorkerLifecycle::Failed`
  (394–404, 466–474). But BOTH worktree operations are only
  `log_teardown_note`-and-continue: a `worktree_remove` error (420–421, 483–484)
  and a `worktree_is_dirty` error (424–425, 487) leave no durable trace, add
  nothing to `failed_workers`, and the command exits 0.
- **Failure scenario:** `kill --remove-worktrees` where git is missing from
  PATH (GitSpawn) or `herdr worktree remove` fails → every worktree silently
  survives, run is marked Ended, exit code 0. Under full-auto orchestration
  (no human reads stderr notes) the gate reads "torn down" when disk state
  says otherwise — exactly the swallowed-failure class the brief asks about.
- **Disposition:** fix-ticket — thread worktree failures into
  `failed_workers` (or a distinct persisted marker) and/or a non-zero exit,
  matching the close/notice discipline.

### S-2 · HIGH · dirty-worktree salvage has no retry path — the refusal is a dead end

- **Where:** status_kill.rs:376–380 (`kill_run` ended-run short-circuit),
  446–448 (`kill_worker` ditto), 430–436 + 497–506 (persist-then-error).
- **Violates:** spec §6 salvage rule intent ("refuses if worktree dirty");
  ADR-0009 ownership principle by extension.
- **What:** on a dirty worktree, both paths first persist terminal state
  (run Ended / worker Ended — see test at 989–1013: builder is `Ended` and
  its workspace closed *despite* the `DirtyWorktrees` error), then return the
  error. Any re-run of `kill … --remove-worktrees` after the user salvages
  and cleans the worktree hits the Ended short-circuit (376/446) and returns
  Ok without ever inspecting or removing worktrees. The refusal therefore
  permanently orphans the worktree from the kill surface — salvage cannot be
  completed through the tool that demanded it.
- **Secondary ordering note:** the dirty check runs *after*
  `workspace_close` (465–475 precede 478–489), so the "refused" worker's
  agent is already dead; the salvage rule protects only disk state. That is
  defensible (close ≠ delete) but should be documented as intended.
- **Disposition:** fix-ticket — either let the ended-run path still honor
  `--remove-worktrees` for recorded worktree paths, or document that
  post-salvage removal is manual (`herdr worktree remove`), and say so in
  the DirtyWorktrees error text.

### S-3 · MEDIUM · persisted `Failed` lifecycle has no read surface in `status`

- **Where:** status_kill.rs:260–310 (`build_status_snapshot` /
  `WorkerStatus`), spec §6 status columns.
- **What:** loop 3's whole point is durable evidence of partial teardown,
  but the status table/JSON renders only worker/agent/live-herdr-status/
  report-mtime. A `Failed` (or `Released`/`Orphaned`) worker is
  indistinguishable from a healthy "gone" one; the only reader of the
  persisted evidence is raw `run.toml`. Spec §6's column list predates the
  Failed lifecycle (loop 3 landed after), so this is a spec-and-code gap,
  not a plain deviation.
- **Failure scenario:** god runs `status --json` after a partially failed
  kill; nothing in the output signals which workers need manual cleanup.
- **Disposition:** fix-ticket (add a lifecycle column to snapshot + spec §6
  amendment) — cheap, additive, JSON is versioned by shape anyway.

### S-4 · MEDIUM · release notice bypasses launcher submission policy and outbox discipline

- **Where:** status_kill.rs:465 and 567–577 (raw `backend.pane_run` of
  `release_notice`), vs ADR-0008 / spec §11 (workers receive messages only
  via the submission-verified, readiness-gated msg path; outbox covers
  launchers declaring `queues_midturn = false`).
- **What:** the release notice is a god→worker message delivered as one
  unverified `pane_run`, ignoring the adoptee's launcher policy. ADR-0009 §5
  deliberately adopts unknown agents under a conservative policy
  (`queues_midturn = false`) because mid-turn injection may interrupt them —
  kill then injects mid-turn anyway.
- **Failure scenario:** killing a run while a conservative/unknown adoptee
  is mid-turn interrupts its in-flight work at the exact moment the notice
  says "your report no longer applies" — clobbering the borrowed pane the
  release semantics exist to protect.
- **Disposition:** fix-ticket or document. Honest counterpoint: the run dir
  (and its outbox drain hook) is dying, so queueing has nowhere to deliver
  from; if raw injection is the deliberate trade-off, ADR-0009/spec §6
  should say so explicitly.

### S-5 · LOW · `kill --worker` on an ended run skips the lifecycle repair `kill` performs

- **Where:** status_kill.rs:446–448 vs 376–380.
- **What:** the run-level path repairs stale worker lifecycles on an
  already-ended run (legacy-run test at 1085); the worker-level path just
  returns Ok, leaving a stale `Running` worker in an Ended run untouched.
  Asymmetry, not corruption (the run-level repair can still fix it).
- **Disposition:** fix-ticket (one-line: reuse the repair) or document.

### S-6 · LOW · unknown worker reported as a Usage error

- **Where:** status_kill.rs:449–455.
- **What:** `--worker ghost` yields `StatusKillError::Usage` — a state
  mismatch dressed as CLI misuse. Scripts branching on error class (and the
  usage string appended to it) get misleading output. Compare
  `MsgError::UnknownTarget` / `GodCliError::UnknownWorker`, which model this
  correctly elsewhere in the crate.
- **Disposition:** fix-ticket (new `UnknownWorker` variant), cosmetic.

### S-7 · LOW · duplicated lifecycle decision in `release_adopted_workers`

- **Where:** status_kill.rs:578–588 vs `persist_kill_state`
  (509–549) + `end_worker_lifecycles` (602–618).
- **What:** the in-memory `Released`/`Failed` writes are recomputed from
  disk at persist time; only the returned `failed_workers` set matters. Two
  writers of one decision is the deletion-test failure noted above — a
  future editor changing one site but not the other diverges silently.
- **Disposition:** fix-ticket (delete the in-memory writes, keep the set),
  zero behavior change expected — the existing test suite should prove it.

### S-8 · LOW · side-effectful match guard in `parse_kill_args`

- **Where:** status_kill.rs:157–163 (`value if run_dir.replace(...)` guard
  with the accepting arm being `_ => {}`).
- **What:** the successful positional capture happens inside a *pattern
  guard's* side effect; the visible arm body is empty. Works, tested, but
  reads as "value ignored" and invites a wrong "cleanup". Same idiom at 193.
- **Disposition:** wontfix/document (style only; touch it only when the
  parser next changes).

## Standards axis — summary

Error handling is deliberate (typed variants, source-chained IO, first
non-empty stderr line surfaced from git). Tests are interface-driven with a
strong failure matrix; the one real-git test correctly isolates the process
boundary. The gaps are concentrated in one theme: **failure signals that
stop at stderr** (S-1) and **terminal-state short-circuits that block
follow-up actions** (S-2, S-5).

## Spec axis — summary

Spec §6 conformance is otherwise solid: close-only-recorded-workspaces,
`--json`, per-worker kill retaining the run, dirty refusal listing every
dirty path. ADR-0009 conformance: release-not-close for adopted panes,
exactly-once notice, Failed-precedence over release, adopted worktrees
excluded from removal — all verified by tests. The asymmetries that remain
are S-2 (salvage retry), S-3 (no read surface for Failed), and S-4 (notice
delivery discipline).

SLICE3 DONE
