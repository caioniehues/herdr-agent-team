//! Data-driven coding-agent launcher table from `docs/spec.md` section 3.

use crate::types::{LauncherEntry, LauncherTable};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("ticket 03: launcher table is not implemented")]
pub struct LauncherError;

pub fn default_launcher_table() -> LauncherTable {
    todo!("ticket 03: provide tested claude and codex launchers")
}

pub fn load_launcher_table(_config_dir: &Path) -> Result<LauncherTable, LauncherError> {
    todo!("ticket 03: load and merge agents.toml")
}

pub fn launcher_entry<'a>(
    _table: &'a LauncherTable,
    _agent: &str,
) -> Result<&'a LauncherEntry, LauncherError> {
    todo!("ticket 03: look up an agent kind")
}
