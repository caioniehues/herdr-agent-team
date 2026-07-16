# Implementation-review program

Date: 2026-07-15
Wayfinder ticket: [Design the implementation-review program](https://github.com/caioniehues/herdr-agent-team/issues/45)
Baseline: [feature reality audit](feature-reality-audit.md), critically re-verified
(all five Partial verdicts reproduced at cited source lines; three corrections
below folded in).

## Corrections to the audit baseline

The audit survives adversarial re-verification with three amendments:

- **C1 (attention lifecycle).** `attention_pending` is consumed by *every*
  status-change publish (`src/reconcile.rs:193`), acknowledged or not. Attention
  is therefore not "durable until acknowledged"; it has no owned lifecycle at
  all. Diagnosis loop 4 is re-scoped from "board `g` does not clear attention"
  to "attention has no owned raise/observe/clear contract."
- **C2 (Absent taxonomy).** Conscious deferrals (task dependencies, provider
  restart, mesh adoption) are non-scope, not gaps, and receive **no review
  effort**. "Target acknowledgment" is map-created forward vocabulary, not
  historical intent; it enters the program as a domain-vocabulary decision
  (Stage 3), not as a missing feature.
- **C3 (Unverified block).** 12/37 Unverified verdicts stem from the proof
  standard's twice-run rule, not from weak implementation evidence. The program
  front-loads one scripted live E2E to collapse this block before any code
  review, because the resulting verdicts re-prioritize the slices.

## Fixed points and authoritative specs

- **Reviewed revision:** `v1.0.0` = `aa0c0e05b0a26074e5f11328a781b41cb633f669`.
  Any fix produced by a diagnosis loop is reviewed against this fixed point
  with `/code-review`.
- **Intent baseline:** `git show 2416889:docs/spec.md` + ADRs 0001–0007, amended
  only by ADR-0008–0011 and explicitly accepted later work (per the
  [evidence ledger](feature-reality-evidence-ledger.md)).
- **Spec sources per slice:** the spec sections and ADRs named in each slice
  below. `CONTEXT.md` is the vocabulary authority; the
  [capability map](feature-capability-map.md) vocabulary
  (Result ready, completion sentinel, Queued/Submitted/Acknowledged) becomes
  authoritative only after the Stage 3 domain decision adopts it.
- **Proof authority:** the [proof standard](feature-proof-standard.md),
  unchanged. Deterministic loops need one retained run; stateful external
  claims need two consecutive fresh runs.

## Stage 0 — Evidence unblocking (runs first)

One scripted, retained, twice-run live E2E against the installed stack
(Herdr 0.7.3, Claude Code 2.1.210, Codex CLI 0.144.4):

spawn (file spec, worktrees, both providers) → msg (immediate + queued drain) →
worker report + pointer push → `team wait` → adopt/release probe → kill with a
deliberately dirty worktree.

Exit criterion: every currently-Unverified row moves to Working or Partial with
retained artifacts, or records the exact external blocker. The verdict changes
re-rank Stage 2 slice priorities before code review begins.

## Stage 1 — Diagnosis loops (RED evidence before any fix)

Each loop enters `/diagnosing-bugs` only once its deterministic, agent-runnable
RED command exists. Priority order by user-facing harm to coordination truth:

| # | Loop | RED shape | Seam |
|---|---|---|---|
| 1 | False aggregate completion | pure reconcile test: all workers `idle\|done`, no reports → must not notify team-complete | `src/reconcile.rs:222-238` |
| 2 | Premature result readiness | create report path with unfinished content → `wait report:<w>` must not reach | `src/god_cli.rs:261-263,354` |
| 3 | Teardown truth after external failure | failing backend close/notice → persisted lifecycle must expose incomplete teardown | `src/status_kill.rs:392-427` |
| 4 | Attention lifecycle (re-scoped per C1) | worker raises attention, unrelated status flip occurs → durable attention must survive until an owned clear action | `src/reconcile.rs:193`, `src/board.rs:162-168` |
| 5 | Dropped task metadata | `worker.task = Some(..)` + title-capable schema → task must reach `pane report-metadata` | `src/hook.rs:136` |

Loop 4 requires a small design decision (who clears attention, through which
verb) before its RED test is meaningful — take that decision inside the loop,
recorded as a spec §12/CONTEXT.md amendment.

## Stage 2 — Code-review slices (deep-module lens, priority order)

Organizing principle (audit finding, confirmed): review hardest where external
effects cross into durable domain truth. Each slice runs `/code-review` with
parallel Standards + Spec reviews against the named spec source.

| Priority | Slice | Modules | Spec source | Why this rank |
|---|---|---|---|---|
| 1 | Event → durable truth | `hook.rs`, `reconcile.rs` | spec §7–8, ADR-0002/0010 | Hosts loops 1, 4, 5; the truth-crossing seam |
| 2 | Durable truth read/wait | `god_cli.rs`, `run.rs` | spec §10, god-tools sections | Hosts loop 2; wait semantics gate the coordinator |
| 3 | Teardown | `status_kill.rs` | spec §6, ADR-0009 | Hosts loop 3; honest end-state |
| 4 | Messaging & outbox | `msg.rs` | spec §11, ADR-0008 | `Delivered` vocabulary; queue ordering already strong |
| 5 | Orchestration | `spawn.rs`, `adopt.rs` | spec §4–5, §12, ADR-0004/0009 | Strongest existing test coverage; review for depth/locality, not correctness panic |
| 6 | Socket | `socket.rs`, `socket_backend.rs` | ADR-0011 | Experimental, opt-in, gated; last unless Stage 0 flags it |

Per-slice questions from `/codebase-design`: does the module pass the deletion
test; do callers get leverage through a small interface; are seams placed where
behavior genuinely varies (HerdrApi, launcher table); are outcomes tested
through the interface rather than past it.

## Stage 3 — Domain-model consistency

One `/domain-modeling` pass recording, in `CONTEXT.md` + a spec amendment:

- adopt **Queued / Submitted / Acknowledged** and **Result ready / completion
  sentinel** as accepted product vocabulary (explicitly new scope, per C2);
- rename or re-document `MessageOutcome::Delivered` and the `delivered` audit
  event to submission semantics;
- record the attention-lifecycle contract decided in loop 4.

Runs after Stage 1 loops 2 and 4, because those decisions feed the vocabulary.

## Ordering and exit criteria

Stage 0 → Stage 1 loops 1–3 → Stage 2 slices 1–3 (interleaved with loops 4–5
and Stage 3) → Stage 2 slices 4–6.

The program is complete when: the Unverified block is resolved with retained
evidence; all five loops reached GREEN through reviewed fixes (or a recorded
decision not to fix); slices 1–4 have recorded review verdicts; and the
vocabulary decision is committed. Fixes ship under the release gate
(fmt/clippy/tests, version bump) per repository rules.

## Stage 0 outcome and re-rank (2026-07-15, #46 closed)

Evidence: `docs/reviews/evidence/stage0/` (script + run1 + run2 + REPORT.md).
All 12 Unverified rows resolved: 6 Working, 5 Partial, 1 Blocked with exact
blockers (mesh messaging — `team adopt` rejects mesh runs; crash-window
pending-adopt recovery — not injectable by this flow). New defects filed:
#59 (outbox drain race: `delivered` then false `delivery_failed`, twice
reproduced), #60 (spawn-return/worker-progress race, feeds Stage 3).

