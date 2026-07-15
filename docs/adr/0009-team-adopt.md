# ADR-0009: `team adopt` — existing panes become full workers; star-only; kill releases

Status: accepted (2026-07-15, grilling interview with Caio)

## Context

Day-one dogfooding (issue #1): coordinators keep long-lived worker panes
with accumulated context that predate any run. The status hook ignores
non-team panes, so those workers get babysat with hand-rolled poll loops —
the exact failure class this plugin exists to remove. Adoption must bring
an existing pane under the run-board, the hook, and `msg`.

Tensions resolved here: protocols are immutable and (until now) generated
pre-launch (ADR-0003, spec §4); mesh peer tables are frozen at generation
time; `team kill` closes team workspaces, but adopted panes are borrowed,
not created.

## Decision

1. **Adopted = full worker.** Adoption generates a fresh immutable worker
   protocol (identity, report path, sentinel, self-contained `msg`
   invocation per ADR-0008/ticket 17), pointer-injects it into the running
   pane (mid-turn safe — queues), and records the worker in `run.toml`
   with `adopted = true`. Hook push, `msg`, `team status`, and the inbox
   apply identically to spawned workers. The immutability invariant is
   restated as **immutable since generation** — generation may happen at
   adoption time, not only pre-launch.
2. **Adopt bootstraps.** No active run → adopt creates an ad-hoc star run
   (default team name `adhoc`, god = current pane, cwd = the adopted
   pane's cwd, minimal reconstructed spec in `run.toml`). Otherwise the
   newest active run is the default target; `--run` overrides; multiple
   active runs without `--run` is a hard error listing candidates.
3. **Star-only in v1.** Adopting into a mesh run is a hard error: mesh
   peer tables are immutable and would silently omit the newcomer.
   The eventual mechanism (append-only protocol amendment files) is
   deliberately deferred until a mesh team actually needs mid-run
   adoption (ADR-0007 discipline). Regenerating peer protocols is
   rejected permanently — silent mutation of briefed contracts.
4. **Kill releases, never closes, adopted panes.** Teardown closes only
   plugin-created workspaces. Adopted workers are marked `released` in
   `run.toml` and receive one injected release notice ("team <name>
   ended; report protocol no longer applies") so they stop writing
   reports into a dead run. Ownership principle: tear down what you
   created, release what you borrowed.
5. **Agent kind from detection, conservative fallback.** The pane's
   detected agent label maps into the launcher table. Unknown label →
   adopt anyway under a synthetic conservative policy (`submit_verify =
   true`, `queues_midturn = false`) with a warning naming the exact
   `agents.toml` entry to add; the table is never mutated. Pane with no
   detected agent → refuse (nothing to message, statuses meaningless).
6. **CLI:** `adopt <pane-id> --name <worker> [--role <text>]
   [--brief <path>] [--run <run-dir>] [--team <name>]`. `--brief` injects
   a launch-line-style brief+protocol pointer pair; without it only the
   protocol pointer is injected (the pane already has its task).
   Not in v1: `spawn --adopt` (compose spawn-then-adopt), a mid-run
   un-adopt verb (kill's release covers end-of-run; a real need reopens
   this).

## Consequences

- spec §4's "generated before any agent launch" applies to spawned
  workers; adopted workers generate at adoption (spec §12).
- `run.toml` worker state gains `adopted` and the `released` lifecycle.
- status/kill and hook paths read `adopted` to pick close-vs-release.
- Ad-hoc runs have no team spec file; `run.toml` is the only record.
