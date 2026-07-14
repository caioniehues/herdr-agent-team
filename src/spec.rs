//! Team-spec parsing, resolution, validation, and dry-run output (spec section 2).

use crate::types::{LauncherTable, TeamSpec};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("ticket 02: team spec parsing is not implemented")]
pub struct SpecError;

pub fn parse_team_spec(_source: &str) -> Result<TeamSpec, SpecError> {
    todo!("ticket 02: parse herdr-team.toml")
}

pub fn load_team_spec(_path: &Path, _launchers: &LauncherTable) -> Result<TeamSpec, SpecError> {
    todo!("ticket 02: load and resolve herdr-team.toml")
}

pub fn team_spec_from_agents(
    _agents: &str,
    _cwd: &Path,
    _launchers: &LauncherTable,
) -> Result<TeamSpec, SpecError> {
    todo!("ticket 02: resolve the --agents shorthand")
}

pub fn validate_team_spec(_spec: &TeamSpec, _launchers: &LauncherTable) -> Result<(), SpecError> {
    todo!("ticket 02: validate a resolved team spec")
}

pub fn render_spawn_plan(_spec: &TeamSpec) -> String {
    todo!("ticket 02: render the --dry-run spawn plan")
}

pub fn spawn_command(_args: &[String]) -> Result<(), SpecError> {
    todo!("ticket 02: implement spawn --dry-run CLI handling")
}