Stage 2 slice order after re-rank: **1 event → durable truth (unchanged) →
2 messaging & outbox (was 4; promoted on #59) → 3 durable truth read/wait
(socket-acceleration claim narrowed: wait proven, backend acceleration
unproven without trace) → 4 teardown → 5 orchestration (fresh-spawn and
healthy adopt/release risk lowered; pending-spawn/pending-adopt recovery
remain focused unproven sub-slices) → 6 socket.**

## Program outcome (2026-07-15, executed)

All stages complete on `integrate/program-wave1` (v1.0.0 + 7 reviewed merges,
197 tests, fmt/clippy/tests clean; NOT pushed — merge to main = release,
gated on Caio):

- **Stage 1:** loops 1–5 GREEN via RED-first fixes (#47–#51), plus #59.
- **Stage 2:** all six slices reviewed with recorded verdicts
  (`docs/reviews/slices/` in the integration worktree + main checkout);
  slice findings → #61–#65 fixed this wave, #66–#83 filed as the new
  frontier (HIGH: #74 worktree-failure truthfulness, #75 dirty-refusal dead
  end; needs-decision: #77 release-notice policy, #79 blocked-worker
  message deadlock).
- **Stage 3:** vocabulary committed (#52): Queued/Submitted/Acknowledged,
  Delivered = submission semantics (no rename), attention lifecycle
  (msg --ack), #60 non-guarantee.

## Out of scope

- Conscious deferrals: task dependencies, provider restart, mesh adoption,
  run history, additional launchers, limux backend (C2).
- Any feature work not required to turn a RED loop GREEN.
