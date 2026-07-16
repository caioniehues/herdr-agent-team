# Worker brief — issue #52: Stage 3 domain vocabulary pass

Read-write task on the INTEGRATION worktree:
`/home/caio/Projects/herdr-agent-team-loops/integration` (branch
`integrate/program-wave1`). Use ABSOLUTE paths for everything. Edit ONLY:
`CONTEXT.md`, `docs/spec.md`, and (only if you choose the doc-comment route)
doc comments in `src/msg.rs`/`src/hook.rs`. NO other code changes.
**NEVER run `git` or `gh`.**

## Context (read first)

1. `/home/caio/Projects/herdr-agent-team-loops/integration/CONTEXT.md` — note the new entries already added by loops 2 and 4 (Result ready / completion sentinel; Attention lifecycle).
2. `docs/spec.md` §11 (msg verb + the new attention subsection) and the wait/report sections.
3. `docs/reviews/loops/loop48-report.md` + `loop47-50-report.md` (the decided contracts), `loop51-59-report.md` (drain semantics).
4. GitHub issue #60 text is reproduced below (decision input).

## The pass (one coherent amendment, per program Stage 3)

1. **Adopt as product vocabulary** (CONTEXT.md entries + spec anchor):
   **Queued / Submitted / Acknowledged** for message lifecycle, and confirm
   **Result ready / completion sentinel** (already entered by loop 2 — align
   wording, don't duplicate).
2. **`MessageOutcome::Delivered` + the `delivered` audit event**: decide
   rename vs re-document. **Coordinator precedent: re-document, do NOT
   rename** — the durable event stream format must stay compatible with
   existing runs' `events.jsonl`. Document that `delivered` means
   *submitted to the pane's input* (submission semantics), not acknowledged
   by the agent. Record this mapping in CONTEXT.md (Delivered → Submitted)
   and spec §11; if you add doc comments at `MessageOutcome::Delivered` and
   the drain emit sites, keep them one-liners.
3. **Attention lifecycle**: already recorded by loop 4 (raise/observe/clear,
   `msg --ack`). Verify the CONTEXT.md + spec §11 text reflects the merged
   implementation; consolidate if the two loop amendments overlap or drift.
4. **Decide #60** (spawn-return/worker-progress race): run 2 of Stage 0
   showed a god's "immediate" message can land after a fast worker already
   finalized its first report; the worker then updated the report.
   **Coordinator precedent: acceptable semantics, document it** — submission
   ordering is not synchronized with worker progress; a god that needs
   read-before-work sequencing must wait for Acknowledged (a future concern;
   today ack is only human-observable). Record in spec §11 as an explicit
   non-guarantee ("Submission is asynchronous to worker progress") and a
   CONTEXT.md note under the message lifecycle entry.

Keep the whole amendment tight — vocabulary, not redesign. If you find a
genuine contradiction between the merged loop amendments, fix the words to
match the CODE (the code is the decided contract).

## Reporting

- Final report: `/home/caio/Projects/herdr-agent-team-loops/integration/docs/reviews/loops/stage3-52-report.md` — what was adopted, what was documented, the #60 decision text, files touched.
- End with exactly one sentinel line:
  `STAGE3 DONE` or `STAGE3 BLOCKED: <reason>`
