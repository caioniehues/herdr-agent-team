# Slice 4 (#56) — messaging: `src/msg.rs` + outbox contract with `src/hook.rs` drain

READ-ONLY two-axis review on `integrate/program-wave1`. Sources:
`docs/spec.md` §11 (incl. the Stage 3 amendments: Message lifecycle
Queued/Submitted/Acknowledged at spec §11:479–503, drain contract §11:526–545,
attention lifecycle §11:507–524), ADR-0008, ADR-0006 discipline as cited by
§11, CONTEXT.md (amended Message-lifecycle entry, lines 38–51). Reviewer:
loop49 worker, 2026-07-15. Companion: slice 1 report
(`slice1-event-truth.md`) reviewed the drain's hook side; cross-referenced
where the contract spans both modules.

## Verdict summary

`msg.rs` is a faithful ADR-0008 implementation: name→pane resolution with
hard ambiguity errors, the exact ADR-0006 submit-verify dance (2 s grace +
one empty `pane run` retry + 30 s verify, `msg.rs:495–518`), sender-never-
blocks enqueue, prevalidated fan-out, and an exemplary attention
raise/observe/clear lifecycle with tests for every clause. **The Stage 3
vocabulary and the code agree** on every load-bearing point (table below).
The findings are: one undecided delivery-policy hole (blocked workers can
deadlock on the very answer that would unblock them), one concrete
divergence bred by the split-module outbox format, and one place where the
amended spec words now promise more than the code delivers.

## Stage 3 vocabulary ↔ code agreement check (explicit brief ask)

| Vocabulary (CONTEXT.md:38–51 / spec §11:479–503) | Code | Agree? |
|---|---|---|
| **Queued** = `MessageOutcome::Enqueued`, in outbox awaiting drain | `msg.rs:117`, `enqueue_message` `msg.rs:521` | ✔ |
| **Submitted** = typed + verified per launcher policy; code/audit word stays `delivered` | `MessageOutcome::Delivered` doc comment `msg.rs:115` says exactly "Submitted to the target pane's input (audit word `delivered`, spec §11); not agent-acknowledged" | ✔ verbatim |
| **Delivered = Submitted, submission semantics only** — no read/processed claim | `deliver_message` `msg.rs:495–518` verifies only `agent wait --status working`; nothing claims more | ✔ |
| **Acknowledged** — no durable per-message ack; `--ack` clears *attention*, not a message | `clear_attention` `msg.rs:333–341` touches only `attention_pending` + the raise gate; doc comment says so | ✔ |
| **Non-guarantee #60** — submission asynchronous to worker progress | no ordering promise anywhere in `msg.rs`; enqueue returns instantly | ✔ |
| Attention raise/observe/clear (spec §11:507–524) | raise `msg.rs:299–329` (gate `attention:<w>`, notify once per cycle), clear `msg.rs:333–341` re-arms the gate; `--ack` form rules enforced `msg.rs:126–130`; tests `ack_clears_durable_attention_and_rearms_the_raise_notification`, `ack_rejects_god_multi_target_and_attention_combinations` | ✔ |
| Drain contract: claim → deliver → verify → remove → `delivered` event (spec §11:530–536) | `hook.rs:281–345` in that exact order | ✔ (but see F3) |

One wording drift the check surfaced: the §11 readiness-gate prose
(spec §11 "if `working`, write the message to the outbox") names only
`working`, while the code enqueues for `blocked` and any future status
string too (`delivery_decision` else-branch, `msg.rs:487–493`). Conservative
and defensible — but undocumented, and it is what arms F1.

## Findings (ranked)

### F1 — MEDIUM-HIGH: a blocked non-queueing worker can never receive the answer that would unblock it

- **Where:** `msg.rs:487–493` (`delivery_decision`: `blocked` →
  `Enqueue` for `queues_midturn = false` launchers) combined with
  `reconcile.rs:233–237` (drain fires only on `idle`/`done` flips — per
  spec §11:528).
