# Slice 2 (#54) — durable truth read/wait

Reviewed: `src/god_cli.rs`, `src/run.rs`  
Worktree: `/home/caio/Projects/herdr-agent-team-loops/integration` (branch `integrate/program-wave1`)  
Spec sources: `docs/spec.md` §13 (God CLI ergonomics, includes wait/inbox verbs)  
Date: 2026-07-15  
Reviewer: fix63-64 worker

---

## Deep-module assessment

**`run.rs`** is a genuine deep module. A small public surface (`create_run`,
`load_run`, `update_run_with_hook`, `append_event`, `list_active_runs`,
`match_pane`) hides lock acquisition, atomic TOML replace, inbox directory
creation, timestamp-based run naming, and backward-compatible deserialization.
All writes go through `write_run_contents` which does create → write → flush →
rename-over, making every update atomic at the filesystem level. The lock
(`fs4` advisory, `.run.toml.lock`) serializes cooperative writers.

**`god_cli.rs`** is shallower: the public surface is five CLI entry points and
two exported types (`WaitVerdict`, `InboxRow`). The private `collect_snapshot`
→ `row` path is well-encapsulated, and `GodCollector` provides the seam for
socket backend replacement. The wait logic (`wait_with`, `condition_met`,
`dead_worker`) is tested through the trait seam as required.

**Loop 2 fix (sentinel readiness):** fully integrated. `InboxRow` now carries
both `report_present` (file exists) and `report_ready` (last non-empty line ==
`HERDR_TEAM_WORKER_COMPLETE`). All three wait conditions that poll for reports
(`AnyReport`, `Report(w)`, `AllReports`) use `report_ready` exclusively.
`dead_worker` also uses `report_ready`. `inbox_detects_stopped_not_done_and_read_marks_persist`
tests the separation end-to-end through the filesystem.

---

## Standards findings

### S1 — `inbox` text format labels `REPORT` for presence, wait fires on readiness (Low)

**File:** `god_cli.rs:145–150`  
**Failure scenario:** Worker writes a partial report (no sentinel yet). `inbox`
shows `REPORT` in the text column. The operator sees `REPORT` and concludes the
worker is done, but `wait --until any-report` continues polling. Operator
confusion, no correctness issue.

```rust
// god_cli.rs:145–150
let state = if row.stopped_not_done {
    "STOPPED-NOT-DONE"
} else if row.report_present {    // ← presence, not readiness
    "REPORT"
} else {
    "-"
};
```

The JSON output correctly includes `report_ready` alongside `report_present`
(`InboxRow` derives `Serialize`). Only the text path uses the weaker signal.

**Spec reference:** §13 says "`inbox` emits one worker row with report
presence/mtime" — so `report_present` is the documented field. But §13 also
says "wait only for Result ready reports", making the operational gap real.

**Disposition:** document / low-priority UX fix-ticket. Consider showing
`REPORT-READY` vs `REPORT (pending)` or adding `report_ready` to text output.

---

### S2 — `attention` row flag conflates `attention_pending` with blocked status; the blocked→attention path is untested (Low-Medium)

**File:** `god_cli.rs:368–369`  
**Root cause:**

```rust
attention: hook.attention_pending.get(worker).copied().unwrap_or(false)
    || status == Some("blocked"),         // ← also fires for any blocked worker
```

A worker in `blocked` status (from `hook.worker_status`) sets `row.attention
= true` even when no `--attention` flag was ever raised.

`condition_met(Until::Attention)` uses:
```rust
Until::Attention => s.rows.iter().any(|r| r.attention),
```

So `wait --until attention` fires whenever any worker is blocked — the same
condition as `wait --until blocked`:
```rust
Until::Blocked => s.statuses.iter().any(|(_, status)| status == "blocked"),
```

Both conditions overlap on blocked workers. `--until attention` is a strict
superset: explicit-attention OR blocked.

**Spec reference:** §13 — "`blocked` and `attention` come from durable hook
metadata" — the spec lists them as two distinct wait conditions without
specifying the overlap. §11 (Attention lifecycle) says attention is
"explicitly raised" (worker sends `--attention`), distinct from blocked status.
The conflation may be intentional (blocked is operationally attention-worthy)
but it's undocumented.

**Testing gap:** The existing `snap` helper (god_cli.rs:559–579) creates the
attention row with `status == "attention"`:
```rust
attention: status == "attention",
```
This only exercises the `attention_pending` path inside `row()` if `status ==
"attention"` maps to the attention flag. However, the REAL `row()` function
checks `attention_pending` (from `hook.attention_pending`) OR `status ==
Some("blocked")`. The test helper is hand-wiring the `attention` field
directly (in `snap()`), bypassing `row()` entirely. No test exercises the
production path where `status == "blocked"` sets `attention = true` and
`wait --until attention` fires.

**Disposition:** fix-ticket (document the overlap AND test both the
blocked→attention path and the attention-only path through production `row()`).

---

### S3 — `AllTerminal` vacuously reaches on empty `worker_lifecycles` (Low)

**File:** `god_cli.rs:276–278`

```rust
Until::AllTerminal => s.worker_lifecycles.iter().all(|(_, state)| terminal(*state)),
```

`iter().all()` on an empty iterator returns `true`. If `wait --until
all-terminal` is called immediately after `team spawn` (workers not yet
persisted to `state.workers`), or on a newly created run before adoption, it
returns `Reached` with exit 0 instantly.

Compare with `AllReports` which has an explicit guard:
```rust
Until::AllReports => !s.rows.is_empty() && s.rows.iter().all(|r| r.report_ready),
```

