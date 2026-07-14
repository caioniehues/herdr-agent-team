//! Typed wrapper for Herdr CLI operations required by `docs/spec.md` sections 4 through 6.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("ticket 05: Herdr client is not implemented")]
pub struct HerdrError;

#[derive(Debug, Clone)]
pub struct HerdrClient {
    pub binary: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceRef {
    pub workspace_id: String,
    pub pane_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeRef {
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneInfo {
    pub pane_id: String,
    pub workspace_id: String,
    pub agent: Option<String>,
    pub agent_status: Option<String>,
    pub cwd: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentInfo {
    pub pane_id: String,
    pub workspace_id: String,
    pub agent: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitOutcome {
    Reached,
    TimedOut,
}

impl HerdrClient {
    pub fn from_env() -> Self {
        todo!("ticket 05: resolve HERDR_BIN_PATH")
    }

    pub fn workspace_create(&self, _cwd: &Path, _label: &str) -> Result<WorkspaceRef, HerdrError> {
        todo!("ticket 05: herdr workspace create")
    }

    pub fn workspace_close(&self, _workspace_id: &str) -> Result<(), HerdrError> {
        todo!("ticket 05: herdr workspace close")
    }

    pub fn worktree_create(&self, _repo: &Path, _branch: &str) -> Result<WorktreeRef, HerdrError> {
        todo!("ticket 05: herdr worktree create")
    }

    pub fn worktree_remove(&self, _path: &Path) -> Result<(), HerdrError> {
        todo!("ticket 05: herdr worktree remove")
    }

    pub fn pane_split(&self, _workspace_id: &str, _cwd: &Path) -> Result<PaneInfo, HerdrError> {
        todo!("ticket 05: herdr pane split")
    }

    pub fn pane_run(&self, _pane_id: &str, _input: &str) -> Result<(), HerdrError> {
        todo!("ticket 05: herdr pane run")
    }

    pub fn pane_read(&self, _pane_id: &str) -> Result<String, HerdrError> {
        todo!("ticket 05: herdr pane read")
    }

    pub fn pane_rename(&self, _pane_id: &str, _title: &str) -> Result<(), HerdrError> {
        todo!("ticket 05: herdr pane rename")
    }

    pub fn agent_wait(
        &self,
        _pane_id: &str,
        _status: &str,
        _timeout: Duration,
    ) -> Result<WaitOutcome, HerdrError> {
        todo!("ticket 05: herdr agent wait")
    }

    pub fn agent_list(&self) -> Result<Vec<AgentInfo>, HerdrError> {
        todo!("ticket 05: herdr agent list")
    }

    pub fn pane_get(&self, _pane_id: &str) -> Result<PaneInfo, HerdrError> {
        todo!("ticket 05: herdr pane get")
    }
}
