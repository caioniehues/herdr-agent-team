//! Durable run-board storage and matching from `docs/spec.md` sections 4 through 6.

use crate::types::RunState;
use serde_json::Value;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
#[error("ticket 06: run-board persistence is not implemented")]
pub struct RunError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunBoard {
    pub dir: PathBuf,
    pub state: RunState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchedWorker {
    pub run: RunBoard,
    pub worker_name: String,
}

pub fn create_run(_state_dir: &Path, _state: RunState) -> Result<RunBoard, RunError> {
    todo!("ticket 06: create run.toml and inbox")
}

pub fn load_run(_run_dir: &Path) -> Result<RunBoard, RunError> {
    todo!("ticket 06: load run.toml")
}

pub fn save_run(_run: &RunBoard) -> Result<(), RunError> {
    todo!("ticket 06: persist run.toml")
}

pub fn list_active_runs(_state_dir: &Path) -> Result<Vec<RunBoard>, RunError> {
    todo!("ticket 06: list active run boards")
}

pub fn match_pane(_state_dir: &Path, _pane_id: &str) -> Result<Option<MatchedWorker>, RunError> {
    todo!("ticket 06: match pane id to an active run worker")
}

pub fn append_event(_run_dir: &Path, _event: &Value) -> Result<(), RunError> {
    todo!("ticket 06: append and flush inbox/events.jsonl")
}

pub fn mark_ended(_run: &mut RunBoard) -> Result<(), RunError> {
    todo!("ticket 06: mark a run ended")
}
