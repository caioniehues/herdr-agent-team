# Loop 48 — premature result readiness

## RED

Command:

```text
cargo test god_cli::tests::inbox_detects_stopped_not_done_and_read_marks_persist -- --exact
```

Verbatim failure before the fix:

```text
---- god_cli::tests::inbox_detects_stopped_not_done_and_read_marks_persist stdout ----

thread 'god_cli::tests::inbox_detects_stopped_not_done_and_read_marks_persist' (420948) panicked at src/god_cli.rs:730:9:
an unfinished report must not satisfy wait report:a
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace


test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 182 filtered out; finished in 0.00s
```

The test creates a durable run state, writes `one\ntwo\n` to
`inbox/a.md`, and asserts that `wait report:a` is not met. The assertion failed
because the pre-fix row marked every extant report file as ready.

## Root cause

`src/god_cli.rs:261` used `InboxRow.report_present` for all report wait
conditions. `src/god_cli.rs:351` set that field from report metadata alone
(`mtime.is_some()`), so a file became ready before its writer had finished.

## Readiness contract

**Result ready** means the report's final non-empty line is the existing worker
protocol completion sentinel, `HERDR_TEAM_WORKER_COMPLETE`. File existence is
not readiness. This reuses the plugin's existing sentinel convention; the
generated protocol now requires workers to write it as the report's final line
and then print that same sentinel.

## Fix summary

- Added `report_ready` to the inbox snapshot, preserving `report_present` for
  actual file presence.
- Made `any-report`, `report:<worker>`, `all-reports`, and dead-worker checks
  use sentinel readiness.
- Added `report_ready` file predicate and a regression test that proves an
  unfinished report is not ready and the sentinel-complete report is ready.
- Updated generated worker protocol golden fixtures, `docs/spec.md`, and
  `CONTEXT.md` with the Result ready / completion sentinel vocabulary.

## GREEN evidence

Focused regression:

```text
cargo test god_cli::tests::inbox_detects_stopped_not_done_and_read_marks_persist -- --exact
cargo test: 1 passed, 182 filtered out (1 suite, 0.00s)
```

Required gate output tail:

```text
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo clippy: No issues found
cargo test: 183 passed (1 suite, 0.31s)
```

## Files touched

- `src/god_cli.rs`
- `src/socket.rs`
- `src/agents_md.rs`
- `tests/golden/agents_md_star.md`
- `tests/golden/agents_md_mesh.md`
- `docs/spec.md`
- `CONTEXT.md`
- `STATUS.log`
- `docs/reviews/loops/loop48-report.md`
