//! herdmates — shared library backing the `herdmates` binary and the
//! `teammux` shim binary (issue #85 commit 3).
//!
//! Split out of what was a single-binary crate so `teammux` (src/bin/
//! teammux.rs) can reuse `idmap`/`tmuxargs`/`teammux` without duplicating
//! source. `src/main.rs` re-imports everything below via `use herdmates::*;`
//! so its existing subcommand dispatch is unchanged.

pub mod adopt;
pub mod agents_md;
pub mod attention;
pub mod audit;
pub mod board;
pub mod focus_pane;
pub mod focusfile;
pub mod gather;
pub mod god_cli;
pub mod herdr;
pub mod hook;
pub mod idmap;
pub mod jump;
pub mod launcher;
pub mod metadata;
pub mod msg;
pub mod pane_board;
pub mod paths;
pub mod pump;
pub mod reconcile;
pub mod recorder;
pub mod run;
pub mod signal_engine;
pub mod socket;
#[cfg(unix)]
pub mod socket_backend;
pub mod spawn;
pub mod spec;
pub mod status_kill;
pub mod teamfiles;
pub mod teammux;
pub mod teammux_launch;
pub mod tmuxargs;
pub mod tokens;
pub mod types;
