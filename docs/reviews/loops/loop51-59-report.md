# Worker report — loop #51 + defect #59 (branch `loop/51-59-hook-outbox`)

Worker: loop51-59. Salvage note: a previous worker died mid-task leaving an
uncommitted diff in `src/hook.rs` (the #51 fix + its regression test). I
verified the salvage at ground truth — the partial `HerdrApi` test double
compiles (the trait has default method bodies), and the fix's precedence
(`run-state task, else spec task`) matches the established precedent at
`src/board.rs:273`. I reverted the fix hunk to capture RED myself, then
restored it. #59 was untouched by the dead worker; done from scratch.

## 1. Loop 51 — dropped task metadata

**Root cause:** `src/hook.rs:136` (pre-fix) — the `PublishMetadata` arm built
`MetadataFacts` with a hardcoded `task: None`, so a worker's task never
reached `pane report-metadata` even when the schema was title-capable
(`map_facts` maps `facts.task` → `title`, `src/metadata.rs:78–81`).

**RED** — regression test `hook::tests::metadata_payload_includes_a_workers_task_when_titles_are_supported`
(fix hunk reverted to `task: None`, test kept):

```
$ cargo test metadata_payload_includes_a_workers_task
---- hook::tests::metadata_payload_includes_a_workers_task_when_titles_are_supported stdout ----

thread 'hook::tests::metadata_payload_includes_a_workers_task_when_titles_are_supported' (629901) panicked at src/hook.rs:590:9:
assertion `left == right` failed
  left: None
 right: Some("ship hook seam")

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 183 filtered out; finished in 0.00s
```

**Fix (minimal):** populate `task` from the run-state worker, falling back to
the spec worker — same precedence the run board already uses
(`board.rs:273`):

```rust
task: run
    .state
    .workers
    .get(&worker_name)
    .and_then(|worker| worker.task.as_deref())
    .or(worker.task.as_deref()),
```

**GREEN:**

```
$ cargo test metadata_payload_includes_a_workers_task
1 passed, 183 filtered out
```

**Files touched:** `src/hook.rs` (fix at the `PublishMetadata` arm +
regression test). Test double overrides only `api_schema` /
`pane_report_metadata` and captures the published `MetadataUpdate`, asserting
`title == Some("ship hook seam")`.

## 2. Defect 59 — outbox drain race

**Confirmed mechanism** (`src/hook.rs::drain_outbox`, pre-fix lines ~260–300):
there was **no claim step between listing and consuming** a queued message.
Two near-simultaneous status events each fire the hook; both invocations run
`queued_message_paths` and list the *same* `.msg` entry. Both then
`read_to_string` → both **deliver (duplicate delivery)** → the winner
`remove_file`s and appends `delivered`; the loser's `remove_file` hits ENOENT
and appends `delivery_failed` for the already-delivered path. (If the loser
lists before but reads after the winner's removal, its `read_to_string`
ENOENTs instead — same false failure, one emit site earlier.) This matches
the live evidence exactly: stage0 `run1/events-final.jsonl` lines 16–17 and
`run2/events-final.jsonl` lines 17–18 record `delivered` then
`delivery_failed` (`No such file or directory (os error 2)`) for the SAME
`00000000000000000001.msg` path. The hypothesis in the brief is confirmed —
with the addition that the loser also **duplicate-delivers** the message
text before recording the false failure.

**RED** — regression test `hook::tests::duplicate_drains_never_record_delivery_failed_after_delivered`,
a deterministic nested drain interleaving a duplicate drain inside the first
drain's deliver callback:

```
$ cargo test duplicate_drains_never
---- hook::tests::duplicate_drains_never_record_delivery_failed_after_delivered stdout ----

thread 'hook::tests::duplicate_drains_never_record_delivery_failed_after_delivered' (635706) panicked at src/hook.rs:621:9:
assertion `left == right` failed: the message must be delivered exactly once
  left: ["queued once", "queued once"]
 right: ["queued once"]

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 184 filtered out; finished in 0.00s
```

The RED run demonstrates both symptoms: the duplicate delivery (asserted
first) and, behind it, the loser's false `delivery_failed` event.

**Fix (minimal, atomic claim via rename — no locks):** before consuming,
`rename` `NNN.msg` → `NNN.claim`. Rename is atomic on POSIX, so exactly one
drain wins each message:

- Rename **ENOENT → `continue` silently**: already claimed by a concurrent
  drain — already-delivered, not a failure.
- `.claim` files are invisible to `queued_message_paths` (it only accepts the
  `.msg` suffix), so a claimed message can never be double-listed.
- Read failure or delivery failure after a claim → best-effort rename back to
  `.msg` (requeue for a later drain, preserving existing retry semantics),
  then `delivery_failed` + stop, as before.
- `remove_file(&claimed)` ENOENT after a successful claim → tolerated as
  already-consumed (never a failure); other IO errors still surface as
  `delivery_failed`.
- Events still record the original `.msg` path, keeping the durable event
  format unchanged.

**GREEN:**

```
$ cargo test duplicate_drains_never
1 passed, 184 filtered out
```

Full suite (all pre-existing drain semantics preserved — sequence order,
failed-delivery requeue+stop, done-drain-before-pointer ordering): 185
passed.

**Files touched:** `src/hook.rs` (`drain_outbox` claim logic + regression
test).

## 3. Gate

```
$ cargo fmt --check
(exit 0, no diff)

$ cargo clippy --all-targets -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.03s

$ cargo test
test socket::tests::silent_partial_peer_is_bounded ... ok

test result: ok. 185 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.31s
```

LOOP51 GREEN
LOOP59 GREEN
