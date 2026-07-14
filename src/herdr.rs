//! Typed wrapper for Herdr CLI operations required by `docs/spec.md` sections 4, 6, and 9.

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum HerdrError {
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
    #[serde(rename = "agent_status")]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitOutcome {
    Reached,
    TimedOut,
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

    pub fn pane_run(&self, pane_id: &str, input: &str) -> Result<(), HerdrError> {
        let args = args(["pane", "run"]).with(pane_id).with(input).finish();
        let stdout = self.invoke(&args)?;
        parse_ok(&stdout).map_err(|message| self.invalid_response(&args, message))
    }

    pub fn pane_read(&self, pane_id: &str) -> Result<String, HerdrError> {
        let args = args(["pane", "read"]).with(pane_id).finish();
        self.invoke(&args)
    }

    pub fn pane_rename(&self, pane_id: &str, title: &str) -> Result<(), HerdrError> {
        let args = args(["pane", "rename"]).with(pane_id).with(title).finish();
        let stdout = self.invoke(&args)?;
        parse_ok(&stdout).map_err(|message| self.invalid_response(&args, message))
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
                parse_agent_info(&stdout)
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

fn parse_agent_info(stdout: &str) -> Result<AgentInfo, String> {
    let result: AgentInfoResult = parse_response(stdout)?;
    expect_kind(&result.kind, "agent_info")?;
    Ok(result.agent)
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

    const LIVE_AGENT_LIST: &str = r#"{"id":"cli:agent:list","result":{"agents":[{"agent":"codex","agent_session":{"agent":"codex","kind":"id","source":"herdr:codex","value":"019f6268-cf99-7110-8c8f-e94817e05fad"},"agent_status":"working","cwd":"/home/caio/Projects/herdr-agent-team","focused":false,"foreground_cwd":"/home/caio/Projects/herdr-agent-team","pane_id":"wG:p6","revision":0,"tab_id":"wG:t2","terminal_id":"term_6569868fff53416","workspace_id":"wG"}],"type":"agent_list"}}"#;
    const LIVE_PANE_GET: &str = r#"{"id":"cli:pane:get","result":{"pane":{"agent":"codex","agent_session":{"agent":"codex","kind":"id","source":"herdr:codex","value":"019f6268-cf99-7110-8c8f-e94817e05fad"},"agent_status":"working","cwd":"/home/caio/Projects/herdr-agent-team","focused":true,"foreground_cwd":"/home/caio/Projects/herdr-agent-team","label":"codex-05-herdr","pane_id":"wG:p6","revision":0,"scroll":{"max_offset_from_bottom":141,"offset_from_bottom":0,"viewport_rows":29},"tab_id":"wG:t2","terminal_id":"term_6569868fff53416","workspace_id":"wG"},"type":"pane_info"}}"#;

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

        let agents = parse_agent_list(LIVE_AGENT_LIST).unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].pane_id, "wG:p6");
        assert_eq!(agents[0].status.as_deref(), Some("working"));
    }

    #[test]
    fn parses_live_agent_wait_match_sample() {
        let fixture = r#"{"id":"cli:agent:wait:resolve","result":{"agent":{"agent":"codex","agent_status":"working","pane_id":"wG:p6","workspace_id":"wG"},"type":"agent_info"}}"#;
        let agent = parse_agent_info(fixture).unwrap();
        assert_eq!(agent.pane_id, "wG:p6");
        assert_eq!(agent.status.as_deref(), Some("working"));
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
