//! Team status and teardown commands from `docs/spec.md` sections 6 and 12.

use crate::herdr::{AgentInfo, HerdrApi, HerdrClient, HerdrError};
use crate::run::{load_run, mark_ended, RunBoard, RunError};
use crate::types::{RunLifecycle, WorkerLifecycle};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTimeError, UNIX_EPOCH};
use thiserror::Error;

const STATUS_USAGE: &str = "usage: herdr-agent-team status <run-dir> [--json]";
const KILL_USAGE: &str =
    "usage: herdr-agent-team kill <run-dir> [--remove-worktrees] [--worker <name>]";

#[derive(Debug, Error)]
pub enum StatusKillError {
    #[error("{0}")]
    Usage(String),

    #[error(transparent)]
    Run(#[from] RunError),

    #[error(transparent)]
    Herdr(#[from] HerdrError),

    #[error("failed to read report metadata at {path}: {source}")]
    ReportMetadata {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("report timestamp is before the Unix epoch at {path}: {source}")]
    ReportClock {
        path: PathBuf,
        #[source]
        source: SystemTimeError,
    },

    #[error("failed to serialize status JSON: {0}")]
    Json(#[from] serde_json::Error),

    #[error("failed to inspect worktree {path} with git: {source}")]
    GitSpawn {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("git status failed for worktree {path}: {stderr}")]
    GitStatus { path: PathBuf, stderr: String },

    #[error("refusing to remove dirty worktree(s): {paths:?}")]
    DirtyWorktrees { paths: Vec<PathBuf> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct StatusSnapshot {
    team: String,
    lifecycle: &'static str,
    workers: Vec<WorkerStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct WorkerStatus {
    worker: String,
    agent: String,
    status: String,
    last_report_time_unix_secs: Option<u64>,
}

pub fn status_command(args: &[String]) -> Result<(), StatusKillError> {
    let (run_dir, json) = parse_command_args(args, "--json", STATUS_USAGE)?;
    let rendered = status_run(&run_dir, json, &HerdrClient::from_env())?;
    print!("{rendered}");
    Ok(())
}

pub fn kill_command(args: &[String]) -> Result<(), StatusKillError> {
    let (run_dir, remove_worktrees, worker) = parse_kill_args(args)?;
    if let Some(worker) = worker {
        kill_worker(
            &run_dir,
            &worker,
            remove_worktrees,
            &HerdrClient::from_env(),
        )
    } else {
        kill_run(&run_dir, remove_worktrees, &HerdrClient::from_env())
    }
}

pub fn status_run<H: HerdrApi>(
    run_dir: &Path,
    json: bool,
    herdr: &H,
) -> Result<String, StatusKillError> {
    status_run_with_source(run_dir, json, herdr)
}

pub fn kill_run<H: HerdrApi>(
    run_dir: &Path,
    remove_worktrees: bool,
    herdr: &H,
) -> Result<(), StatusKillError> {
    let backend = SystemTeardown { herdr };
    kill_run_with_backend(run_dir, remove_worktrees, &backend)
}

pub fn kill_worker<H: HerdrApi>(
    run_dir: &Path,
    worker_name: &str,
    remove_worktrees: bool,
    herdr: &H,
) -> Result<(), StatusKillError> {
    kill_worker_with_backend(
        run_dir,
        worker_name,
        remove_worktrees,
        &SystemTeardown { herdr },
    )
}

fn parse_kill_args(args: &[String]) -> Result<(PathBuf, bool, Option<String>), StatusKillError> {
    let mut run_dir = None;
    let mut remove_worktrees = false;
    let mut worker = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--remove-worktrees" if !remove_worktrees => remove_worktrees = true,
            "--remove-worktrees" => {
                return Err(StatusKillError::Usage(format!(
                    "duplicate option --remove-worktrees; {KILL_USAGE}"
                )))
            }
            "--worker" => {
                index += 1;
                let value = args.get(index).ok_or_else(|| {
                    StatusKillError::Usage(format!("--worker requires a name; {KILL_USAGE}"))
                })?;
                if worker.replace(value.clone()).is_some() {
                    return Err(StatusKillError::Usage(format!(
                        "duplicate option --worker; {KILL_USAGE}"
                    )));
                }
            }
            value if value.starts_with('-') => {
                return Err(StatusKillError::Usage(format!(
                    "unknown option {value}; {KILL_USAGE}"
                )))
            }
            value if run_dir.replace(PathBuf::from(value)).is_some() => {
                return Err(StatusKillError::Usage(format!(
                    "expected one run directory; {KILL_USAGE}"
                )))
            }
            _ => {}
        }
        index += 1;
    }
    Ok((
        run_dir.ok_or_else(|| StatusKillError::Usage(KILL_USAGE.to_owned()))?,
        remove_worktrees,
        worker,
    ))
}

fn parse_command_args(
    args: &[String],
    allowed_flag: &str,
    usage: &str,
) -> Result<(PathBuf, bool), StatusKillError> {
    let mut run_dir = None;
    let mut flag = false;

    for arg in args {
        if arg == allowed_flag {
            if flag {
                return Err(StatusKillError::Usage(format!(
                    "duplicate option {allowed_flag}; {usage}"
                )));
            }
            flag = true;
        } else if arg.starts_with('-') {
            return Err(StatusKillError::Usage(format!(
                "unknown option {arg}; {usage}"
            )));
        } else if run_dir.replace(PathBuf::from(arg)).is_some() {
            return Err(StatusKillError::Usage(format!(
                "expected one run directory; {usage}"
            )));
        }
    }

    run_dir
        .map(|run_dir| (run_dir, flag))
        .ok_or_else(|| StatusKillError::Usage(usage.to_owned()))
}

fn status_run_with_source<H: HerdrApi>(
    run_dir: &Path,
    json: bool,
    source: &H,
) -> Result<String, StatusKillError> {
    let run = load_run(run_dir)?;
    let agents = source.agent_list()?;
    let report_times = read_report_times(&run)?;
    let snapshot = build_status_snapshot(&run, &agents, &report_times);
    if json {
        render_status_json(&snapshot)
    } else {
        Ok(render_status_table(&snapshot))
    }
}

fn read_report_times(run: &RunBoard) -> Result<BTreeMap<String, Option<u64>>, StatusKillError> {
    run.state
        .spec
        .workers
        .iter()
        .map(|worker| {
            let path = run.dir.join("inbox").join(format!("{}.md", worker.name));
            let timestamp = match fs::metadata(&path) {
                Ok(metadata) => {
                    let modified =
                        metadata
                            .modified()
                            .map_err(|source| StatusKillError::ReportMetadata {
                                path: path.clone(),
                                source,
                            })?;
                    Some(
                        modified
                            .duration_since(UNIX_EPOCH)
                            .map_err(|source| StatusKillError::ReportClock {
                                path: path.clone(),
                                source,
                            })?
                            .as_secs(),
                    )
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(source) => {
                    return Err(StatusKillError::ReportMetadata {
                        path: path.clone(),
                        source,
                    });
                }
            };
            Ok((worker.name.clone(), timestamp))
        })
        .collect()
}

fn build_status_snapshot(
    run: &RunBoard,
    agents: &[AgentInfo],
    report_times: &BTreeMap<String, Option<u64>>,
) -> StatusSnapshot {
    let workers = run
        .state
        .spec
        .workers
        .iter()
        .map(|worker| {
            let pane_id = run
                .state
                .workers
                .get(&worker.name)
                .and_then(|state| state.pane_id.as_deref());
            let status = match pane_id {
                Some(pane_id) => agents
                    .iter()
                    .find(|agent| agent.pane_id == pane_id)
                    .and_then(|agent| agent.status.as_deref())
                    .unwrap_or_else(|| {
                        if agents.iter().any(|agent| agent.pane_id == pane_id) {
                            "unknown"
                        } else {
                            "gone"
                        }
                    }),
                None => "unknown",
            };

            WorkerStatus {
                worker: worker.name.clone(),
                agent: worker.agent.clone(),
                status: status.to_owned(),
                last_report_time_unix_secs: report_times.get(&worker.name).copied().flatten(),
            }
        })
        .collect();

    StatusSnapshot {
        team: run.state.spec.name.clone(),
        lifecycle: lifecycle_name(run.state.lifecycle),
        workers,
    }
}

fn lifecycle_name(lifecycle: RunLifecycle) -> &'static str {
    match lifecycle {
        RunLifecycle::Active => "active",
        RunLifecycle::Ended => "ended",
    }
}

fn render_status_table(snapshot: &StatusSnapshot) -> String {
    let mut output = format!(
        "TEAM\t{}\nLIFECYCLE\t{}\nWORKER\tAGENT\tSTATUS\tLAST_REPORT_UNIX_SECS\n",
        snapshot.team, snapshot.lifecycle
    );
    for worker in &snapshot.workers {
        let report_time = worker
            .last_report_time_unix_secs
            .map(|timestamp| timestamp.to_string())
            .unwrap_or_else(|| "-".to_owned());
        output.push_str(&format!(
            "{}\t{}\t{}\t{}\n",
            worker.worker, worker.agent, worker.status, report_time
        ));
    }
    output
}

fn render_status_json(snapshot: &StatusSnapshot) -> Result<String, StatusKillError> {
    let mut output = serde_json::to_string_pretty(snapshot)?;
    output.push('\n');
    Ok(output)
}

trait TeardownBackend {
    fn workspace_close(&self, workspace_id: &str) -> Result<(), StatusKillError>;
    fn pane_run(&self, pane_id: &str, input: &str) -> Result<(), StatusKillError>;
    fn worktree_is_dirty(&self, path: &Path) -> Result<bool, StatusKillError>;
    fn worktree_remove(&self, path: &Path) -> Result<(), StatusKillError>;
}

struct SystemTeardown<'a, H> {
    herdr: &'a H,
}

impl<H: HerdrApi> TeardownBackend for SystemTeardown<'_, H> {
    fn workspace_close(&self, workspace_id: &str) -> Result<(), StatusKillError> {
        self.herdr.workspace_close(workspace_id)?;
        Ok(())
    }

    fn pane_run(&self, pane_id: &str, input: &str) -> Result<(), StatusKillError> {
        self.herdr.pane_run(pane_id, input)?;
        Ok(())
    }

    fn worktree_is_dirty(&self, path: &Path) -> Result<bool, StatusKillError> {
        worktree_is_dirty(path)
    }

    fn worktree_remove(&self, path: &Path) -> Result<(), StatusKillError> {
        self.herdr.worktree_remove(path)?;
        Ok(())
    }
}

fn kill_run_with_backend(
    run_dir: &Path,
    remove_worktrees: bool,
    backend: &impl TeardownBackend,
) -> Result<(), StatusKillError> {
    let mut run = load_run(run_dir)?;
    if run.state.lifecycle == RunLifecycle::Ended {
        if end_worker_lifecycles(&mut run) {
            mark_ended(&mut run)?;
        }
        return Ok(());
    }

    release_adopted_workers(&mut run, backend);

    let workspace_ids = run
        .state
        .workers
        .values()
        .filter(|worker| !worker.adopted)
        .filter_map(|worker| worker.workspace_id.as_deref())
        .collect::<BTreeSet<_>>();
    for workspace_id in workspace_ids {
        if let Err(error) = backend.workspace_close(workspace_id) {
            log_teardown_note("workspace", workspace_id, &error);
        }
    }

    let mut dirty_paths = Vec::new();
    if remove_worktrees {
        let worktree_paths = run
            .state
            .workers
            .values()
            .filter(|worker| !worker.adopted)
            .filter_map(|worker| worker.worktree_path.as_deref())
            .collect::<BTreeSet<_>>();
        for path in worktree_paths {
            match backend.worktree_is_dirty(path) {
                Ok(true) => dirty_paths.push(path.to_path_buf()),
                Ok(false) => {
                    if let Err(error) = backend.worktree_remove(path) {
                        log_teardown_note("worktree", &path.display().to_string(), &error);
                    }
                }
                Err(error) => {
                    log_teardown_note("worktree", &path.display().to_string(), &error);
                }
            }
        }
    }

    end_worker_lifecycles(&mut run);
    mark_ended(&mut run)?;
    if dirty_paths.is_empty() {
        Ok(())
    } else {
        Err(StatusKillError::DirtyWorktrees { paths: dirty_paths })
    }
}

fn kill_worker_with_backend(
    run_dir: &Path,
    worker_name: &str,
    remove_worktrees: bool,
    backend: &impl TeardownBackend,
) -> Result<(), StatusKillError> {
    let mut run = load_run(run_dir)?;
    if run.state.lifecycle == RunLifecycle::Ended {
        return Ok(());
    }
    let worker = run.state.workers.get(worker_name).cloned().ok_or_else(|| {
        StatusKillError::Usage(format!(
            "unknown worker '{worker_name}' in {}; {KILL_USAGE}",
            run_dir.display()
        ))
    })?;
    if matches!(
        worker.lifecycle,
        WorkerLifecycle::Ended | WorkerLifecycle::Released | WorkerLifecycle::Failed
    ) {
        return Ok(());
    }

    if worker.adopted {
        if let Some(pane_id) = worker.pane_id.as_deref() {
            if let Err(error) = backend.pane_run(pane_id, &release_notice(&run.state.spec.name)) {
                log_teardown_note("adopted pane", pane_id, &error);
            }
        }
    } else if let Some(workspace_id) = worker.workspace_id.as_deref() {
        if let Err(error) = backend.workspace_close(workspace_id) {
            log_teardown_note("workspace", workspace_id, &error);
        }
    }

    let mut dirty_paths = Vec::new();
    if remove_worktrees && !worker.adopted {
        if let Some(path) = worker.worktree_path.as_deref() {
            match backend.worktree_is_dirty(path) {
                Ok(true) => dirty_paths.push(path.to_path_buf()),
                Ok(false) => {
                    if let Err(error) = backend.worktree_remove(path) {
                        log_teardown_note("worktree", &path.display().to_string(), &error);
                    }
                }
                Err(error) => log_teardown_note("worktree", &path.display().to_string(), &error),
            }
        }
    }
    let terminal = if worker.adopted {
        WorkerLifecycle::Released
    } else {
        WorkerLifecycle::Ended
    };
    run.state
        .workers
        .get_mut(worker_name)
        .expect("worker was loaded above")
        .lifecycle = terminal;
    if run.state.workers.values().all(|state| {
        matches!(
            state.lifecycle,
            WorkerLifecycle::Ended | WorkerLifecycle::Released | WorkerLifecycle::Failed
        )
    }) {
        mark_ended(&mut run)?;
    } else {
        crate::run::save_run(&run)?;
    }
    if dirty_paths.is_empty() {
        Ok(())
    } else {
        Err(StatusKillError::DirtyWorktrees { paths: dirty_paths })
    }
}

fn release_adopted_workers(run: &mut RunBoard, backend: &impl TeardownBackend) {
    let pending = run
        .state
        .workers
        .iter()
        .filter(|(_, worker)| {
            worker.adopted
                && !matches!(
                    worker.lifecycle,
                    WorkerLifecycle::Failed | WorkerLifecycle::Released
                )
        })
        .map(|(name, worker)| (name.clone(), worker.pane_id.clone()))
        .collect::<Vec<_>>();
    let notice = release_notice(&run.state.spec.name);

    for (name, pane_id) in pending {
        match pane_id {
            Some(pane_id) => {
                if let Err(error) = backend.pane_run(&pane_id, &notice) {
                    log_teardown_note("adopted pane", &pane_id, &error);
                }
            }
            None => eprintln!(
                "note: team kill found adopted worker '{name}' without a pane; releasing it"
            ),
        }
        run.state
            .workers
            .get_mut(&name)
            .expect("release candidates come from the same run state")
            .lifecycle = WorkerLifecycle::Released;
    }
}

fn release_notice(team_name: &str) -> String {
    format!("team {team_name} ended; report protocol no longer applies")
}

fn log_teardown_note(resource: &str, identifier: &str, error: &StatusKillError) {
    eprintln!("note: team kill could not close {resource} '{identifier}' ({error}); continuing");
}

fn end_worker_lifecycles(run: &mut RunBoard) -> bool {
    let mut changed = false;
    for worker in run.state.workers.values_mut() {
        if worker.lifecycle == WorkerLifecycle::Failed {
            continue;
        }
        let terminal = if worker.adopted {
            WorkerLifecycle::Released
        } else {
            WorkerLifecycle::Ended
        };
        if worker.lifecycle != terminal {
            worker.lifecycle = terminal;
            changed = true;
        }
    }
    changed
}

fn worktree_is_dirty(path: &Path) -> Result<bool, StatusKillError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["status", "--porcelain", "--untracked-files=normal"])
        .output()
        .map_err(|source| StatusKillError::GitSpawn {
            path: path.to_path_buf(),
            source,
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr)
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or("git status failed without diagnostic output")
            .to_owned();
        return Err(StatusKillError::GitStatus {
            path: path.to_path_buf(),
            stderr,
        });
    }

