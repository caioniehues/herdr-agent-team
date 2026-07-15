# Fix report — issue #65: orphaned `.claim` sweep (branch `fix/65-claim-sweep`)

Worktree: `/home/caio/Projects/herdr-agent-team-loops/fix65`. No git commands
were run. Files touched: `src/hook.rs` only (+ this report).

## Defect (confirmed)

`drain_outbox` (`src/hook.rs`) atomically claims a message by renaming
`<seq>.msg` → `<seq>.claim` before delivering (the #59 fix), but
`queued_message_paths` lists only well-formed `<seq>.msg` entries. A drain
that dies between claim and delivery (crash, kill, power loss) therefore
leaves the message as a `.claim` file that **no future drain can ever see**
— orphaned forever, violating spec §11's requirement that undelivered
messages stay retryable on the next status flip.

## RED

Regression test
`hook::tests::stale_claim_from_a_crashed_drain_is_swept_back_and_delivered`:
seeds `00000000000000000001.claim` (content `orphaned once`, mtime backdated
1 hour via `File::set_times` to model the crashed drain), runs
`drain_outbox`, asserts the message is delivered.

```
$ cargo test stale_claim_from_a_crashed_drain
---- hook::tests::stale_claim_from_a_crashed_drain_is_swept_back_and_delivered stdout ----

thread 'hook::tests::stale_claim_from_a_crashed_drain_is_swept_back_and_delivered' (951279) panicked at src/hook.rs:669:9:
assertion `left == right` failed: a crashed drain's claim must stay retryable (spec §11), not orphaned forever
  left: []
 right: ["orphaned once"]

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 196 filtered out; finished in 0.00s
```

## Fix — age-gated sweep before listing

New `sweep_stale_claims(run_dir, target)`, called at the top of
`drain_outbox` before `queued_message_paths`: every `.claim` in the target's
outbox whose mtime is at least `STALE_CLAIM_THRESHOLD` (300 s) old is renamed
back to `.msg` (atomic, best-effort/ENOENT-tolerant), so the orphan re-enters
the very same drain pass. Missing outbox dir short-circuits, mirroring the
listing.

**Deliberate deviation from the brief's letter, with the evidence:** the
brief's "sweep only files not claimed by this invocation" is not a
sufficient discriminator on its own — the #59 regression test
(`duplicate_drains_never_record_delivery_failed_after_delivered`)
deterministically nests a second drain invocation inside the first drain's
deliver callback; an unconditional sweep in that inner invocation would
requeue the outer drain's **live** claim, double-deliver the message, and
reintroduce the exact false `delivery_failed` that #59 fixed. The staleness
gate implements the brief's intent conservatively across concurrent
invocations: a live claim is held for at most one submission-verification
cycle (`SUBMIT_GRACE_TIMEOUT` 2 s + `SUBMIT_VERIFY_TIMEOUT` 30 s + one empty
`pane run` retry ≈ 1 minute worst case, `src/msg.rs:20–21`), so a 300 s
threshold can only match a dead drain's orphan. Cost: a crash's orphan
younger than 300 s waits for a later status flip — which is spec §11's
normal retry latency anyway ("retried on the next flip").

The #59 test doubles as the guard for the other direction: it passes only
because fresh claims are never swept.

## GREEN

```
$ cargo test stale_claim_from_a_crashed_drain
1 passed (196 filtered out)
```

Full suite — including the #59 duplicate-drain regression and all other
drain-semantics tests — below.

## Gate (verbatim tail)

```
$ cargo test
test result: ok. 197 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.31s

$ cargo clippy --all-targets -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.03s

$ cargo fmt --check
(exit 0, no diff)
```

FIX65 GREEN
