//! Team status and teardown commands from `docs/spec.md` section 6.

use crate::herdr::HerdrClient;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("ticket 09: status and kill are not implemented")]
pub struct StatusKillError;

pub fn status_command(_args: &[String]) -> Result<(), StatusKillError> {
    todo!("ticket 09: implement team status")
}

pub fn kill_command(_args: &[String]) -> Result<(), StatusKillError> {
    todo!("ticket 09: implement team kill")
}

pub fn status_run(
    _run_dir: &Path,
    _json: bool,
    _herdr: &HerdrClient,
) -> Result<String, StatusKillError> {
    todo!("ticket 09: join run.toml with live Herdr agent state")
}

pub fn kill_run(
    _run_dir: &Path,
    _remove_worktrees: bool,
    _herdr: &HerdrClient,
) -> Result<(), StatusKillError> {
    todo!("ticket 09: close recorded workspaces and mark the run ended")
}
