//! Typed wrapper for Herdr CLI operations required by `docs/spec.md` sections 4, 6, and 9.

use crate::metadata::{MetadataUpdate, SOURCE};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum HerdrError {
    #[error("public socket {operation} failed: {source}")]
    Transport {
        operation: String,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to spawn `{argv}`: {source}")]
    Spawn {
        argv: String,
        #[source]
        source: std::io::Error,
    },

    #[error("`{argv}` exited with status {status:?}: {stderr}")]
    Command {
        argv: String,
        status: Option<i32>,
        stderr: String,
    },

    #[error("invalid response from `{argv}`: {message}")]
    InvalidResponse { argv: String, message: String },
}

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PaneInfo {
    pub pane_id: String,
    pub workspace_id: String,
    pub tab_id: Option<String>,
    pub agent: Option<String>,
    pub agent_id: Option<String>,
    pub agent_session: Option<AgentSession>,
    pub agent_status: Option<String>,
    pub cwd: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentInfo {
    pub pane_id: String,
    pub workspace_id: String,
    pub agent: Option<String>,
    pub agent_id: Option<String>,
    pub agent_session: Option<AgentSession>,
    #[serde(rename = "agent_status")]
    pub status: Option<String>,
}

/// An opaque agent-session reference reported by Herdr.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSession {
    pub source: String,
    pub agent: String,
    pub kind: String,
    pub value: String,
}

#[derive(Debug, Deserialize)]
struct PaneInfoWire {
    pane_id: String,
    workspace_id: String,
    #[serde(default)]
    tab_id: Option<String>,
    agent: Option<String>,
    agent_session: Option<AgentSession>,
    agent_status: Option<String>,
    cwd: Option<PathBuf>,
}

impl<'de> Deserialize<'de> for PaneInfo {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = PaneInfoWire::deserialize(deserializer)?;
        let agent_id = wire
            .agent_session
            .as_ref()
            .map(|session| session.value.clone());
        Ok(Self {
            pane_id: wire.pane_id,
            workspace_id: wire.workspace_id,
            tab_id: wire.tab_id,
            agent: wire.agent,
            agent_id,
            agent_session: wire.agent_session,
            agent_status: wire.agent_status,
            cwd: wire.cwd,
        })
    }
}

#[derive(Debug, Deserialize)]
struct AgentInfoWire {
    pane_id: String,
    workspace_id: String,
    agent: Option<String>,
    agent_session: Option<AgentSession>,
    #[serde(rename = "agent_status")]
    status: Option<String>,
}

