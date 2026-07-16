# Slice 6 — socket review (#58)

Fixed point reviewed: `v1.0.0` / `aa0c0e05b0a26074e5f11328a781b41cb633f669` (current HEAD).

Scope: `src/socket.rs`, `src/socket_backend.rs`; spec axis: ADR-0011. The stage-0 `team wait` executions establish reachability but not that a direct subscription, rather than CLI fallback, produced the wake-up. This review treats the socket as an unproven acceleration path and verifies the fallback boundary.

## Findings

1. `src/socket.rs:138-144`: 🟡 risk — an explicit `HERDR_TEAM_BACKEND=socket` discards every connect, schema, handshake, and protocol-validation error through `.ok()`, so `board_command`/`wait_command` silently use CLI (`src/board.rs:206-215`, `src/god_cli.rs:194-203`). ADR-0011 requires the CLI fallback, but also says an unsupported schema must be refused with a clear error (`docs/adr/0011-direct-socket-backend.md:35-37`, `docs/adr/0011-direct-socket-backend.md:51-65`). Failure scenario: a user dogfoods the opted-in backend against a stale socket or schema drift and sees healthy CLI waits forever, concluding the socket acceleration works. Disposition: fix-ticket — retain the socket initialization error and emit a clear one-shot fallback diagnostic/trace marker (or make explicit socket selection fail); test each rejected handshake/schema/path condition through `try_from_env` and both collectors.

2. `src/socket.rs:337-352` / `src/socket.rs:422-455`: 🔴 bug — public types and methods unconditionally name `UnixStream`, while its import is Unix-only (`src/socket.rs:11-12`). On Windows the module cannot compile even when the backend is not selected, rather than retaining the CLI fallback. ADR-0011 explicitly requires the public socket abstraction to support Windows or defer Windows to CLI fallback (`docs/adr/0011-direct-socket-backend.md:71-81`). Failure scenario: a Windows user cannot build/run the plugin at all because experimental Unix-only transport is compiled into the default binary. Disposition: fix-ticket — `cfg(unix)` the Unix implementation and expose `try_from_env` as CLI fallback on other platforms, or introduce a platform transport abstraction/named-pipe implementation; add a Windows-target compile check.

## Standards axis

The transport handles the substantive protocol discipline: it validates runtime schema before connecting (`src/socket.rs:147-167`, `src/socket.rs:543-560`), validates handshake protocol (`src/socket.rs:281-303`), checks request IDs and typed results (`src/socket.rs:174-185`, `src/socket.rs:305-328`, `src/socket.rs:348-399`), bounds frames (`src/socket.rs:481-506`), and writes redacted trace fields only (`src/socket.rs:402-419`). Hermetic fake-socket tests cover malformed frames, reconnect re-snapshots, typed payload rejection, controller ownership, and bounded fallback (`src/socket.rs:784-1250`). They do not exercise the explicit environment-selection failure as a visible outcome, and cannot expose the non-Unix compilation break from the Unix-only test host.

## Spec axis

Mutating `HerdrApi` calls correctly delegate to the CLI fallback (`src/socket_backend.rs:272-315`), consistent with ADR-0011’s limited socket scope (`docs/adr/0011-direct-socket-backend.md:41-50`). `CollectorSocketController` snapshots before subscribing and re-enters snapshot state after transport loss (`src/socket_backend.rs:68-179`), while its wrappers continue to derive board/wait snapshots from the durable fallback (`src/socket_backend.rs:215-269`). Thus fallback semantics are intentionally identical for completion truth: `wait_with` bases conditions on `run.toml` and inbox collection (`src/god_cli.rs:214-242`), and the board collector uses the durable board snapshot (`src/board.rs:287-305`). The two findings concern honest gating and platform availability, not this semantic preservation.

## Deep-module lens

1. **Deletion test:** `socket_backend` can be deleted with callers almost unchanged because `BoardCollector` and `GodCollector` already provide the seam (`src/board.rs:53-63`, `src/god_cli.rs:61-70`); that is correct for an experimental acceleration. `socket` itself is a necessary concentrated transport module, but its Unix type leakage prevents fully deleting it from a non-Unix build.
2. **Caller leverage:** callers select one collector and keep their durable-state semantics; they do not see frames, reconnects, or subscriptions (`src/socket_backend.rs:203-269`). This is a deep interface. The opt-in constructor weakens leverage by hiding whether the selected backend is actually in use.
3. **Seam placement:** `HerdrApi` owns CLI mutation delegation (`src/socket_backend.rs:272-315`) and the single controller owns snapshot/subscription/reconnect lifecycle (`src/socket_backend.rs:55-179`), exactly matching ADR-0011’s intended seams. Board and wait only map controller results to their respective fallback policies.
4. **Outcome testing:** fake sockets test protocol outcomes and compare socket-backed collection with CLI fixtures (`src/socket.rs:850-879`), rather than probing private stream state. Add outcome tests for explicit socket selection failure visibility and non-Unix availability; the stage-0 live evidence remains insufficient to prove that a subscription—not fallback—caused either observed wait.

SLICE6 DONE
