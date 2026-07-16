# Slice 5 — orchestration review (#57)

Fixed point reviewed: `v1.0.0` / `aa0c0e05b0a26074e5f11328a781b41cb633f669` (current HEAD).

Scope: `src/spawn.rs`, `src/adopt.rs`; spec axis: `docs/spec.md` §4–5, §12, ADR-0004, and ADR-0009. Stage-0 fresh-spawn and healthy-adopt/release evidence lowers ordinary-path risk; this pass focuses on locality/depth and the two pending recovery windows.

## Findings

1. `src/adopt.rs:415`: 🔴 bug — recovery calls `write_worker_protocol` unconditionally, but that helper creates with `create_new` (`src/spawn.rs:877-885`). A crash after the initial protocol write (`src/adopt.rs:415`) and before brief submission/checkpoint persistence (`src/adopt.rs:424-434`) leaves the worker `pending` plus a real immutable protocol; rerunning `adopt` fails with `AlreadyExists` rather than submitting the pointer. This violates `docs/spec.md` §12 “Crash recovery” (`docs/spec.md:544-550`) and ADR-0009 decision 1’s immutable-since-generation contract (`docs/adr/0009-team-adopt.md:20-27`). Failure scenario: the coordinator crashes after protocol generation, then the adopted pane can never be recovered through `adopt`. Disposition: fix-ticket — on recovered pending adoption, reuse an existing protocol (and verify it belongs to that worker) just as spawn resume does at `src/spawn.rs:521-526`; add a test that seeds this exact on-disk state.

2. `src/spawn.rs:841-851`: 🟡 risk — `pane_run`/submission succeeds before `brief_submitted` is transactionally persisted. A process crash in that interval returns to the `pending`/`resources_ready` state; resume detects the live agent (`src/spawn.rs:821-839`) and injects the same brief again (`src/spawn.rs:841-842`). `docs/spec.md` §4 requires checkpointed recovery (`docs/spec.md:144-153`) and says workers receive one launch prompt (`docs/spec.md:125-133`); the current test covers recovery before submission but not this post-submit/pre-checkpoint state (`src/spawn.rs:1803-1855`). Failure scenario: an agent begins the requested work, then receives an indistinguishable second request after recovery. Disposition: fix-ticket — persist an intent/submission checkpoint before injection, or make the submit operation idempotently identifiable, then test the post-submit crash boundary.

## Standards axis

The normal spawn path follows the required ordering: all allocation completes before protocol generation and worker launches (`src/spawn.rs:462-488`), worktree setup is run before workspace creation/launch (`src/spawn.rs:727-769`), and state mutations use the locked fresh-load transaction (`src/spawn.rs:544-560`). The explicit tests cover allocation ordering, missing panes/worktrees, running-worker no-op behavior, and concurrent checkpoint serialization (`src/spawn.rs:1803-2180`). The two findings are the uncovered durable-boundary cases.

## Spec axis

Spawn meets ADR-0004’s cwd/worktree division: it asks Herdr to create worktrees and workspaces rather than placing a `cd` in the prompt (`src/spawn.rs:727-769`, `src/spawn.rs:900-912`; ADR-0004 decision, `docs/adr/0004-worktree-isolation-and-setup-command.md:16-22`). Adopt correctly refuses mesh runs (`src/adopt.rs:452-455`), uses detected-agent plus conservative fallback (`src/adopt.rs:321-330`, `src/adopt.rs:649-668`), and preserves borrowed-pane ownership by never launching an agent CLI (`src/adopt.rs:415-434`; ADR-0009 decision 3–5, `docs/adr/0009-team-adopt.md:33-50`). The first finding breaks the stated adopt recovery sequence; the second leaves spawn recovery at-least-once without a documented duplicate-brief policy.

## Deep-module lens

1. **Deletion test:** `spawn` cannot be deleted or absorbed without exposing run construction, protocol rendering, allocation, launch, recovery, and state transaction details to CLI callers. `adopt` is similarly a necessary policy boundary around ownership and recovery; neither is a shallow convenience wrapper.
2. **Caller leverage:** callers get compact commands (`spawn` and `adopt`) plus `HerdrApi`; they do not directly coordinate panes, worktrees, or `run.toml`. The recovery state machine is still wider than ideal because checkpoint and protocol-existence decisions are duplicated across `src/spawn.rs:491-528` and `src/adopt.rs:361-434`.
3. **Seam placement:** the meaningful Herdr variation is behind `HerdrApi` (`src/spawn.rs:416-429`, `src/adopt.rs:313-320`), and launcher behavior remains table-driven (`src/spawn.rs:820-842`, `src/adopt.rs:330`). That placement is appropriate. Durable recovery policy would be deeper if protocol reuse/creation and checkpoint transitions were a shared operation.
4. **Outcome testing:** fakes exercise public orchestration outcomes rather than shell internals, including resume and adoption behavior (`src/spawn.rs:1803-2180`, `src/adopt.rs:925-1048`). They miss the two explicit crash boundaries above; add interface-level tests that seed persisted state and filesystem artifacts, then rerun the commands.

SLICE5 DONE