impl<'de> Deserialize<'de> for AgentInfo {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = AgentInfoWire::deserialize(deserializer)?;
        let agent_id = wire
            .agent_session
            .as_ref()
            .map(|session| session.value.clone());
        Ok(Self {
            pane_id: wire.pane_id,
            workspace_id: wire.workspace_id,
            agent: wire.agent,
            agent_id,
            agent_session: wire.agent_session,
            status: wire.status,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitOutcome {
    Reached,
    TimedOut,
}

/// Operations used by the plugin's command and hook seams.
///
/// This is deliberately the single abstraction over Herdr so command paths can
/// be tested without invoking the CLI and future backends can implement the
/// same contract.
pub trait HerdrApi {
    fn workspace_create(&self, _: &Path, _: &str) -> Result<WorkspaceRef, HerdrError> {
        Err(unsupported_api())
    }
    fn workspace_close(&self, _: &str) -> Result<(), HerdrError> {
        Err(unsupported_api())
    }
    fn worktree_create(&self, _: &Path, _: &str) -> Result<WorktreeRef, HerdrError> {
        Err(unsupported_api())
    }
    fn worktree_remove(&self, _: &Path) -> Result<(), HerdrError> {
        Err(unsupported_api())
    }
    fn pane_split(&self, _: &str, _: &Path) -> Result<PaneInfo, HerdrError> {
        Err(unsupported_api())
    }
    /// Split off an existing pane (as opposed to [`HerdrApi::pane_split`],
    /// which splits a freshly created workspace's root pane in a fixed
    /// direction) — the shape `teammux`'s `split-window` needs (issue #85
    /// commit 5): a target pane, a direction, and an optional ratio.
    fn pane_split_pane(
        &self,
        _target_pane_id: &str,
        _direction: &str,
        _ratio: Option<f64>,
    ) -> Result<PaneInfo, HerdrError> {
        Err(unsupported_api())
    }
    fn pane_run(&self, _: &str, _: &str) -> Result<(), HerdrError> {
        Err(unsupported_api())
    }
    fn pane_read(&self, _: &str) -> Result<String, HerdrError> {
        Err(unsupported_api())
    }
    fn pane_rename(&self, _: &str, _: &str) -> Result<(), HerdrError> {
        Err(unsupported_api())
    }
    /// `kill-pane -t %N` (issue #85 commit 6): close the pane outright.
    fn pane_close(&self, _pane_id: &str) -> Result<(), HerdrError> {
        Err(unsupported_api())
    }
    /// `resize-pane -t %N -x AMOUNT` (issue #85 commit 6): herdr models
    /// resize as a directional border move, not tmux's absolute-size target,
    /// so the shim maps it onto a fixed direction (see `teammux::resize_pane`
    /// for the documented assumption) with `amount` as a 0-1 ratio.
    fn pane_resize(
        &self,
        _pane_id: &str,
        _direction: &str,
        _amount: Option<f64>,
    ) -> Result<(), HerdrError> {
        Err(unsupported_api())
    }
    fn agent_wait(&self, _: &str, _: &str, _: Duration) -> Result<WaitOutcome, HerdrError> {
        Err(unsupported_api())
    }
    fn agent_list(&self) -> Result<Vec<AgentInfo>, HerdrError> {
        Err(unsupported_api())
    }
    fn pane_get(&self, _: &str) -> Result<PaneInfo, HerdrError> {
        Err(unsupported_api())
    }
    fn pane_list(&self, _workspace_id: Option<&str>) -> Result<Vec<PaneInfo>, HerdrError> {
        Err(unsupported_api())
    }
    fn api_schema(&self) -> Result<String, HerdrError> {
        Err(unsupported_api())
    }
    fn pane_report_metadata(&self, _: &str, _: &MetadataUpdate) -> Result<(), HerdrError> {
        Err(unsupported_api())
    }
    fn notification_show(&self, _: &str, _: &str, _: &str) -> Result<(), HerdrError> {
        Err(unsupported_api())
    }

    fn health_check(&self) -> Result<(), HerdrError> {
        self.agent_list().map(|_| ())
    }
}

fn unsupported_api() -> HerdrError {
    HerdrError::Command {
        argv: "unsupported HerdrApi operation".to_owned(),
        status: None,
        stderr: "operation not implemented by this backend".to_owned(),
    }
}

impl HerdrClient {
    pub fn from_env() -> Self {
        let binary = std::env::var_os("HERDR_BIN_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("herdr"));
        Self { binary }
    }

    pub fn workspace_create(&self, cwd: &Path, label: &str) -> Result<WorkspaceRef, HerdrError> {
        let args = args(["workspace", "create"])
            .with("--cwd")
            .with(cwd)
            .with("--label")
            .with(label)
            .finish();
        let stdout = self.invoke(&args)?;
        parse_workspace_create(&stdout).map_err(|message| self.invalid_response(&args, message))
    }

    pub fn workspace_close(&self, workspace_id: &str) -> Result<(), HerdrError> {
        let args = args(["workspace", "close"]).with(workspace_id).finish();
        let stdout = self.invoke(&args)?;
        parse_ok(&stdout).map_err(|message| self.invalid_response(&args, message))
    }

    pub fn worktree_create(&self, repo: &Path, branch: &str) -> Result<WorktreeRef, HerdrError> {
        let args = args(["worktree", "create"])
            .with("--cwd")
            .with(repo)
            .with("--branch")
            .with(branch)
            .with("--json")
            .finish();
        let stdout = self.invoke(&args)?;
        parse_worktree_create(&stdout).map_err(|message| self.invalid_response(&args, message))
    }

    pub fn worktree_remove(&self, path: &Path) -> Result<(), HerdrError> {
        // Protocol 16 removes worktrees by opaque workspace ID. The fixed public seam
        // accepts the path returned by `worktree_create`, so resolve the ID from Herdr.
        let list_args = args(["worktree", "list"])
            .with("--cwd")
            .with(path)
            .with("--json")
            .finish();
        let stdout = self.invoke(&list_args)?;
        let workspace_id = parse_worktree_workspace_id(&stdout, path)
            .map_err(|message| self.invalid_response(&list_args, message))?;

        let remove_args = args(["worktree", "remove"])
            .with("--workspace")
            .with(workspace_id)
            .with("--json")
            .finish();
        let stdout = self.invoke(&remove_args)?;
        parse_worktree_remove(&stdout)
            .map_err(|message| self.invalid_response(&remove_args, message))
    }

    pub fn pane_split(&self, workspace_id: &str, cwd: &Path) -> Result<PaneInfo, HerdrError> {
        let args = args(["pane", "split"])
            .with("--workspace")
            .with(workspace_id)
            .with("--direction")
            .with("right")
            .with("--cwd")
            .with(cwd)
            .with("--no-focus")
            .finish();
        let stdout = self.invoke(&args)?;
        parse_pane_info(&stdout).map_err(|message| self.invalid_response(&args, message))
    }

    pub fn pane_split_pane(
        &self,
        target_pane_id: &str,
        direction: &str,
        ratio: Option<f64>,
    ) -> Result<PaneInfo, HerdrError> {
        let mut command = args(["pane", "split"])
            .with(target_pane_id)
            .with("--direction")
            .with(direction)
            .with("--no-focus");
        if let Some(ratio) = ratio {
            command = command.with("--ratio").with(ratio.to_string());
        }
        let args = command.finish();
        let stdout = self.invoke(&args)?;
        parse_pane_info(&stdout).map_err(|message| self.invalid_response(&args, message))
    }

    pub fn pane_run(&self, pane_id: &str, input: &str) -> Result<(), HerdrError> {
        let args = args(["pane", "run"]).with(pane_id).with(input).finish();
        self.invoke(&args)?;
        Ok(())
    }

    pub fn pane_read(&self, pane_id: &str) -> Result<String, HerdrError> {
        let args = args(["pane", "read"]).with(pane_id).finish();
        self.invoke(&args)
    }

    pub fn pane_rename(&self, pane_id: &str, title: &str) -> Result<(), HerdrError> {
        let args = args(["pane", "rename"]).with(pane_id).with(title).finish();
        let stdout = self.invoke(&args)?;
        parse_pane_info(&stdout)
            .map(|_| ())
            .map_err(|message| self.invalid_response(&args, message))
    }

    pub fn pane_close(&self, pane_id: &str) -> Result<(), HerdrError> {
        let args = args(["pane", "close"]).with(pane_id).finish();
        self.invoke(&args)?;
        Ok(())
    }

    pub fn pane_resize(
        &self,
        pane_id: &str,
        direction: &str,
        amount: Option<f64>,
    ) -> Result<(), HerdrError> {
        let mut command = args(["pane", "resize"])
            .with("--direction")
            .with(direction)
            .with("--pane")
            .with(pane_id);
        if let Some(amount) = amount {
            command = command.with("--amount").with(amount.to_string());
        }
        let args = command.finish();
        self.invoke(&args)?;
        Ok(())
    }

    pub fn agent_wait(
        &self,
        pane_id: &str,
        status: &str,
        timeout: Duration,
    ) -> Result<WaitOutcome, HerdrError> {
        let timeout_ms = timeout.as_millis().min(u128::from(u64::MAX)).to_string();
        let args = args(["agent", "wait"])
            .with(pane_id)
            .with("--status")
            .with(status)
            .with("--timeout")
            .with(timeout_ms)
            .finish();

        match self.invoke(&args) {
            Ok(stdout) => {
                parse_agent_wait(&stdout, pane_id, status)
                    .map_err(|message| self.invalid_response(&args, message))?;
                Ok(WaitOutcome::Reached)
            }
            Err(HerdrError::Command {
                status: Some(1), ..
            }) => Ok(WaitOutcome::TimedOut),
            Err(error) => Err(error),
        }
    }

    pub fn agent_list(&self) -> Result<Vec<AgentInfo>, HerdrError> {
        let args = args(["agent", "list"]).finish();
        let stdout = self.invoke(&args)?;
        parse_agent_list(&stdout).map_err(|message| self.invalid_response(&args, message))
    }

    pub fn pane_get(&self, pane_id: &str) -> Result<PaneInfo, HerdrError> {
        let args = args(["pane", "get"]).with(pane_id).finish();
        let stdout = self.invoke(&args)?;
        parse_pane_info(&stdout).map_err(|message| self.invalid_response(&args, message))
    }

    pub fn pane_list(&self, workspace_id: Option<&str>) -> Result<Vec<PaneInfo>, HerdrError> {
        let mut command = args(["pane", "list"]);
        if let Some(workspace_id) = workspace_id {
            command = command.with("--workspace").with(workspace_id);
        }
        let args = command.finish();
        let stdout = self.invoke(&args)?;
        parse_pane_list(&stdout).map_err(|message| self.invalid_response(&args, message))
    }

    pub fn api_schema(&self) -> Result<String, HerdrError> {
        self.invoke(&args(["api", "schema", "--json"]).finish())
    }

    pub fn pane_report_metadata(
        &self,
        pane_id: &str,
        update: &MetadataUpdate,
    ) -> Result<(), HerdrError> {
        let mut command = args(["pane", "report-metadata"])
            .with(pane_id)
            .with("--source")
            .with(SOURCE);
        if let Some(title) = &update.title {
            command = command.with("--title").with(title);
        }
        if let Some(display_agent) = &update.display_agent {
            command = command.with("--display-agent").with(display_agent);
        }
        if let Some(custom_status) = &update.custom_status {
            command = command.with("--custom-status").with(custom_status);
        }
        if let Some((status, label)) = &update.state_label {
            command = command
                .with("--state-label")
                .with(format!("{status}={label}"));
        }
        if let Some(seq) = update.seq {
            command = command.with("--seq").with(seq.to_string());
        }
        if let Some(ttl_ms) = update.ttl_ms {
            command = command.with("--ttl-ms").with(ttl_ms.to_string());
        }
        self.invoke(&command.finish())?;
        Ok(())
    }

    pub fn notification_show(
        &self,
        title: &str,
        body: &str,
        sound: &str,
    ) -> Result<(), HerdrError> {
        let args = args(["notification", "show"])
            .with(title)
            .with("--body")
            .with(body)
            .with("--sound")
            .with(sound)
            .finish();
        self.invoke(&args)?;
        Ok(())
    }

    fn invoke(&self, args: &[OsString]) -> Result<String, HerdrError> {
        let argv = display_argv(&self.binary, args);
        let output = Command::new(&self.binary)
            .args(args)
            .output()
            .map_err(|source| HerdrError::Spawn {
                argv: argv.clone(),
                source,
            })?;

        if !output.status.success() {
            let stderr = first_line(&output.stderr)
                .or_else(|| first_line(&output.stdout))
                .unwrap_or_else(|| "command failed without diagnostic output".to_owned());
            return Err(HerdrError::Command {
                argv,
                status: output.status.code(),
                stderr,
            });
        }

        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    fn invalid_response(&self, args: &[OsString], message: String) -> HerdrError {
        HerdrError::InvalidResponse {
            argv: display_argv(&self.binary, args),
            message,
        }
    }
}

impl HerdrApi for HerdrClient {
    fn workspace_create(&self, cwd: &Path, label: &str) -> Result<WorkspaceRef, HerdrError> {
        Self::workspace_create(self, cwd, label)
    }
    fn workspace_close(&self, workspace_id: &str) -> Result<(), HerdrError> {
        Self::workspace_close(self, workspace_id)
    }
    fn worktree_create(&self, repo: &Path, branch: &str) -> Result<WorktreeRef, HerdrError> {
        Self::worktree_create(self, repo, branch)
    }
    fn worktree_remove(&self, path: &Path) -> Result<(), HerdrError> {
        Self::worktree_remove(self, path)
    }
    fn pane_split(&self, workspace_id: &str, cwd: &Path) -> Result<PaneInfo, HerdrError> {
        Self::pane_split(self, workspace_id, cwd)
    }
    fn pane_split_pane(
        &self,
        target_pane_id: &str,
        direction: &str,
        ratio: Option<f64>,
    ) -> Result<PaneInfo, HerdrError> {
        Self::pane_split_pane(self, target_pane_id, direction, ratio)
    }
    fn pane_run(&self, pane_id: &str, input: &str) -> Result<(), HerdrError> {
        Self::pane_run(self, pane_id, input)
    }
    fn pane_read(&self, pane_id: &str) -> Result<String, HerdrError> {
        Self::pane_read(self, pane_id)
    }
    fn pane_rename(&self, pane_id: &str, title: &str) -> Result<(), HerdrError> {
        Self::pane_rename(self, pane_id, title)
    }
    fn pane_close(&self, pane_id: &str) -> Result<(), HerdrError> {
        Self::pane_close(self, pane_id)
    }
    fn pane_resize(
        &self,
        pane_id: &str,
        direction: &str,
        amount: Option<f64>,
    ) -> Result<(), HerdrError> {
        Self::pane_resize(self, pane_id, direction, amount)
    }
    fn agent_wait(
        &self,
        pane_id: &str,
        status: &str,
        timeout: Duration,
    ) -> Result<WaitOutcome, HerdrError> {
        Self::agent_wait(self, pane_id, status, timeout)
    }
    fn agent_list(&self) -> Result<Vec<AgentInfo>, HerdrError> {
        Self::agent_list(self)
    }
    fn pane_get(&self, pane_id: &str) -> Result<PaneInfo, HerdrError> {
        Self::pane_get(self, pane_id)
    }
    fn pane_list(&self, workspace_id: Option<&str>) -> Result<Vec<PaneInfo>, HerdrError> {
        Self::pane_list(self, workspace_id)
    }
    fn api_schema(&self) -> Result<String, HerdrError> {
        Self::api_schema(self)
    }
    fn pane_report_metadata(
        &self,
        pane_id: &str,
        update: &MetadataUpdate,
    ) -> Result<(), HerdrError> {
        Self::pane_report_metadata(self, pane_id, update)
    }
    fn notification_show(&self, title: &str, body: &str, sound: &str) -> Result<(), HerdrError> {
        Self::notification_show(self, title, body, sound)
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet, VecDeque};
    use std::sync::{Arc, Barrier, Condvar, Mutex, MutexGuard};

    pub(crate) struct BriefOrder {
        remaining: Mutex<VecDeque<String>>,
        changed: Condvar,
    }

    impl BriefOrder {
        pub fn new(panes: impl IntoIterator<Item = impl Into<String>>) -> Self {
            Self {
                remaining: Mutex::new(panes.into_iter().map(Into::into).collect()),
                changed: Condvar::new(),
            }
        }

        fn wait_turn(&self, pane_id: &str) {
            let mut remaining = self.remaining.lock().expect("brief order lock");
            while remaining.front().map(String::as_str) != Some(pane_id) {
                remaining = self.changed.wait(remaining).expect("brief order wait");
            }
            remaining.pop_front();
            self.changed.notify_all();
        }
    }

    #[derive(Default)]
    pub(crate) struct SyncCell<T>(Mutex<T>);

    impl<T: Copy> SyncCell<T> {
        pub fn get(&self) -> T {
            *self.0.lock().expect("fake cell lock")
        }

        pub fn set(&self, value: T) {
            *self.0.lock().expect("fake cell lock") = value;
        }
    }

    #[derive(Default)]
    pub(crate) struct SyncRefCell<T>(Mutex<T>);

    impl<T> SyncRefCell<T> {
        pub fn borrow(&self) -> MutexGuard<'_, T> {
            self.0.lock().expect("fake refcell lock")
        }

        pub fn borrow_mut(&self) -> MutexGuard<'_, T> {
            self.0.lock().expect("fake refcell lock")
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(crate) enum FakeCall {
        PaneGet(String),
        PaneRun(String, String),
        AgentWait(String, String),
        Notification(String, String, String),
        PaneSplitPane(String, String),
        PaneClose(String),
        PaneResize(String, String, Option<String>),
    }

    #[derive(Default)]
    pub(crate) struct FakeHerdr {
        pub calls: SyncRefCell<Vec<String>>,
        pub typed_calls: SyncRefCell<Vec<FakeCall>>,
        pub workspace_count: SyncCell<usize>,
        pub worktree_count: SyncCell<usize>,
        pub protocols_state_dir: SyncRefCell<Option<PathBuf>>,
        pub protocol_snapshots: SyncRefCell<Vec<BTreeMap<PathBuf, String>>>,
        pub fail_launch_pane: SyncRefCell<Option<String>>,
        pub fail_worktree_branch: SyncRefCell<Option<String>>,
        pub missing_panes: SyncRefCell<BTreeSet<String>>,
        pub launch_barrier: SyncRefCell<Option<Arc<Barrier>>>,
        pub brief_order: SyncRefCell<Option<Arc<BriefOrder>>>,
        pub fail_health: SyncCell<bool>,
        pub omit_agent: SyncCell<bool>,
        pub omit_agent_id: SyncCell<bool>,
        pub wait_timeouts: SyncCell<usize>,
        pub agent_id_delays: SyncCell<usize>,
        pub require_empty_submit: SyncCell<bool>,
        pub empty_submit_seen: SyncCell<bool>,
        pub pane: SyncRefCell<Option<PaneInfo>>,
        pub panes: SyncRefCell<Vec<PaneInfo>>,
        pub split_result: SyncRefCell<Option<PaneInfo>>,
        pub fail_split: SyncCell<bool>,
        pub fail_close: SyncCell<bool>,
        pub fail_resize: SyncCell<bool>,
        pub agents: SyncRefCell<Vec<AgentInfo>>,
        pub waits: SyncRefCell<VecDeque<WaitOutcome>>,
    }

    impl FakeHerdr {
        pub fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
        pub fn protocol_snapshots(&self) -> Vec<BTreeMap<PathBuf, String>> {
            self.protocol_snapshots.borrow().clone()
        }
        pub fn command_error() -> HerdrError {
            HerdrError::Command {
                argv: "fake pane run".to_owned(),
                status: Some(1),
                stderr: "injected failure".to_owned(),
            }
        }
        fn snapshot_protocols(&self) {
            let Some(state_dir) = self.protocols_state_dir.borrow().as_ref().cloned() else {
                return;
            };
            let entries = std::fs::read_dir(state_dir.join("runs"))
                .expect("read run state directory")
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
                .collect::<Vec<_>>();
            assert_eq!(
                entries.len(),
                1,
                "expected exactly one generated run directory"
            );
            let run_dir = &entries[0];
            let snapshots = std::fs::read_dir(run_dir.join("protocols"))
                .expect("read generated protocols")
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .map(|path| {
                    let contents = std::fs::read_to_string(&path).expect("read protocol");
                    (path, contents)
                })
                .collect::<BTreeMap<_, _>>();
            assert!(!snapshots.is_empty(), "protocols must predate agent launch");
            self.protocol_snapshots.borrow_mut().push(snapshots);
        }
    }

    impl HerdrApi for FakeHerdr {
        fn health_check(&self) -> Result<(), HerdrError> {
            self.calls.borrow_mut().push("health_check".to_owned());
            if self.fail_health.get() {
                Err(Self::command_error())
            } else {
                Ok(())
            }
        }
        fn workspace_create(&self, cwd: &Path, label: &str) -> Result<WorkspaceRef, HerdrError> {
            let number = self.workspace_count.get() + 1;
            self.workspace_count.set(number);
            self.calls
                .borrow_mut()
                .push(format!("workspace_create:{label}:{}", cwd.display()));
            Ok(WorkspaceRef {
                workspace_id: format!("workspace-{number}"),
                pane_id: format!("pane-{number}"),
            })
        }
        fn workspace_close(&self, workspace_id: &str) -> Result<(), HerdrError> {
            self.calls
                .borrow_mut()
                .push(format!("workspace_close:{workspace_id}"));
            Ok(())
        }
        fn worktree_create(&self, repo: &Path, branch: &str) -> Result<WorktreeRef, HerdrError> {
            self.calls
                .borrow_mut()
                .push(format!("worktree_create:{branch}:{}", repo.display()));
            if self.fail_worktree_branch.borrow().as_deref() == Some(branch) {
                return Err(Self::command_error());
            }
            let number = self.worktree_count.get() + 1;
            self.worktree_count.set(number);
            Ok(WorktreeRef {
                path: repo.join(format!("worktree-{number}")),
            })
        }
        fn worktree_remove(&self, path: &Path) -> Result<(), HerdrError> {
            self.calls
                .borrow_mut()
                .push(format!("worktree_remove:{}", path.display()));
            Ok(())
        }
        fn pane_run(&self, pane_id: &str, input: &str) -> Result<(), HerdrError> {
            if input.starts_with('\'') {
                self.snapshot_protocols();
                let barrier = self.launch_barrier.borrow().clone();
                if let Some(barrier) = barrier {
                    barrier.wait();
                }
            }
            if input.starts_with("Read your brief") {
                let order = self.brief_order.borrow().clone();
                if let Some(order) = order {
                    order.wait_turn(pane_id);
                }
            }
            self.calls
                .borrow_mut()
                .push(format!("pane_run:{pane_id}:{input}"));
            self.typed_calls
                .borrow_mut()
                .push(FakeCall::PaneRun(pane_id.to_owned(), input.to_owned()));
            if self.fail_launch_pane.borrow().as_deref() == Some(pane_id) && input.starts_with('\'')
            {
                return Err(Self::command_error());
            }
            if input.is_empty() {
                self.empty_submit_seen.set(true);
            }
            Ok(())
        }
        fn agent_wait(
            &self,
            pane_id: &str,
            status: &str,
            _: Duration,
        ) -> Result<WaitOutcome, HerdrError> {
            self.calls
                .borrow_mut()
                .push(format!("agent_wait:{pane_id}:{status}"));
            self.typed_calls
                .borrow_mut()
                .push(FakeCall::AgentWait(pane_id.to_owned(), status.to_owned()));
            if let Some(outcome) = self.waits.borrow_mut().pop_front() {
                return Ok(outcome);
            }
            if self.wait_timeouts.get() > 0 {
                self.wait_timeouts.set(self.wait_timeouts.get() - 1);
                return Ok(WaitOutcome::TimedOut);
            }
            if status == "working"
                && self.require_empty_submit.get()
                && !self.empty_submit_seen.get()
            {
                return Ok(WaitOutcome::TimedOut);
            }
            Ok(WaitOutcome::Reached)
        }
        fn agent_list(&self) -> Result<Vec<AgentInfo>, HerdrError> {
            self.calls.borrow_mut().push("agent_list".to_owned());
            Ok(self.agents.borrow().clone())
        }
        fn pane_get(&self, pane_id: &str) -> Result<PaneInfo, HerdrError> {
            self.calls.borrow_mut().push(format!("pane_get:{pane_id}"));
            self.typed_calls
                .borrow_mut()
                .push(FakeCall::PaneGet(pane_id.to_owned()));
            if self.missing_panes.borrow().contains(pane_id) {
                return Err(Self::command_error());
            }
            if let Some(pane) = self.pane.borrow().clone() {
                return Ok(pane);
            }
            let delayed = self.agent_id_delays.get() > 0;
            if delayed {
                self.agent_id_delays.set(self.agent_id_delays.get() - 1);
            }
            let agent_detected = !self.omit_agent.get();
            Ok(PaneInfo {
                pane_id: pane_id.to_owned(),
                workspace_id: pane_id.replace("pane", "workspace"),
                tab_id: None,
                agent: agent_detected.then(|| {
                    if pane_id == "pane-1" {
                        "claude".to_owned()
                    } else {
                        "codex".to_owned()
                    }
                }),
                agent_id: (agent_detected && !self.omit_agent_id.get() && !delayed)
                    .then(|| format!("agent-session-{pane_id}")),
                agent_session: (agent_detected && !self.omit_agent_id.get() && !delayed).then(
                    || AgentSession {
                        source: "herdr:test".to_owned(),
                        agent: "claude".to_owned(),
                        kind: "id".to_owned(),
                        value: format!("agent-session-{pane_id}"),
                    },
                ),
                agent_status: Some("idle".to_owned()),
                cwd: None,
            })
        }
        fn pane_list(&self, _workspace_id: Option<&str>) -> Result<Vec<PaneInfo>, HerdrError> {
            self.calls.borrow_mut().push("pane_list".to_owned());
            Ok(self.panes.borrow().clone())
        }
        fn pane_split_pane(
            &self,
            target_pane_id: &str,
            direction: &str,
            ratio: Option<f64>,
        ) -> Result<PaneInfo, HerdrError> {
            self.calls.borrow_mut().push(format!(
                "pane_split_pane:{target_pane_id}:{direction}:{ratio:?}"
            ));
            self.typed_calls.borrow_mut().push(FakeCall::PaneSplitPane(
                target_pane_id.to_owned(),
                direction.to_owned(),
            ));
            if self.fail_split.get() {
                return Err(Self::command_error());
            }
            match self.split_result.borrow().clone() {
                Some(pane) => Ok(pane),
                None => Ok(PaneInfo {
                    pane_id: format!("{target_pane_id}-split"),
                    workspace_id: "workspace-split".to_owned(),
                    tab_id: None,
                    agent: None,
                    agent_id: None,
                    agent_session: None,
                    agent_status: None,
                    cwd: None,
                }),
            }
        }
        fn pane_rename(&self, pane_id: &str, title: &str) -> Result<(), HerdrError> {
            self.calls
                .borrow_mut()
                .push(format!("pane_rename:{pane_id}:{title}"));
            Ok(())
        }
        fn pane_close(&self, pane_id: &str) -> Result<(), HerdrError> {
            self.calls
                .borrow_mut()
                .push(format!("pane_close:{pane_id}"));
            self.typed_calls
                .borrow_mut()
                .push(FakeCall::PaneClose(pane_id.to_owned()));
            if self.fail_close.get() {
                return Err(Self::command_error());
            }
            Ok(())
        }
        fn pane_resize(
            &self,
            pane_id: &str,
            direction: &str,
            amount: Option<f64>,
        ) -> Result<(), HerdrError> {
            self.calls
                .borrow_mut()
                .push(format!("pane_resize:{pane_id}:{direction}:{amount:?}"));
            self.typed_calls.borrow_mut().push(FakeCall::PaneResize(
                pane_id.to_owned(),
                direction.to_owned(),
                amount.map(|amount| amount.to_string()),
            ));
            if self.fail_resize.get() {
                return Err(Self::command_error());
            }
            Ok(())
        }
        fn api_schema(&self) -> Result<String, HerdrError> {
            self.calls.borrow_mut().push("api_schema".to_owned());
            Ok("{}".to_owned())
        }
        fn pane_report_metadata(
            &self,
            pane_id: &str,
            _: &MetadataUpdate,
        ) -> Result<(), HerdrError> {
            self.calls
                .borrow_mut()
                .push(format!("pane_report_metadata:{pane_id}"));
            Ok(())
        }
        fn notification_show(
            &self,
            title: &str,
            body: &str,
            sound: &str,
        ) -> Result<(), HerdrError> {
            self.calls
                .borrow_mut()
                .push(format!("notification:{title}:{body}:{sound}"));
            self.typed_calls.borrow_mut().push(FakeCall::Notification(
                title.to_owned(),
                body.to_owned(),
                sound.to_owned(),
            ));
            Ok(())
        }
    }
}

struct Args(Vec<OsString>);

fn args<const N: usize>(initial: [&str; N]) -> Args {
    Args(initial.into_iter().map(OsString::from).collect())
}

impl Args {
    fn with(mut self, value: impl AsRef<std::ffi::OsStr>) -> Self {
        self.0.push(value.as_ref().to_owned());
        self
    }

    fn finish(self) -> Vec<OsString> {
        self.0
    }
}

fn display_argv(binary: &Path, args: &[OsString]) -> String {
    std::iter::once(binary.as_os_str())
        .chain(args.iter().map(OsString::as_os_str))
        .map(|arg| format!("{:?}", arg.to_string_lossy()))
        .collect::<Vec<_>>()
        .join(" ")
}

fn first_line(bytes: &[u8]) -> Option<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_owned)
}

#[derive(Debug, Deserialize)]
struct Response<T> {
    result: T,
}

#[derive(Debug, Deserialize)]
struct WorkspaceCreateResult {
    #[serde(rename = "type")]
    kind: String,
    workspace: WorkspaceIdentity,
    root_pane: PaneIdentity,
}

#[derive(Debug, Deserialize)]
struct WorkspaceIdentity {
    workspace_id: String,
}

#[derive(Debug, Deserialize)]
struct PaneIdentity {
    pane_id: String,
}

#[derive(Debug, Deserialize)]
struct WorktreeCreateResult {
    #[serde(rename = "type")]
    kind: String,
    worktree: WorktreeRef,
}

#[derive(Debug, Deserialize)]
struct WorktreeListResult {
    #[serde(rename = "type")]
    kind: String,
    worktrees: Vec<WorktreeListEntry>,
}

#[derive(Debug, Deserialize)]
struct WorktreeListEntry {
    path: PathBuf,
    open_workspace_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WorktreeRemoveResult {
    #[serde(rename = "type")]
    kind: String,
    workspace_id: String,
    path: PathBuf,
    forced: bool,
}

#[derive(Debug, Deserialize)]
struct PaneInfoResult {
    #[serde(rename = "type")]
    kind: String,
    pane: PaneInfo,
}

#[derive(Debug, Deserialize)]
struct AgentInfoResult {
    #[serde(rename = "type")]
    kind: String,
    agent: AgentInfo,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum AgentWaitResponse {
    Immediate { result: AgentInfoResult },
    Future(AgentStatusEventEnvelope),
}

#[derive(Debug, Deserialize)]
struct AgentStatusEventEnvelope {
    event: String,
    data: AgentStatusEventData,
}

#[derive(Debug, Deserialize)]
struct AgentStatusEventData {
    #[serde(rename = "type")]
    kind: Option<String>,
    pane_id: String,
    agent_status: String,
}

#[derive(Debug, Deserialize)]
struct PaneListResult {
    #[serde(rename = "type")]
    kind: String,
    panes: Vec<PaneInfo>,
}

#[derive(Debug, Deserialize)]
struct AgentListResult {
    #[serde(rename = "type")]
    kind: String,
    agents: Vec<AgentInfo>,
}

#[derive(Debug, Deserialize)]
struct OkResult {
    #[serde(rename = "type")]
    kind: String,
}

fn parse_response<T: DeserializeOwned>(stdout: &str) -> Result<T, String> {
    serde_json::from_str::<Response<T>>(stdout)
        .map(|response| response.result)
        .map_err(|error| error.to_string())
}

fn expect_kind(actual: &str, expected: &str) -> Result<(), String> {
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "expected response type `{expected}`, received `{actual}`"
        ))
    }
}

fn parse_workspace_create(stdout: &str) -> Result<WorkspaceRef, String> {
    let result: WorkspaceCreateResult = parse_response(stdout)?;
    expect_kind(&result.kind, "workspace_created")?;
    Ok(WorkspaceRef {
        workspace_id: result.workspace.workspace_id,
        pane_id: result.root_pane.pane_id,
    })
}

fn parse_worktree_create(stdout: &str) -> Result<WorktreeRef, String> {
    let result: WorktreeCreateResult = parse_response(stdout)?;
    expect_kind(&result.kind, "worktree_created")?;
    Ok(result.worktree)
}

fn parse_worktree_workspace_id(stdout: &str, path: &Path) -> Result<String, String> {
    let result: WorktreeListResult = parse_response(stdout)?;
    expect_kind(&result.kind, "worktree_list")?;
    let worktree = result
        .worktrees
        .into_iter()
        .find(|worktree| worktree.path == path)
        .ok_or_else(|| format!("worktree `{}` was not returned by Herdr", path.display()))?;
    worktree.open_workspace_id.ok_or_else(|| {
        format!(
            "worktree `{}` has no opaque workspace ID and cannot be removed",
            path.display()
        )
    })
}

fn parse_worktree_remove(stdout: &str) -> Result<(), String> {
    let result: WorktreeRemoveResult = parse_response(stdout)?;
    expect_kind(&result.kind, "worktree_removed")?;
    let _protocol_fields = (result.workspace_id, result.path, result.forced);
    Ok(())
}

fn parse_pane_info(stdout: &str) -> Result<PaneInfo, String> {
    let result: PaneInfoResult = parse_response(stdout)?;
    expect_kind(&result.kind, "pane_info")?;
    Ok(result.pane)
}

fn parse_agent_wait(stdout: &str, pane_id: &str, status: &str) -> Result<(), String> {
    match serde_json::from_str::<AgentWaitResponse>(stdout).map_err(|error| error.to_string())? {
        AgentWaitResponse::Immediate { result } => {
            expect_kind(&result.kind, "agent_info")?;
            if result.agent.pane_id != pane_id {
                return Err(format!(
                    "wait matched pane `{}`, expected `{pane_id}`",
                    result.agent.pane_id
                ));
            }
            if result.agent.status.as_deref() != Some(status) {
                return Err(format!(
                    "wait matched status `{:?}`, expected `{status}`",
                    result.agent.status
                ));
            }
            Ok(())
        }
        AgentWaitResponse::Future(envelope) => {
            expect_kind(&envelope.event, "pane.agent_status_changed")?;
            if let Some(kind) = envelope.data.kind.as_deref() {
                expect_kind(kind, "pane_agent_status_changed")?;
            }
            if envelope.data.pane_id != pane_id {
                return Err(format!(
                    "wait matched pane `{}`, expected `{pane_id}`",
                    envelope.data.pane_id
                ));
            }
            if envelope.data.agent_status != status {
                return Err(format!(
                    "wait matched status `{}`, expected `{status}`",
                    envelope.data.agent_status
                ));
            }
            Ok(())
        }
    }
}

fn parse_pane_list(stdout: &str) -> Result<Vec<PaneInfo>, String> {
    let result: PaneListResult = parse_response(stdout)?;
    expect_kind(&result.kind, "pane_list")?;
    Ok(result.panes)
}

fn parse_agent_list(stdout: &str) -> Result<Vec<AgentInfo>, String> {
    let result: AgentListResult = parse_response(stdout)?;
    expect_kind(&result.kind, "agent_list")?;
    Ok(result.agents)
}

fn parse_ok(stdout: &str) -> Result<(), String> {
    let result: OkResult = parse_response(stdout)?;
    expect_kind(&result.kind, "ok")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    const LIVE_AGENT_LIST: &str = r#"{"id":"cli:agent:list","result":{"agents":[{"agent":"codex","agent_session":{"agent":"codex","kind":"id","source":"herdr:codex","value":"019f6268-cf99-7110-8c8f-e94817e05fad"},"agent_status":"working","cwd":"/home/caio/Projects/herdr-agent-team","focused":false,"foreground_cwd":"/home/caio/Projects/herdr-agent-team","pane_id":"wG:p6","revision":0,"tab_id":"wG:t2","terminal_id":"term_6569868fff53416","workspace_id":"wG"}],"type":"agent_list"}}"#;
    const LIVE_PANE_GET: &str = r#"{"id":"cli:pane:get","result":{"pane":{"agent":"codex","agent_session":{"agent":"codex","kind":"id","source":"herdr:codex","value":"019f6268-cf99-7110-8c8f-e94817e05fad"},"agent_status":"working","cwd":"/home/caio/Projects/herdr-agent-team","focused":true,"foreground_cwd":"/home/caio/Projects/herdr-agent-team","label":"codex-05-herdr","pane_id":"wG:p6","revision":0,"scroll":{"max_offset_from_bottom":141,"offset_from_bottom":0,"viewport_rows":29},"tab_id":"wG:t2","terminal_id":"term_6569868fff53416","workspace_id":"wG"},"type":"pane_info"}}"#;

    struct FakeBinary {
        _directory: PathBuf,
        path: PathBuf,
    }

    impl FakeBinary {
        fn returning(stdout: &str) -> Self {
            let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("test clock should follow Unix epoch")
                .as_nanos();
            let directory = std::env::temp_dir().join(format!(
                "herdr-client-tests-{}-{nanos}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&directory).expect("create fake Herdr directory");
            let response = directory.join("response");
            fs::write(&response, stdout).expect("write fake Herdr response");
            let path = directory.join("herdr");
            let staging_path = directory.join("herdr.staging");
            fs::write(
                &staging_path,
                format!("#!/bin/sh\ncat '{}'\n", response.display()),
            )
            .expect("write fake Herdr executable");
            let mut permissions = fs::metadata(&staging_path)
                .expect("stat fake Herdr executable")
                .permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&staging_path, permissions)
                .expect("make fake Herdr executable runnable");
            fs::rename(staging_path, &path).expect("atomically install fake Herdr executable");

            // Some filesystems briefly retain a writable reference to a newly
            // installed executable. Wait until execve accepts it before handing
            // the path to tests running in parallel.
            for attempt in 0..100 {
                match Command::new(&path).output() {
                    Ok(_) => break,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::ExecutableFileBusy
                            && attempt < 99 =>
                    {
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => panic!("probe fake Herdr executable: {error}"),
                }
            }
            Self {
                _directory: directory,
                path,
            }
        }

        fn client(&self) -> HerdrClient {
            HerdrClient {
                binary: self.path.clone(),
            }
        }
    }

    impl Drop for FakeBinary {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self._directory);
        }
    }

    #[test]
    fn parses_workspace_create_ids_from_response() {
        let fixture = r#"{"id":"cli:workspace:create","result":{"type":"workspace_created","workspace":{"workspace_id":"opaque-workspace"},"tab":{},"root_pane":{"pane_id":"opaque-pane"}}}"#;
        assert_eq!(
            parse_workspace_create(fixture).unwrap(),
            WorkspaceRef {
                workspace_id: "opaque-workspace".to_owned(),
                pane_id: "opaque-pane".to_owned(),
            }
        );
    }

    #[test]
    fn parses_worktree_responses_and_resolves_opaque_workspace_id() {
        let created = r#"{"id":"cli:worktree:create","result":{"type":"worktree_created","workspace":{},"tab":{},"root_pane":{},"worktree":{"path":"/tmp/repo-feature","branch":"feature","is_bare":false,"is_detached":false,"is_prunable":false,"is_linked_worktree":true,"label":"feature"}}}"#;
        assert_eq!(
            parse_worktree_create(created).unwrap(),
            WorktreeRef {
                path: PathBuf::from("/tmp/repo-feature")
            }
        );

        let listed = r#"{"id":"cli:worktree:list","result":{"type":"worktree_list","source":{"cwd":"/tmp/repo"},"worktrees":[{"path":"/tmp/repo-feature","branch":"feature","is_bare":false,"is_detached":false,"is_prunable":false,"is_linked_worktree":true,"label":"feature","open_workspace_id":"opaque-workspace"}]}}"#;
        assert_eq!(
            parse_worktree_workspace_id(listed, Path::new("/tmp/repo-feature")).unwrap(),
            "opaque-workspace"
        );

        let removed = r#"{"id":"cli:worktree:remove","result":{"type":"worktree_removed","workspace_id":"opaque-workspace","path":"/tmp/repo-feature","forced":false}}"#;
        parse_worktree_remove(removed).unwrap();
    }

    #[test]
    fn parses_live_protocol_16_pane_and_agent_samples() {
        let pane = parse_pane_info(LIVE_PANE_GET).unwrap();
        assert_eq!(pane.pane_id, "wG:p6");
        assert_eq!(pane.workspace_id, "wG");
        assert_eq!(pane.agent_status.as_deref(), Some("working"));
        assert_eq!(
            pane.agent_id.as_deref(),
            Some("019f6268-cf99-7110-8c8f-e94817e05fad")
        );
        assert_eq!(
            pane.agent_session,
            Some(AgentSession {
                source: "herdr:codex".to_owned(),
                agent: "codex".to_owned(),
                kind: "id".to_owned(),
                value: "019f6268-cf99-7110-8c8f-e94817e05fad".to_owned(),
            })
        );

        let agents = parse_agent_list(LIVE_AGENT_LIST).unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].pane_id, "wG:p6");
        assert_eq!(agents[0].status.as_deref(), Some("working"));
        assert_eq!(
            agents[0].agent_id.as_deref(),
            Some("019f6268-cf99-7110-8c8f-e94817e05fad")
        );
        assert_eq!(
            agents[0]
                .agent_session
                .as_ref()
                .map(|session| session.kind.as_str()),
            Some("id")
        );
    }

