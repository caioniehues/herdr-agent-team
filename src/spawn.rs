//! Team preflight and worker launch flow from `docs/spec.md` section 4.

use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("ticket 07: team spawning is not implemented")]
pub struct SpawnError;

pub fn spawn_command(_args: &[String]) -> Result<(), SpawnError> {
    todo!("ticket 07: implement team spawn")
}

pub fn spawn_team(_spec_path: Option<&Path>, _agents: Option<&str>) -> Result<(), SpawnError> {
    todo!("ticket 07: preflight and spawn a resolved team")
}
