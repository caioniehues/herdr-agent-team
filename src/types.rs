//! Shared domain types from `docs/spec.md` sections 2 through 4.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// A fully resolved `herdr-team.toml` (spec section 2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamSpec {
    pub name: String,
    #[serde(default)]
    pub topology: Topology,
    #[serde(default = "default_cwd")]
    pub cwd: PathBuf,
    #[serde(default)]
    pub setup: Vec<String>,
    #[serde(default)]
    pub god: GodSpec,
    pub workers: Vec<WorkerSpec>,
}

/// How the team reaches its existing god session (spec section 2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GodSpec {
    #[serde(default = "default_god_target")]
    pub target: String,
}

impl Default for GodSpec {
    fn default() -> Self {
        Self {
            target: default_god_target(),
        }
    }
}

/// One coding-agent CLI participating in a team (spec section 2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerSpec {
    pub name: String,
    pub agent: String,
    pub role: String,
    pub worktree: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub brief: PathBuf,
}

/// Who may communicate directly inside a team (spec section 2).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Topology {
    #[default]
    Star,
    Mesh,
}

/// One data-driven agent definition from `agents.toml` (spec section 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LauncherEntry {
    pub command: Vec<String>,
    pub submit_verify: bool,
    pub reads_agents_md: AgentsMdMode,
    /// Whether a mid-turn `pane run` queues safely in this agent's TUI
    /// (spec section 3; ADR-0008). Conservative default: `false`.
    #[serde(default)]
    pub queues_midturn: bool,
}

/// Whether an agent reads generated AGENTS.md natively or needs a pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentsMdMode {
    Native,
    Pointer,
}

/// The launcher table keyed by the worker's configured agent kind.
pub type LauncherTable = BTreeMap<String, LauncherEntry>;

/// Durable run-board state written to `run.toml` (spec section 4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunState {
    pub spec: TeamSpec,
    pub god_pane_id: String,
    pub workers: BTreeMap<String, WorkerRunState>,
    pub lifecycle: RunLifecycle,
}

/// Live identifiers and lifecycle for one worker in a run (spec section 4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerRunState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<PathBuf>,
    #[serde(default)]
    pub adopted: bool,
    pub lifecycle: WorkerLifecycle,
}

/// Whether a durable team run is still active (spec sections 4 and 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunLifecycle {
    Active,
    Ended,
}

/// Spawn lifecycle retained for partial-failure diagnosis (spec section 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkerLifecycle {
    Pending,
    Running,
    Failed,
    Ended,
    Released,
    Orphaned,
}

fn default_cwd() -> PathBuf {
    PathBuf::from(".")
}

fn default_god_target() -> String {
    "self".to_owned()
}