    #[test]
    fn parses_live_agent_wait_match_sample() {
        let fixture = r#"{"id":"cli:agent:wait:resolve","result":{"agent":{"agent":"codex","agent_status":"working","pane_id":"wG:p6","workspace_id":"wG"},"type":"agent_info"}}"#;
        parse_agent_wait(fixture, "wG:p6", "working").unwrap();
    }

    #[test]
    fn tolerates_missing_and_null_agent_sessions_without_constructing_ids() {
        let missing = r#"{"id":"cli:pane:get","result":{"pane":{"agent":"codex","agent_status":"working","pane_id":"opaque-pane","workspace_id":"opaque-workspace"},"type":"pane_info"}}"#;
        let pane = parse_pane_info(missing).unwrap();
        assert_eq!(pane.agent_id, None);

        let null = r#"{"id":"cli:agent:list","result":{"agents":[{"agent":"codex","agent_session":null,"agent_status":"working","pane_id":"opaque-pane","workspace_id":"opaque-workspace"}],"type":"agent_list"}}"#;
        let agents = parse_agent_list(null).unwrap();
        assert_eq!(agents[0].agent_id, None);
    }

    #[test]
    fn pane_run_accepts_zero_exit_with_empty_stdout() {
        let binary = FakeBinary::returning("");
        binary
            .client()
            .pane_run("opaque-pane", "launch command")
            .expect("empty stdout is the live successful pane-run response");
    }

