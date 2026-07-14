//! Generated worker communication protocol from `docs/spec.md` section 4.

use crate::types::{RunState, TeamSpec, WorkerSpec};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("ticket 04: AGENTS.md generation is not implemented")]
pub struct AgentsMdError;

pub fn render_agents_md(
    _team: &TeamSpec,
    _worker: &WorkerSpec,
    _run: &RunState,
    _run_dir: &Path,
) -> Result<String, AgentsMdError> {
    todo!("ticket 04: render generated AGENTS.md")
}
