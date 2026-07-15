# Fix #61 + Fix #62 report — worker fix61-62

Worktree: `/home/caio/Projects/herdr-agent-team-loops/fix61b` (branch
`fix/61-62-orchestration`, base v1.0.0 `aa0c0e0`). No git commands were run.
All tests are pure-logic through `FakeHerdr` — no live panes.

## Fix #61 — adopt recovery cannot re-run

**Root cause:** `src/adopt.rs` (v1.0.0 ~line 415) called
`write_worker_protocol` unconditionally on every submission path, including
`AdoptDisposition::Recovered`. The helper (`src/spawn.rs`,
`write_worker_protocol`) opens with `create_new(true)` to enforce the
protocol-immutability invariant — so a crash after protocol write but before
brief submission left a pending adopted worker whose every recovery re-run
died with `AlreadyExists`, forever. Spawn resume already had the correct
pattern (`if !protocol.exists()`).

**RED command** (test seeds exactly the crash state: pending adopted worker
persisted in `run.toml`, `protocols/newcomer.md` present on disk):

```
$ cargo test pending_adoptee_rerun_recovers_when_protocol_survived_the_crash

---- adopt::tests::pending_adoptee_rerun_recovers_when_protocol_survived_the_crash stdout ----

thread 'adopt::tests::pending_adoptee_rerun_recovers_when_protocol_survived_the_crash' (875516) panicked at src/adopt.rs:1008:10:
recovery must reuse the surviving immutable protocol: Spawn(Io { action: "create immutable generated protocol", path: "/tmp/herdr-adopt-tests-875515-1784144428489585869-0/runs/active-team-1784144428489/protocols/newcomer.md", source: Os { code: 17, kind: AlreadyExists, message: "File exists" } })

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 184 filtered out
```

**Fix (minimal, mirrors spawn resume):** in the adopt submission path, when
`disposition == Recovered` and the protocol file already exists, reuse it
instead of rewriting; a fresh `Adopted` disposition still writes
unconditionally, preserving the immutability invariant. Per the brief's
"verify it belongs to that worker": new `verify_recovered_protocol` reads the
surviving file and requires the generated identity marker
(``- Worker: `<name>` `` — the same marker existing tests assert on; the path
is additionally name-keyed as `protocols/<worker>.md`). A file naming a
different worker fails with the new `AdoptError::ForeignProtocol` before any
prompt is injected; unreadable files fail with `AdoptError::ProtocolRead`.

**GREEN:**
- `pending_adoptee_rerun_recovers_when_protocol_survived_the_crash` — 1 passed
  (recovery succeeds, disposition `Recovered`, exactly one prompt submission,
  surviving protocol byte-identical afterwards).
- `recovery_rejects_a_surviving_protocol_that_names_another_worker` — 1 passed
  (ForeignProtocol error, zero pane_run calls).
- Existing adopt recovery tests (`pending_adoptee_rerun_recovers_without_launching_agent_cli`,
  `recovered_brief_checkpoint_skips_duplicate_submission`,
  `healthy_adoptee_rerun_is_a_no_op`) stay green — full suite below.

**Files touched:** `src/adopt.rs` (fix + error variants + 2 regression tests).

## Fix #62 — post-submit/pre-checkpoint duplicate brief

**Root cause:** `src/spawn.rs` `launch_worker` (~841–851 in v1.0.0):
`submit_worker_prompt` succeeds, and only afterwards does one atomic state
update persist `BriefSubmitted` + `Running`. A crash inside that window
leaves `(Pending, ResourcesReady)` on disk; `resume_resolved` selects the
worker and unconditionally re-injects the same launch prompt — violating
spec §4 (one launch prompt per worker; checkpointed recovery).

**RED command** (test seeds the post-submit crash boundary: state rolled back
to `(Pending, ResourcesReady)` while the pane's agent reports `working` —
i.e. the brief landed and the agent is mid-turn on it):

```
$ cargo test resume_does_not_reinject_a_brief_the_working_agent_already_received

---- spawn::tests::resume_does_not_reinject_a_brief_the_working_agent_already_received stdout ----

thread 'spawn::tests::resume_does_not_reinject_a_brief_the_working_agent_already_received' (875532) panicked at src/spawn.rs:1901:9:
a brief the working agent already received must not be re-injected

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 184 filtered out
```

**Fix (smallest honest mechanism — the "idempotently identifiable" option):**
no new checkpoint variant, no serde change. `launch_worker` already does one
`pane_get` on resume (`agent_already_running`); the fix keeps that single
call's full `PaneInfo` and adds one gate: if the pending worker's agent is
already `working` at resume time, the brief was submitted before the crash —
only a submitted launch prompt starts a worker's turn, and ADR-0006 is
precisely the project's convention that `working` is submission evidence
(`agent wait --status working` as submission check). In that case resume
skips prompt injection and just completes the checkpoint
(`BriefSubmitted` + `Running`). Recovery-before-submit is untouched: an
`idle`/agent-absent pane re-injects exactly as before (FakeHerdr's default
`pane_get` reports `idle`, so the pre-existing resume tests at
`src/spawn.rs:1803+` exercise that path unchanged and stay green).

**Honest residual (documented, not hidden):** if the agent *finished* the
brief turn between crash and resume (idle again), resume still re-injects —
`idle` cannot distinguish "never briefed" from "briefed and already done"
without pane-content forensics, and a silently briefless worker is the worse
failure. The fix closes the window where the submission is live and provable.

**GREEN:** `resume_does_not_reinject_a_brief_the_working_agent_already_received`
— 1 passed (no `Read your brief` injection; lifecycle `Running`; checkpoint
`BriefSubmitted`). Existing resume tests
(`resume_launches_pending_worker_and_leaves_running_worker_untouched`,
`resume_launches_cli_when_live_pending_pane_has_no_agent`,
`resume_skips_pending_adopted_worker_without_brief_preflight`,
`resume_preflight_ignores_running_workers_missing_brief`) all green.

**Files touched:** `src/spawn.rs` (fix + 1 regression test).

## Gate

Run from `/home/caio/Projects/herdr-agent-team-loops/fix61b` after `cargo fmt`:

```
$ cargo fmt --check
FMT-OK (no diff)
$ cargo clippy --all-targets -- -D warnings
    Checking herdr-agent-team v1.0.0 (/home/caio/Projects/herdr-agent-team-loops/fix61b)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 3.01s
$ cargo test
test result: ok. 186 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.31s
```

FIX61 GREEN
FIX62 GREEN
