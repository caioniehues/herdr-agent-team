# Fix #63 + #64 — socket honesty + portability

Worker: **fix63-64**  
Branch: `fix/63-64-socket`  
Base: v1.0.0 (`aa0c0e0`)  
Date: 2026-07-15

---

## Fix #64 — UnixStream in non-cfg'd signatures

### RED evidence

Grep confirmed three un-cfg'd uses of `UnixStream` in `src/socket.rs` that fail
compilation on non-Unix targets:

```
src/socket.rs:337:  fn open_stream(&self, timeout: Duration) -> Result<UnixStream, HerdrError>
src/socket.rs:352:  fn open_subscription(&self, …) -> Result<BufReader<UnixStream>, HerdrError>
src/socket.rs:423:  pub(crate) struct SubscriptionStream { reader: BufReader<UnixStream>, … }
```

`UnixStream` is imported under `#[cfg(unix)]` (line 11-12) but the three sites
above are in an unconditioned impl block and struct — on Windows, `UnixStream`
is not in scope and they fail with E0412.

Windows GNU target (`x86_64-pc-windows-gnu`) is not installed on this machine
and `rustup target add` was denied permission. The fix was verified by making
the cfg boundary **total** — no Unix-only names appear outside a `#[cfg(unix)]`
context in production code — as documented below.

### Root cause

`SocketClient<C>`'s transport-layer impl block and the `SubscriptionStream`
type were not guarded. `open_stream` and `open_subscription` return
`UnixStream`-bearing types without `cfg`. The module-level
`connect` free function was already correctly split with
`#[cfg(unix)]` / `#[cfg(not(unix))]`, but the impl methods calling it were not.

### Fix summary

| File | Change |
|---|---|
| `src/socket.rs` | `#[cfg(unix)]` on `impl<C: HerdrApi> SocketClient<C>` (transport block) |
| `src/socket.rs` | `#[cfg(unix)]` on `SubscriptionStream` struct |
| `src/socket.rs` | `#[cfg(unix)]` on `SubscriptionPoll` enum |
| `src/socket.rs` | `#[cfg(unix)]` on `impl SubscriptionStream` |
| `src/socket.rs` | `try_from_env` split into `#[cfg(unix)]` + `#[cfg(not(unix))]` stubs (non-Unix returns `Ok(None)`) |
| `src/socket.rs` | Tests gated to `#[cfg(all(test, unix))]` (they use `UnixListener`) |
| `src/main.rs` | `#[cfg(unix)]` on `pub mod socket_backend` |
| `src/board.rs` | `#[cfg(unix)]` gate on the `SocketBoardCollector` branch |
| `src/god_cli.rs` | `#[cfg(unix)]` gate on the `SocketGodCollector` let-binding |

The cfg boundary is now total: `UnixStream` appears only inside `#[cfg(unix)]`
blocks. On non-Unix, `try_from_env()` always returns `Ok(None)` and no
socket-backend types are referenced.

### GREEN evidence

```
cargo fmt --check   → exit 0  (no diffs)
cargo clippy --all-targets -- -D warnings → exit 0 (0 warnings)
cargo test          → 186 passed; 0 failed
```

---

## Fix #63 — explicit socket selection silently falls back

### RED evidence

Compile-time RED tests written first (before `try_from_parts` existed):

```
src/socket.rs:1297  SocketClient::try_from_parts(…)  → E0599
src/socket.rs:1314  SocketClient::try_from_parts(…)  → E0599
src/socket.rs:1326  SocketClient::try_from_parts(…)  → E0599
src/socket.rs:1332  SocketClient::try_from_parts(…)  → E0599
```

`cargo test` confirmed compile failure before the fix was applied.

Behavioral RED: the original code was:

```rust
// src/socket.rs — try_from_env before fix
let path = std::env::var_os("HERDR_SOCKET_PATH").map(PathBuf::from)?;
Self::connect(path, HerdrClient::from_env()).ok()   // ← .ok() swallows Err → None
```

With `HERDR_TEAM_BACKEND=socket` and any connect/schema/handshake error, this
returns `None`, indistinguishable from "backend not selected".  Callers in
`board_command` / `wait_command` silently fell through to the CLI backend with
no diagnostic visible to the operator.

### Root cause

`try_from_env` used `.ok()` to convert the `Result` to `Option`, making every
initialization error (schema mismatch, wrong protocol, socket path not found,
handshake failure) vanish when the user has explicitly opted into socket via
`HERDR_TEAM_BACKEND=socket`.  Implicit/auto selection (variable absent or set
to anything other than `"socket"`) should still silently fall back — that
design is correct and is preserved by the fix.

### Fix summary

| File | Change |
|---|---|
| `src/socket.rs` | `try_from_env` return type changed to `Result<Option<Self>, HerdrError>` |
| `src/socket.rs` | New `try_from_parts(backend, socket_path, fallback)` inner helper — testable without touching env vars |
| `src/socket.rs` | Missing `HERDR_SOCKET_PATH` with explicit backend now returns `Err` (was silently `None`) |
| `src/board.rs` | `board_command` propagates `Err` as `BoardError::Usage(…)` |
| `src/god_cli.rs` | `wait_command` propagates `Err` as `GodCliError::Usage(…)` |

The fix preserves the existing behavior for implicit/auto selection
(`backend != "socket"` → `Ok(None)` immediately, no connect attempt).

### GREEN evidence (three new tests, all pass)

```
socket::tests::explicit_selection_schema_error_is_surfaced_not_silenced … ok
socket::tests::explicit_selection_missing_socket_path_is_an_error … ok
socket::tests::implicit_selection_connect_failure_stays_silent … ok
```

These test `try_from_parts` directly:
- `backend="socket"` + schema failure (FakeHerdr returns `{}`) → `Err`, not `Ok(None)`
- `backend="socket"` + `socket_path=None` → `Err` mentioning `HERDR_SOCKET_PATH`
- `backend=""` or `backend="auto"` + non-existent path → `Ok(None)` (silent, by design)

---

## Gate output (final)

```
cargo fmt --check        → exit 0
cargo clippy --all-targets -- -D warnings → exit 0
cargo test               → 186 passed; 0 failed; 0 ignored  (0.31s)
```

---

## Files touched

- `src/socket.rs` — cfg gating, `try_from_parts`, `try_from_env` return type, test gate
- `src/socket_backend.rs` — no changes (gated via `main.rs`)
- `src/main.rs` — `#[cfg(unix)] pub mod socket_backend`
- `src/board.rs` — unix-gated match, Result handling
- `src/god_cli.rs` — cfg-gated let bindings, Result handling

---

FIX64 GREEN
FIX63 GREEN