**Spec reference:** §13 — "`all-terminal` is literal: failed and orphaned
workers count as terminal and the condition exits 0." The spec says "literal"
but the empty-team case is not mentioned. Vacuous truth here seems
unintentional given the `AllReports` guard.

**Disposition:** fix-ticket — add `!s.worker_lifecycles.is_empty() &&` guard,
add a test.

---

### S4 — `report_command` reads mtime outside the update lock (Very Low)

**File:** `god_cli.rs:173–183`

```rust
let mtime = report_mtime_ms(&path)?.unwrap_or(0);   // outside lock
update_run_with_hook::<_, GodCliError>(&run_dir, |_, hook| {
    hook.report_read_mtime_ms.insert(worker.clone(), mtime);   // inside lock
    Ok(())
})?;
```

If the worker atomically replaces the report file in the window between the
mtime read and `update_run_with_hook`, the stored mark records the old mtime.
On the next `report` invocation the file appears unread.

**Practical impact:** The report is written once by the worker (workers write
then go idle), so the race window is closed before the god operator typically
reads. Benign in practice.

**Disposition:** document (comment in code); wontfix unless a restart-recovery
scenario produces double-writes.

---

## Spec findings

### Spec-1 — `wait --until attention` and `wait --until blocked` overlap undocumented (Low)

**File:** `god_cli.rs:368–369`, spec §13, §11  
(This duplicates S2 above from a spec-conformance angle.)

Spec §13 lists the conditions as distinct. The implementation makes `attention`
a strict superset of `blocked`. The exit code and verdict name differ only in
naming (`Reached` for both, via different conditions). If the intent is that a
blocked worker should satisfy `--until attention`, the spec should say so.

**Disposition:** document in spec §13 — "attention also fires for blocked
workers (blocked is treated as an implicit attention-worthy state)."

---

## Test coverage gaps

### T1 — blocked→attention path untested (Low)

**File:** `god_cli.rs:568–579, 619–621`

The `snap` helper sets `row.attention` directly; it doesn't go through the
production `row()` constructor that reads from `hook.worker_status`. No test
verifies that a blocked worker (`status == "blocked"`) causes
`wait --until attention` to reach. Test should use real filesystem via
`collect_snapshot` with `hook.worker_status["a"] = "blocked"` (no
`attention_pending` set).

---

### T2 — `AllTerminal` with empty team untested (Low)

**File:** `god_cli.rs:276–278`

Add: `wait --until all-terminal` on a run with no workers returns timeout, not
reached. (Currently it would return `Reached` due to vacuous all.)

---

### T3 — `report_ready` sentinel comparison is strict (note, not bug) (Very Low)

**File:** `god_cli.rs:377–384`

```rust
.find(|line| !line.trim().is_empty())
.is_some_and(|line| line == COMPLETION_SENTINEL)
```

`.find` skips whitespace-only lines but the final comparison is a strict `==`
without trimming the sentinel line. A sentinel written as
`"HERDR_TEAM_WORKER_COMPLETE "` (trailing space) would not match.
`str::lines()` strips line terminators, so `\r\n` endings are safe.
Whether trailing-space resistance is needed depends on how worker protocols are
generated. Currently the protocol writes the sentinel via `writeln!` which
produces no trailing space, so this is safe. Add a comment or a narrow test to
make the invariant explicit.

---

## Summary table

| ID | Axis | Severity | File:Line | Short description |
|----|------|----------|-----------|-------------------|
| S1 | Standards | Low | god_cli.rs:145–150 | Text `REPORT` label uses presence; wait uses readiness |
| S2 | Standards | Low-Med | god_cli.rs:368–369 | `attention` ORs `blocked` status; blocked→attention path untested |
| S3 | Standards | Low | god_cli.rs:276–278 | `AllTerminal` vacuously reaches on empty worker list |
| S4 | Standards | Very Low | god_cli.rs:173–183 | mtime read outside lock (benign race) |
| Spec-1 | Spec | Low | god_cli.rs:368–369 | `--until attention` / `--until blocked` overlap undocumented |
| T1 | Tests | Low | god_cli.rs:619–621 | blocked→attention path not exercised through `row()` |
| T2 | Tests | Low | god_cli.rs:276–278 | `AllTerminal` empty-team case not tested |
| T3 | Tests | Very Low | god_cli.rs:377–384 | Strict sentinel comparison; trailing-space resistance untested |

---

## Confirmations (no findings)

- Loop 2 fix is correct and complete: `report_ready` used in all wait conditions
  and `dead_worker`; `inbox_detects_stopped_not_done_and_read_marks_persist` tests
  the full sentinel-readiness path through the filesystem (god_cli.rs:705–774).
- `run.write_run_contents` is atomic at the filesystem level (tmp → rename) ✓
- `update_run_with_hook` lock/unlock is correct; unlock failure propagates ✓
- `--unread` retains missing reports (read==false when mtime==None) ✓ (spec §13)
- `list_active_runs` sorts by timestamp suffix; `select_run` picks newest ✓
- `adopted_worker` writes to both `spec.workers` AND `state.workers`
  (`adopt.rs:631–634`), so `collect_snapshot` rows are complete ✓
- `worker_status` and `attention_pending` are both `BTreeMap`, so `statuses`
  Vec has deterministic order ✓
- `select_wait_run` rejects explicitly inactive runs before polling ✓ (spec §13)
- Exit codes 0/2/3/4/1 match spec §13 ✓
- `AllReports` guards against empty team (`!s.rows.is_empty()`) ✓
- `AnyReport` dead-worker condition `!s.rows.iter().any(r.report_ready)` is
  correct: fires only when no worker has yet produced a ready report ✓

---

SLICE2 DONE