    #[test]
    fn agent_wait_accepts_future_status_event_envelope() {
        let binary = FakeBinary::returning(
            r#"{"event":"pane.agent_status_changed","data":{"pane_id":"opaque-pane","workspace_id":"opaque-workspace","agent_status":"working","agent":"codex"}}"#,
        );
        assert_eq!(
            binary
                .client()
                .agent_wait("opaque-pane", "working", Duration::from_secs(1))
                .expect("future status event is a successful wait match"),
            WaitOutcome::Reached
        );
    }

    #[test]
    fn future_status_event_must_match_requested_pane_and_status() {
        let wrong_pane = r#"{"event":"pane.agent_status_changed","data":{"pane_id":"other-pane","agent_status":"working"}}"#;
        assert!(parse_agent_wait(wrong_pane, "opaque-pane", "working")
            .unwrap_err()
            .contains("expected `opaque-pane`"));

        let wrong_status = r#"{"event":"pane.agent_status_changed","data":{"pane_id":"opaque-pane","agent_status":"idle"}}"#;
        assert!(parse_agent_wait(wrong_status, "opaque-pane", "working")
            .unwrap_err()
            .contains("expected `working`"));
    }

    #[test]
    fn pane_rename_accepts_live_pane_info_response() {
        let binary = FakeBinary::returning(
            r#"{"id":"cli:pane:rename","result":{"pane":{"agent":null,"agent_session":null,"agent_status":"unknown","cwd":"/tmp","focused":false,"pane_id":"opaque-pane","revision":1,"tab_id":"opaque-tab","terminal_id":"opaque-terminal","workspace_id":"opaque-workspace"},"type":"pane_info"}}"#,
        );
        binary
            .client()
            .pane_rename("opaque-pane", "worker")
            .expect("pane_info is the live successful rename response");
    }

    #[test]
    fn maps_agent_wait_exit_one_to_timeout() {
        let client = HerdrClient {
            binary: PathBuf::from("/bin/false"),
        };
        assert_eq!(
            client
                .agent_wait("opaque-pane", "idle", Duration::from_millis(1))
                .unwrap(),
            WaitOutcome::TimedOut
        );
    }

    #[test]
    fn rejects_an_unexpected_response_type() {
        let error = parse_ok(r#"{"id":"test","result":{"type":"pane_info"}}"#).unwrap_err();
        assert!(error.contains("expected response type `ok`"));
    }

    #[test]
    fn command_errors_include_argv_and_first_stderr_line() {
        let client = HerdrClient {
            binary: PathBuf::from("/bin/sh"),
        };
        let error = client
            .invoke(&[
                OsString::from("-c"),
                OsString::from("printf 'first line\\nsecond line\\n' >&2; exit 7"),
            ])
            .unwrap_err();
        match error {
            HerdrError::Command { argv, stderr, .. } => {
                assert!(argv.contains("/bin/sh"));
                assert_eq!(stderr, "first line");
            }
            other => panic!("expected command error, received {other:?}"),
        }
    }
}
