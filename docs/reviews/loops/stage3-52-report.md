# Stage 3 report — issue #52: domain vocabulary pass

Worktree: `/home/caio/Projects/herdr-agent-team-loops/integration` (branch
`integrate/program-wave1`). No git commands were run. Edits limited to
`CONTEXT.md`, `docs/spec.md`, and two one-line doc comments
(`src/msg.rs`, `src/hook.rs`).

## 1. Adopted vocabulary

**Message lifecycle — Queued / Submitted / Acknowledged** adopted as product
vocabulary:

- New CONTEXT.md entry **Message lifecycle** (placed between **Msg verb** and
  **Outbox**): Queued = outbox entry (`MessageOutcome::Enqueued`); Submitted
  = pane-input submission verified per launcher policy; Acknowledged =
  target demonstrably read/acted — today only human-observable, no durable
  per-message ack. Explicitly notes `msg --ack` acknowledges *attention*, not
  a message.
- New spec §11 subsection **"Message lifecycle — Queued / Submitted /
  Acknowledged (adopted 2026-07-15, Stage 3)"** anchoring the same three
  states, placed before the attention-lifecycle subsection.

**Result ready / completion sentinel** (loop 2's entries): confirmed already
present and consistent in CONTEXT.md and spec §13 — not duplicated. The new
spec lifecycle subsection cross-references them ("report readiness uses the
separate Result ready / completion sentinel vocabulary (§13)") so the two
vocabularies are linked, not merged.

## 2. `MessageOutcome::Delivered` / `delivered` event — re-documented, NOT renamed

Per coordinator precedent: the durable `events.jsonl` format and the enum
keep the word `delivered` for compatibility with existing runs. The mapping
**Delivered → Submitted** (submission semantics: typed into the pane's input
and verified — no claim the agent read or processed it) is recorded in:

- CONTEXT.md (**Message lifecycle** entry).
- spec §11 (lifecycle subsection: "Decided: the code and audit words stay
  `delivered` … read Delivered = Submitted").
- One-line doc comments: `src/msg.rs` at `MessageOutcome::Delivered`
  ("Submitted to the target pane's input (audit word `delivered`, spec §11);
  not agent-acknowledged.") and `src/hook.rs` at the drain's `delivered`
  emit site.

While there, the spec §11 outbox-drain bullets were aligned with the merged
#59 code (words follow the code, per brief): atomic claim via rename
(`<seq>.msg` → `<seq>.claim`), rename-loser ENOENT = already-claimed and
skipped silently (never a failure), failed delivery requeues the claimed
file. The pre-amendment text still described the pre-#59 drain
("deliver, delete the file") with no claim step.

## 3. Attention lifecycle — verified, no consolidation needed

CONTEXT.md's **Attention lifecycle** entry and spec §11's "Attention
lifecycle (raise / observe / clear)" subsection were checked against the
merged implementation (`reconcile.rs` read-not-remove publish, `msg --ack`
clearing `attention_pending` + the raise-notification gate, `--ack`
validation rejecting god/all/comma/`--attention`). The two amendments agree
with each other and with the code — no drift, no overlap to consolidate. The
new lifecycle texts cross-reference it only to disambiguate `--ack`
(attention ack ≠ message ack).

## 4. Decision #60 — accepted semantics, documented as an explicit non-guarantee

Decision text (recorded in spec §11):

> **Non-guarantee (#60, decided 2026-07-15): submission is asynchronous to
> worker progress.** Stage 0 run 2 showed a god's "immediate" message landing
> after a fast worker had already finalized its first report (the worker then
> updated the report). This is accepted semantics: submission ordering is not
> synchronized with what the worker has done or is doing. A god that needs
> read-before-work sequencing must wait for Acknowledged — a future concern,
> since acknowledgment is currently only human-observable.

CONTEXT.md carries the condensed note inside the **Message lifecycle** entry
("Non-guarantee (#60): submission is asynchronous to worker progress …").

## Files touched

- `CONTEXT.md` — new **Message lifecycle** entry.
- `docs/spec.md` — §11: new lifecycle subsection (incl. #60 decision);
  drain bullets aligned with merged #59 claim semantics.
- `src/msg.rs` — one-line doc comment on `MessageOutcome::Delivered`.
- `src/hook.rs` — one-line comment at the `delivered` emit site.
- `docs/reviews/loops/stage3-52-report.md` — this report.

Sanity gate after the comment-only code edits: `cargo fmt --check` clean,
`cargo clippy --all-targets -- -D warnings` clean, `cargo test` 193 passed.

STAGE3 DONE