    Ok(!output.stdout.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::herdr::test_support::FakeHerdr;
    use crate::run::{create_run, load_run, match_pane};
    use crate::types::{
        GodSpec, RunState, TeamSpec, Topology, WorkerLifecycle, WorkerRunState, WorkerSpec,
    };
    use std::cell::RefCell;
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("test clock should be after Unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "herdr-status-kill-tests-{}-{nanos}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("create test directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn run_state(worktree_root: &Path) -> RunState {
        let workers = vec![
            WorkerSpec {
                name: "builder".to_owned(),
                agent: "codex".to_owned(),
                role: "builder".to_owned(),
                task: None,
                worktree: true,
                branch: Some("feat/builder".to_owned()),
                brief: PathBuf::from("briefs/builder.md"),
            },
            WorkerSpec {
                name: "reviewer".to_owned(),
                agent: "claude".to_owned(),
                role: "reviewer".to_owned(),
                task: None,
                worktree: true,
                branch: Some("feat/reviewer".to_owned()),
                brief: PathBuf::from("briefs/reviewer.md"),
            },
        ];
        let runtime = |workspace: &str, pane: &str, worktree: &str| WorkerRunState {
            task: None,
            workspace_id: Some(workspace.to_owned()),
            pane_id: Some(pane.to_owned()),
            agent_id: Some(format!("agent-{pane}")),
            agent_session: None,
            worktree_path: Some(worktree_root.join(worktree)),
            adopted: false,
            launch_checkpoint: Default::default(),
            lifecycle: WorkerLifecycle::Running,
        };

        RunState {
            spec: TeamSpec {
                name: "status-wave".to_owned(),
                topology: Topology::Star,
                cwd: PathBuf::from("/repo"),
                setup: Vec::new(),
                god: GodSpec::default(),
                workers,
            },
            god_pane_id: "god-pane".to_owned(),
            herdr_session: Default::default(),
            workers: BTreeMap::from([
                (
                    "builder".to_owned(),
                    runtime("workspace-builder", "pane-builder", "builder"),
                ),
                (
                    "reviewer".to_owned(),
                    runtime("workspace-reviewer", "pane-reviewer", "reviewer"),
                ),
            ]),
            lifecycle: RunLifecycle::Active,
        }
    }

    #[test]
    fn status_reports_live_gone_and_report_mtime_in_spec_order() {
        let temp = TempDir::new();
        let run = create_run(temp.path(), run_state(temp.path())).expect("create run");
        fs::write(run.dir.join("inbox/builder.md"), "done").expect("write report");
        let source = FakeHerdr::default();
        *source.agents.borrow_mut() = vec![AgentInfo {
            pane_id: "pane-builder".to_owned(),
            workspace_id: "workspace-builder".to_owned(),
            agent: Some("codex".to_owned()),
            agent_id: None,
            agent_session: None,
            status: Some("working".to_owned()),
        }];

        let table = status_run_with_source(&run.dir, false, &source).expect("render table");
        let rows = table.lines().collect::<Vec<_>>();

        assert_eq!(rows[0], "TEAM\tstatus-wave");
        assert_eq!(rows[1], "LIFECYCLE\tactive");
        assert!(rows[3].starts_with("builder\tcodex\tworking\t"));
        assert!(!rows[3].ends_with("\t-"));
        assert_eq!(rows[4], "reviewer\tclaude\tgone\t-");
    }

    #[test]
    fn status_json_is_stable_and_machine_readable() {
        let temp = TempDir::new();
        let run = RunBoard {
            dir: temp.path().to_path_buf(),
            state: run_state(temp.path()),
        };
        let report_times = BTreeMap::from([
            ("builder".to_owned(), Some(1_721_234_567)),
            ("reviewer".to_owned(), None),
        ]);
        let agents = vec![AgentInfo {
            pane_id: "pane-builder".to_owned(),
            workspace_id: "workspace-builder".to_owned(),
            agent: Some("codex".to_owned()),
            agent_id: None,
            agent_session: None,
            status: Some("idle".to_owned()),
        }];

        let snapshot = build_status_snapshot(&run, &agents, &report_times);

        assert_eq!(
            render_status_json(&snapshot).unwrap(),
            concat!(
                "{\n",
                "  \"team\": \"status-wave\",\n",
                "  \"lifecycle\": \"active\",\n",
                "  \"workers\": [\n",
                "    {\n",
                "      \"worker\": \"builder\",\n",
                "      \"agent\": \"codex\",\n",
                "      \"status\": \"idle\",\n",
                "      \"last_report_time_unix_secs\": 1721234567\n",
                "    },\n",
                "    {\n",
                "      \"worker\": \"reviewer\",\n",
                "      \"agent\": \"claude\",\n",
                "      \"status\": \"gone\",\n",
                "      \"last_report_time_unix_secs\": null\n",
                "    }\n",
                "  ]\n",
                "}\n"
            )
        );
    }

    #[derive(Default)]
    struct FakeTeardown {
        dirty: BTreeSet<PathBuf>,
        unavailable_panes: BTreeSet<String>,
        unavailable_workspaces: BTreeSet<String>,
        closed: RefCell<Vec<String>>,
        notices: RefCell<Vec<(String, String)>>,
        inspected: RefCell<Vec<PathBuf>>,
        removed: RefCell<Vec<PathBuf>>,
    }

    impl TeardownBackend for FakeTeardown {
        fn workspace_close(&self, workspace_id: &str) -> Result<(), StatusKillError> {
            self.closed.borrow_mut().push(workspace_id.to_owned());
            if self.unavailable_workspaces.contains(workspace_id) {
                return Err(StatusKillError::Usage("workspace_not_found".to_owned()));
            }
            Ok(())
        }

        fn pane_run(&self, pane_id: &str, input: &str) -> Result<(), StatusKillError> {
            self.notices
                .borrow_mut()
                .push((pane_id.to_owned(), input.to_owned()));
            if self.unavailable_panes.contains(pane_id) {
                return Err(StatusKillError::Usage("pane_not_found".to_owned()));
            }
            Ok(())
        }

        fn worktree_is_dirty(&self, path: &Path) -> Result<bool, StatusKillError> {
            self.inspected.borrow_mut().push(path.to_path_buf());
            Ok(self.dirty.contains(path))
        }

        fn worktree_remove(&self, path: &Path) -> Result<(), StatusKillError> {
            self.removed.borrow_mut().push(path.to_path_buf());
            Ok(())
        }
    }

    #[test]
    fn kill_closes_only_recorded_workspaces_and_ends_the_run() {
        let temp = TempDir::new();
        let run = create_run(temp.path(), run_state(temp.path())).expect("create run");
        let backend = FakeTeardown::default();

        kill_run_with_backend(&run.dir, false, &backend).expect("kill run");

        assert_eq!(
            backend.closed.into_inner(),
            vec!["workspace-builder", "workspace-reviewer"]
        );
        assert!(backend.removed.into_inner().is_empty());
        let ended = load_run(&run.dir).expect("load ended run");
        assert_eq!(ended.state.lifecycle, RunLifecycle::Ended);
        assert_eq!(
            ended
                .state
                .workers
                .values()
                .map(|worker| worker.lifecycle)
                .collect::<Vec<_>>(),
            [WorkerLifecycle::Ended, WorkerLifecycle::Ended]
        );
        assert!(match_pane(temp.path(), "pane-builder")
            .expect("match pane")
            .is_none());
    }

    #[test]
    fn kill_releases_adopted_worker_once_and_closes_only_owned_workspace() {
        let temp = TempDir::new();
        let mut state = run_state(temp.path());
        let adopted = state.workers.get_mut("reviewer").unwrap();
        adopted.workspace_id = Some("workspace-borrowed".to_owned());
        adopted.pane_id = Some("pane-borrowed".to_owned());
        adopted.worktree_path = None;
        adopted.adopted = true;
        let run = create_run(temp.path(), state).expect("create mixed-ownership run");
        let backend = FakeTeardown::default();

        kill_run_with_backend(&run.dir, true, &backend).expect("kill mixed-ownership run");

        assert_eq!(
            backend.notices.borrow().as_slice(),
            [(
                "pane-borrowed".to_owned(),
                "team status-wave ended; report protocol no longer applies".to_owned(),
            )]
        );
        assert_eq!(
            backend.closed.borrow().as_slice(),
            ["workspace-builder".to_owned()]
        );
        assert_eq!(
            backend.inspected.borrow().as_slice(),
            [temp.path().join("builder")]
        );
        assert_eq!(
            backend.removed.borrow().as_slice(),
            [temp.path().join("builder")]
        );
        let ended = load_run(&run.dir).expect("load released run");
        assert_eq!(ended.state.lifecycle, RunLifecycle::Ended);
        assert_eq!(
            ended.state.workers["builder"].lifecycle,
            WorkerLifecycle::Ended
        );
        assert_eq!(
            ended.state.workers["reviewer"].lifecycle,
            WorkerLifecycle::Released
        );

        kill_run_with_backend(&run.dir, true, &backend).expect("repeat kill is idempotent");
        assert_eq!(backend.notices.borrow().len(), 1);
        assert_eq!(backend.closed.borrow().len(), 1);
    }

    #[test]
    fn kill_preserves_failed_workers_while_ending_every_other_lifecycle() {
        let temp = TempDir::new();
        let mut state = run_state(temp.path());
        state
            .workers
            .get_mut("reviewer")
            .expect("reviewer runtime")
            .lifecycle = WorkerLifecycle::Failed;
        let run = create_run(temp.path(), state).expect("create run");
        let backend = FakeTeardown::default();

        kill_run_with_backend(&run.dir, false, &backend).expect("kill run");

        let ended = load_run(&run.dir).expect("load ended run");
        assert_eq!(ended.state.lifecycle, RunLifecycle::Ended);
        assert_eq!(
            ended.state.workers["builder"].lifecycle,
            WorkerLifecycle::Ended
        );
        assert_eq!(
            ended.state.workers["reviewer"].lifecycle,
            WorkerLifecycle::Failed
        );
    }

    #[test]
    fn kill_worker_leaves_other_workers_and_run_active_and_refuses_dirty_worktree() {
        let temp = TempDir::new();
        let state = run_state(temp.path());
        let run = create_run(temp.path(), state).expect("create run");
        let backend = FakeTeardown {
            dirty: BTreeSet::from([run.state.workers["builder"].worktree_path.clone().unwrap()]),
            ..Default::default()
        };
        let error = kill_worker_with_backend(&run.dir, "builder", true, &backend)
            .expect_err("dirty worker must be preserved");
        assert!(matches!(error, StatusKillError::DirtyWorktrees { .. }));
        let persisted = load_run(&run.dir).expect("load run");
        assert_eq!(persisted.state.lifecycle, RunLifecycle::Active);
        assert_eq!(
            persisted.state.workers["builder"].lifecycle,
            WorkerLifecycle::Ended
        );
        assert_eq!(
            persisted.state.workers["reviewer"].lifecycle,
            WorkerLifecycle::Running
        );
        assert_eq!(backend.closed.into_inner(), ["workspace-builder"]);
    }

    #[test]
    fn kill_preserves_failed_adopted_workers_without_releasing_them() {
        let temp = TempDir::new();
        let mut state = run_state(temp.path());
        let adopted = state.workers.get_mut("reviewer").expect("reviewer runtime");
        adopted.adopted = true;
        adopted.pane_id = Some("pane-failed".to_owned());
        adopted.lifecycle = WorkerLifecycle::Failed;
        let run = create_run(temp.path(), state).expect("create failed adopted run");
        let backend = FakeTeardown::default();

        kill_run_with_backend(&run.dir, false, &backend).expect("kill run");

        let ended = load_run(&run.dir).expect("load ended run");
        assert_eq!(ended.state.lifecycle, RunLifecycle::Ended);
        assert_eq!(
            ended.state.workers["reviewer"].lifecycle,
            WorkerLifecycle::Failed
        );
        assert!(backend.notices.into_inner().is_empty());
    }

    #[test]
    fn kill_with_a_closed_adopted_pane_releases_every_worker_and_ends_the_run() {
        let temp = TempDir::new();
        let mut state = run_state(temp.path());
        let adopted = state.workers.get_mut("reviewer").expect("reviewer runtime");
        adopted.adopted = true;
        adopted.workspace_id = Some("workspace-borrowed".to_owned());
        adopted.pane_id = Some("pane-closed".to_owned());
        adopted.worktree_path = None;
        let run = create_run(temp.path(), state).expect("create mixed-ownership run");
        let backend = FakeTeardown {
            unavailable_panes: BTreeSet::from(["pane-closed".to_owned()]),
            unavailable_workspaces: BTreeSet::from(["workspace-builder".to_owned()]),
            ..FakeTeardown::default()
        };

        kill_run_with_backend(&run.dir, false, &backend)
            .expect("missing adopted resources must not abort kill");

        assert_eq!(backend.closed.into_inner(), ["workspace-builder"]);
        let ended = load_run(&run.dir).expect("load ended run");
        assert_eq!(ended.state.lifecycle, RunLifecycle::Ended);
        assert_eq!(
            ended.state.workers["builder"].lifecycle,
            WorkerLifecycle::Ended
        );
        assert_eq!(
            ended.state.workers["reviewer"].lifecycle,
            WorkerLifecycle::Released
        );
    }

    #[test]
    fn kill_repairs_stale_worker_lifecycles_without_repeating_ended_run_teardown() {
        let temp = TempDir::new();
        let mut run = create_run(temp.path(), run_state(temp.path())).expect("create run");
        mark_ended(&mut run).expect("create legacy ended run");
        let backend = FakeTeardown::default();

        kill_run_with_backend(&run.dir, false, &backend).expect("repair ended run");

        assert!(backend.closed.into_inner().is_empty());
        assert!(load_run(&run.dir)
            .expect("load repaired run")
            .state
            .workers
            .values()
            .all(|worker| worker.lifecycle == WorkerLifecycle::Ended));
    }

    #[test]
    fn kill_removes_clean_worktrees_but_preserves_every_dirty_path() {
        let temp = TempDir::new();
        let run = create_run(temp.path(), run_state(temp.path())).expect("create run");
        let clean = temp.path().join("builder");
        let dirty = temp.path().join("reviewer");
        let backend = FakeTeardown {
            dirty: BTreeSet::from([dirty.clone()]),
            ..FakeTeardown::default()
        };

        let error = kill_run_with_backend(&run.dir, true, &backend).unwrap_err();

        assert!(matches!(
            error,
            StatusKillError::DirtyWorktrees { paths } if paths == vec![dirty.clone()]
        ));
        assert_eq!(backend.inspected.into_inner(), vec![clean.clone(), dirty]);
        assert_eq!(backend.removed.into_inner(), vec![clean]);
        let ended = load_run(&run.dir).expect("load ended run");
        assert_eq!(ended.state.lifecycle, RunLifecycle::Ended);
        assert_eq!(
            ended
                .state
                .workers
                .values()
                .map(|worker| worker.lifecycle)
                .collect::<Vec<_>>(),
            [WorkerLifecycle::Ended, WorkerLifecycle::Ended]
        );
    }

    #[test]
    fn git_status_boundary_detects_untracked_evidence() {
        let temp = TempDir::new();
        let repo = temp.path().join("repo");
        let init = Command::new("git")
            .args(["init", "--quiet"])
            .arg(&repo)
            .status()
            .expect("run git init");
        assert!(init.success());
        assert!(!worktree_is_dirty(&repo).expect("clean status"));

        fs::write(repo.join("evidence.txt"), "uncommitted").expect("write evidence");

        assert!(worktree_is_dirty(&repo).expect("dirty status"));
    }

    #[test]
    fn command_args_accept_one_run_and_reject_unknown_or_duplicate_options() {
        assert_eq!(
            parse_command_args(
                &["--json".to_owned(), "/run/one".to_owned()],
                "--json",
                STATUS_USAGE
            )
            .unwrap(),
            (PathBuf::from("/run/one"), true)
        );
        assert!(parse_command_args(&["--wat".to_owned()], "--json", STATUS_USAGE).is_err());
        assert!(parse_command_args(
            &["--json".to_owned(), "--json".to_owned()],
            "--json",
            STATUS_USAGE
        )
        .is_err());
    }
}
