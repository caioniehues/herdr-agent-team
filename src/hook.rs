//! Push-based worker status report hook from `docs/spec.md` section 5.

use crate::herdr::HerdrClient;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("ticket 08: agent status hook is not implemented")]
pub struct HookError;

pub fn hook_command() -> Result<(), HookError> {
    todo!("ticket 08: read HERDR_PLUGIN_EVENT_JSON and process the status flip")
}

pub fn on_agent_status(
    _event_json: &str,
    _state_dir: &Path,
    _herdr: &HerdrClient,
) -> Result<(), HookError> {
    todo!("ticket 08: persist the event and inject a report pointer")
}
