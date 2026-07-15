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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
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
    #[serde(default, skip_serializing_if = "HerdrSessionIdentity::is_empty")]
    pub herdr_session: HerdrSessionIdentity,
    pub workers: BTreeMap<String, WorkerRunState>,
    pub lifecycle: RunLifecycle,
}

/// Live identifiers and lifecycle for one worker in a run (spec section 4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerRunState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_session: Option<crate::herdr::AgentSession>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<PathBuf>,
    #[serde(default)]
    pub adopted: bool,
    #[serde(default)]
    pub launch_checkpoint: WorkerLaunchCheckpoint,
    pub lifecycle: WorkerLifecycle,
}

/// Durable progress through the spawn launch flow (spec section 4).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerLaunchCheckpoint {
    Pending,
    ResourcesReady,
    #[default]
    BriefSubmitted,
}

/// The Herdr runtime selected when this run was created or adopted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HerdrSessionIdentity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub socket_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl HerdrSessionIdentity {
    pub fn from_environment(socket_path: Option<PathBuf>, name: Option<String>) -> Self {
        Self { socket_path, name }
    }

    pub fn is_empty(&self) -> bool {
        self.socket_path.is_none() && self.name.is_none()
    }
}

pub fn current_herdr_session_identity() -> HerdrSessionIdentity {
    HerdrSessionIdentity::from_environment(
        std::env::var_os("HERDR_SOCKET_PATH").map(PathBuf::from),
        std::env::var("HERDR_SESSION").ok(),
    )
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