- **Violates:** spec §11:521 "Clear — god-owned: `msg <worker> <text>
  --ack` **answers the worker** and clears attention"; ADR-0008's stated
  capability bar ("delivery deferred until the recipient **can process
  it**" — a blocked agent is precisely waiting for input).
- **Scenario:** codex worker (shipped `queues_midturn = false`) hits an
  approval prompt → status `blocked` → raises attention. God replies
  `msg builder "approved, proceed" --ack`. The reply is Queued
  (`blocked` ≠ idle/done/unknown), and `msg_command` then clears attention
  unconditionally after the enqueue succeeds (`msg.rs:133–141` — the
  outcome, Delivered vs Enqueued, is not consulted). Durable state now
  says "attention handled"; the answer sits in `outbox/builder/`; the only
  drain trigger is a flip to idle/done, which a worker waiting at a prompt
  may never produce. Deadlock until a human touches the pane.
- **Depth note:** whether a blocked TUI can safely take `pane run` text is
  a live-verifiable question per ADR-0010 (blocked may mean an approval
  dialog where injected text is dangerous, or a plain input prompt where
  it's exactly what's needed — possibly launcher-specific).
- **Disposition:** fix-ticket + needs-decision — either (a) deliver-on-
  blocked per launcher policy (new launcher field, live-verify per agent),
  (b) drain on the blocked flip too, or minimally (c) don't clear
  attention when the ack's answer was only Queued, so the board keeps
  saying the worker is waiting.

### F2 — MEDIUM: `next_sequence` is blind to `.claim` files — sequence reuse and FIFO inversion around an in-flight drain

- **Where:** `msg.rs:576–584` (`strip_suffix(".msg")` only);
  claim scheme at `hook.rs:286` (`<seq>.claim`).
- **Violates:** spec §11:527 "drains that member's outbox **in sequence
  order**" (sequence order is the FIFO proxy — a later-sent message must
  not sort before an earlier one).
- **Scenario:** outbox holds only `…006.msg`; the hook claims it
  (`…006.claim`). A concurrent `msg` enqueue scans the dir, sees no
  `.msg`, and allocates sequence **1**. Delivery of 6 fails and is
  requeued as `…006.msg`. The next drain now delivers message 1 (sent
  later) before message 6 (sent earlier) — send-order inverted — and
  sequence numbers get reused across generations, so `delivered` audit
  events for distinct messages carry identical paths.
- **Depth note:** this is the concrete cost of the outbox file format
  living in two modules (see design section) — the #59 claim scheme was
  added in `hook.rs` and the allocator in `msg.rs` never learned about it.
- **Disposition:** fix-ticket — include `.claim` names in `next_sequence`
  (accept both suffixes), plus a test enqueueing against a claimed-only
  outbox.

### F3 — MEDIUM: amended spec words now promise a requeue the code only attempts

- **Where:** spec §11:538–539 "Failed delivery **requeues the claimed
  file** (renamed back to `.msg`, retried on the next flip)" vs
  `hook.rs:306,321` — `let _ = std::fs::rename(&claimed, &path)`
  (best-effort, result discarded) and no recovery at all for a crash
  between claim and requeue (slice 1 finding F1: orphaned `.claim` is
  never re-listed).
- **Violates:** the words-code agreement this slice is chartered to check;
  ADR-0010 (a behavior claim in the spec must match live behavior).
- **Scenario:** requeue rename fails (or the hook dies mid-drain) → the
  spec reader believes the message will be "retried on the next flip"; it
  won't — it is stranded as `.claim` with, at best, a `delivery_failed`
  event.
- **Disposition:** fix-ticket — same remedy as slice 1 F1 (stale-`.claim`
  sweep at drain start) makes the spec sentence true; alternatively soften
  the spec sentence. One ticket should own both so words and code converge
  once.

### F4 — LOW-MEDIUM: direct deliveries leave no durable audit trace — only drained ones do

- **Where:** `send_message` `msg.rs:379–412` returns
  `MessageOutcome::Delivered` with no `events.jsonl` append; the
  `delivered` audit event is written only on the drain path
  (`hook.rs:344`).
- **Violates (weakly):** CONTEXT.md:41–43 reads as if Submitted messages
  generally carry the durable audit word ("what the code **and the durable
  audit event** call `delivered`"); in reality the common case (claude
  targets, `queues_midturn = true` → always direct) is unaudited, so
  `events.jsonl` shows a biased history: only messages that happened to
  queue.
- **Scenario:** post-incident audit of "what did the god tell the worker
  and when" reconstructs only the queued minority; direct messages are
  invisible in durable truth.
- **Disposition:** fix-ticket (append a `delivered` event on the direct
  path too — cheap, symmetric) or document the asymmetry in §11.

### F5 — LOW-MEDIUM: the god target gets no conservative launcher fallback

- **Where:** `target_launcher` `msg.rs:466–475` applies
  `conservative_adopted_launcher` only when `target.adopted`; god resolves
  with `adopted: false, agent: None` (`msg.rs:421–427`), so an off-table
  god agent → hard `LauncherError` (via `launcher_entry`), and an
  undetectable one → `MissingAgentKind`.
- **Violates:** ADR-0002 — the god is *any* user-chosen interactive
  session the plugin never spawns; it is definitionally an adopted-style
  pane. ADR-0008's fallback rationale ("adopted pane whose detected agent
  has no launcher-table entry", `launcher.rs:94–96`) applies verbatim.
- **Scenario:** user runs their god session in an agent CLI not in the
  launcher table → every worker `msg god …` (the only briefed reply
  channel, star topology) fails hard; the worker's reply path is dead —
  the exact defect class ADR-0008 exists to prevent.
- **Disposition:** fix-ticket — treat `god` like an adopted target in
  `target_launcher` (conservative fallback), plus a test.

### F6 — LOW: drain path re-delivers file content without re-sanitizing

- **Where:** `deliver_queued_message` `msg.rs:217–232` →
  `deliver_message` directly; sanitization happens only at enqueue time
  (`send_message`, `msg.rs:402`).
- **Scenario:** anything that writes `outbox/<target>/<seq>.msg` out-of-band
  (a worker has filesystem access to the run dir; the format is documented
  in spec §11) gets its bytes typed into a pane verbatim — escape
  sequences included — bypassing `strip_escape_sequences`. Defense in
  depth, not a spec violation.
- **Disposition:** fix-ticket (one-line: sanitize in
  `deliver_queued_message`) or wontfix with a comment stating the trust
  boundary.

### F7 — LOW: explicitly named terminal workers aren't rejected upfront

- **Where:** `send_to_targets` `msg.rs:150–204` — the `all` expansion
  filters terminal lifecycles (`msg.rs:157`), but a direct name or comma
  list resolves an Orphaned/Ended worker happily (pane id is still
  recorded) and fails later with a raw herdr pane error — or worse,
  **enqueues** to a worker that will never flip again (queued into a void,
  cousin of F1).
- **Disposition:** fix-ticket (small): reject terminal-lifecycle targets
  by name with a candidates-style error, consistent with the never-guess
  resolution rule (spec §11:452–454).

### F8 — NIT: minor ergonomics and ordering notes

- `newest_active_run` (`msg.rs:367–372`, ordering from
  `run.rs:228–233`) silently picks the newest of several active runs; a
  god with two live teams can message the wrong one. Deterministic, and
  worker protocols always pass `--run`, so god-side only. Consider a
  multiple-runs warning/error.
- `msg god <text> --attention`: the message is sent before the attention
  raise validates `HERDR_PANE_ID` (`msg.rs:133–138`) — a mis-invoked
  worker gets a delivered message but a failed raise (partial effect).
  Harmless; note only.
- Blocked-status enqueue (the F1 branch) has no direct unit test in
  `msg.rs` — `non_queueing_working_target_…` covers `working` only.

## Outbox contract with `hook.rs` (brief's explicit scope)

The contract holds at the happy path: zero-padded width-20 `<seq>.msg`
(`msg.rs:23,551`) is what `queued_message_paths` parses (`hook.rs:383`);
enqueue's `create_new` retry loop (`msg.rs:552–560`) is concurrency-correct
against the drain's claim renames; ordering is numeric on both sides.
The two defects at the seam are F2 (allocator ignorant of `.claim`) and
F3/slice-1-F1 (crash window in the claim scheme). Root cause is structural:
**the outbox file format is owned by no one** — suffixes, width, claim
extension, and parsing are duplicated knowledge across `msg.rs` and
`hook.rs`. A single `outbox` module owning enqueue + list + claim + requeue
would have made F2 impossible by construction and gives F3's fix one home.

## Deep-module assessment

- **Deletion test:** `msg.rs` passes — it hides target resolution,
  launcher policy, submit verification, sanitization, and queue allocation
  behind two entry points (`msg_command`, crate-internal
  `deliver_queued_message`). Deleting it re-scatters ADR-0008 into every
  generated protocol, which is the pre-ADR defect.
- **Caller leverage / small interface:** strong — one verb absorbs
  fan-out, gating, attention, ack. `delivery_decision` (`msg.rs:487`) is a
  properly tiny pure policy core. The `MessageOutcome` enum is honest
  vocabulary (post-Stage-3 doc comment is exemplary).
- **Seams:** `HerdrApi` again (typed fake calls in tests), plus the
  launcher table as a data seam — behavior genuinely varies by entry;
  `conservative_table()` in tests exercises the variation. The missing
  seam is the outbox module (above).
- **Outcomes through the interface:** yes — tests assert on fake-herdr
  call sequences, real files, and persisted metadata, not internals.
  Coverage gaps: blocked-status gate (F8), claimed-outbox allocation (F2),
  off-table god agent (F5).

## Standards axis

Clean: exhaustive `thiserror` taxonomy with usage-appended argument errors;
`strip_escape_sequences` is a careful hand-rolled CSI/OSC/DCS/C1 consumer
with a good test; sequence exhaustion (`u64::MAX`) handled; no unwraps on
external data; `create_new` + rescan loop is the right lock-free idiom.
Escape-stripping of *error-message interpolations* (`strip_escape_sequences`
on target names in errors) shows unusual care. `attention_writer_preserves_
fresher_hook_worker_state` proves the read-modify-write discipline against
stale boards — good regression depth.

SLICE4 DONE
