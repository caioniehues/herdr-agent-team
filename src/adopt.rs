//! Existing-pane adoption from `docs/spec.md` section 12 and ADR-0009.

use crate::herdr::{HerdrClient, HerdrError, PaneInfo};
use crate::launcher::{
    conservative_adopted_launcher, default_launcher_table, load_launcher_table, LauncherError,
};
use crate::run::{create_run, list_active_runs, load_run, save_run, RunBoard, RunError};
use crate::spawn::{
    adoption_prompt, is_safe_worker_filename, submit_worker_prompt, worker_protocol_path,
    write_worker_protocol, HerdrApi, SpawnError,
};
use crate::types::{
    current_herdr_session_identity, GodSpec, LauncherEntry, LauncherTable, RunLifecycle, RunState,
    TeamSpec, Topology, WorkerLifecycle, WorkerRunState, WorkerSpec,
};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

const ADOPT_USAGE: &str = "usage: herdr-agent-team adopt <pane-id> --name <worker> [--role <text>] [--brief <path>] [--run <run-dir>] [--team <name>]";
const DEFAULT_TEAM: &str = "adhoc";
const DEFAULT_ROLE: &str = "adopted worker";

#[derive(Debug, Error)]
pub enum AdoptError {
    #[error("{0}")]
    Arguments(String),

    #[error(transparent)]
    Herdr(#[from] HerdrError),

    #[error(transparent)]
    Launcher(#[from] LauncherError),

    #[error(transparent)]
    Run(#[from] RunError),

    #[error(transparent)]
    Spawn(#[from] SpawnError),

    #[error("required environment variable {0} is not set")]
    MissingEnvironment(&'static str),

    #[error("multiple active team runs found; pass --run with one of: {candidates}")]
    AmbiguousRuns { candidates: String },

    #[error(
        "--team names a new ad-hoc team, but run {run_dir} is active; pass --run <dir> or kill it"
    )]
    TeamConflictsWithActiveRun { run_dir: PathBuf },

    #[error("cannot adopt into ended run `{run_dir}`")]
    InactiveRun { run_dir: PathBuf },

    #[error("cannot adopt into mesh run `{run_dir}`; immutable peer protocols cannot include the newcomer")]
    MeshRun { run_dir: PathBuf },

    #[error("worker name '{worker}' must be a safe single filename component")]
    UnsafeWorkerName { worker: String },

    #[error("worker '{worker}' already exists in run `{run_dir}`")]
    DuplicateWorker { worker: String, run_dir: PathBuf },

    #[error("pane '{pane_id}' is already a worker in run `{run_dir}`")]
    DuplicatePane { pane_id: String, run_dir: PathBuf },

    #[error("pane '{pane_id}' has no detected agent and cannot be adopted")]
    AgentNotDetected { pane_id: String },

    #[error("pane '{pane_id}' has no reported cwd; cannot bootstrap an ad-hoc run")]
    PaneCwdNotDetected { pane_id: String },

    #[error("brief does not exist or is inaccessible `{path}`: {source}")]
    Brief {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to read current directory: {0}")]
    CurrentDirectory(std::io::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AdoptArguments {
    pane_id: String,
    name: String,
    role: String,
    brief: Option<PathBuf>,
    run_dir: Option<PathBuf>,
    team: Option<String>,
}

#[derive(Debug)]
enum RunTarget {
    Existing(Box<RunBoard>),
    Bootstrap,
}

#[derive(Debug)]
struct AdoptOutcome {
    run: RunBoard,
    unknown_agent: Option<String>,
}

pub fn adopt_command(args: &[String]) -> Result<(), AdoptError> {
    let mut arguments = parse_adopt_arguments(args)?;
    let current_dir = env::current_dir().map_err(AdoptError::CurrentDirectory)?;
    if let Some(brief) = arguments.brief.as_mut() {
        let unresolved = absolutize(brief, &current_dir);
        let resolved = fs::canonicalize(&unresolved).map_err(|source| AdoptError::Brief {
            path: unresolved,
            source,
        })?;
        if !resolved.is_file() {
            return Err(AdoptError::Brief {
                path: resolved,
                source: std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a file"),
            });
        }
        *brief = resolved;
    }

    let state_dir = env::var_os("HERDR_PLUGIN_STATE_DIR")
        .map(|path| absolutize(Path::new(&path), &current_dir))
        .ok_or(AdoptError::MissingEnvironment("HERDR_PLUGIN_STATE_DIR"))?;
    let target = select_run_target(
        arguments.run_dir.as_deref(),
        arguments.team.as_deref(),
        &state_dir,
        &current_dir,
    )?;
    let god_pane_id = match &target {
        RunTarget::Existing(run) => run.state.god_pane_id.clone(),
        RunTarget::Bootstrap => env::var("HERDR_PANE_ID")
            .map_err(|_| AdoptError::MissingEnvironment("HERDR_PANE_ID"))?,
    };
    let launchers = match env::var_os("HERDR_PLUGIN_CONFIG_DIR") {
        Some(path) => load_launcher_table(&absolutize(Path::new(&path), &current_dir))?,
        None => default_launcher_table(),
    };
    let herdr = HerdrClient::from_env();
    let outcome = adopt_resolved(
        arguments,
        target,
        &state_dir,
        &god_pane_id,
        &launchers,
        &herdr,
    )?;
    if let Some(agent) = outcome.unknown_agent.as_deref() {
        eprintln!("{}", unknown_agent_warning(agent));
    }
    println!(
        "worker adopted into team run: {}",
        outcome.run.dir.display()
    );
    Ok(())
}

fn parse_adopt_arguments(args: &[String]) -> Result<AdoptArguments, AdoptError> {
    let mut pane_id = None;
    let mut name = None;
    let mut role = None;
    let mut brief = None;
    let mut run_dir = None;
    let mut team = None;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--name" => set_option(args, &mut index, &mut name, "--name")?,
            "--role" => set_option(args, &mut index, &mut role, "--role")?,
            "--brief" => set_path_option(args, &mut index, &mut brief, "--brief")?,
            "--run" => set_path_option(args, &mut index, &mut run_dir, "--run")?,
            "--team" => set_option(args, &mut index, &mut team, "--team")?,
            option if option.starts_with('-') => {
                return Err(arguments(format!("unknown option '{option}'")));
            }
            value => {
                if pane_id.replace(value.to_owned()).is_some() {
                    return Err(arguments("expected exactly one pane id"));
                }
            }
        }
        index += 1;
    }

    let pane_id = pane_id.ok_or_else(|| arguments("missing pane id"))?;
    let name = name.ok_or_else(|| arguments("--name is required"))?;
    if !is_safe_worker_filename(&name) {
        return Err(AdoptError::UnsafeWorkerName { worker: name });
    }

    Ok(AdoptArguments {
        pane_id,
        name,
        role: role.unwrap_or_else(|| DEFAULT_ROLE.to_owned()),
        brief,
        run_dir,
        team,
    })
}

fn set_option(
    args: &[String],
    index: &mut usize,
    slot: &mut Option<String>,
    option: &'static str,
) -> Result<(), AdoptError> {
    if slot.is_some() {
        return Err(arguments(format!("{option} may only be supplied once")));
    }
    *index += 1;
    let value = args
        .get(*index)
        .ok_or_else(|| arguments(format!("{option} requires a value")))?;
    *slot = Some(value.clone());
    Ok(())
}

fn set_path_option(
    args: &[String],
    index: &mut usize,
    slot: &mut Option<PathBuf>,
    option: &'static str,
) -> Result<(), AdoptError> {
    if slot.is_some() {
        return Err(arguments(format!("{option} may only be supplied once")));
    }
    *index += 1;
    let value = args
        .get(*index)
        .ok_or_else(|| arguments(format!("{option} requires a value")))?;
    *slot = Some(PathBuf::from(value));
    Ok(())
}

fn arguments(detail: impl AsRef<str>) -> AdoptError {
    AdoptError::Arguments(format!("{}; {ADOPT_USAGE}", detail.as_ref()))
}

fn select_run_target(
    explicit_run: Option<&Path>,
    team: Option<&str>,
    state_dir: &Path,
    current_dir: &Path,
) -> Result<RunTarget, AdoptError> {
    if explicit_run.is_some() && team.is_some() {
        return Err(arguments("--team and --run cannot be used together"));
    }
    if let Some(run_dir) = explicit_run {
        let run = load_run(&absolutize(run_dir, current_dir))?;
        return choose_run(Some(run), Vec::new());
    }
    let active_runs = list_active_runs(state_dir)?;
    if team.is_some() {
        if let Some(run) = active_runs.first() {
            return Err(AdoptError::TeamConflictsWithActiveRun {
                run_dir: run.dir.clone(),
            });
        }
        return Ok(RunTarget::Bootstrap);
    }
    choose_run(None, active_runs)
}

fn choose_run(
    explicit_run: Option<RunBoard>,
    mut active_runs: Vec<RunBoard>,
) -> Result<RunTarget, AdoptError> {
    if let Some(run) = explicit_run {
        if run.state.lifecycle != RunLifecycle::Active {
            return Err(AdoptError::InactiveRun {
                run_dir: run.dir.clone(),
            });
        }
        return Ok(RunTarget::Existing(Box::new(run)));
    }

    match active_runs.len() {
        0 => Ok(RunTarget::Bootstrap),
        1 => Ok(RunTarget::Existing(Box::new(
            active_runs.pop().expect("length checked"),
        ))),
        _ => Err(AdoptError::AmbiguousRuns {
            candidates: active_runs
                .iter()
                .map(|run| run.dir.display().to_string())
                .collect::<Vec<_>>()
                .join(", "),
        }),
    }
}

fn adopt_resolved<H: HerdrApi>(
    arguments: AdoptArguments,
    target: RunTarget,
    state_dir: &Path,
    god_pane_id: &str,
    launchers: &LauncherTable,
    herdr: &H,
) -> Result<AdoptOutcome, AdoptError> {
    let pane = herdr.pane_get(&arguments.pane_id)?;
    let agent = pane
        .agent
        .as_deref()
        .filter(|agent| !agent.is_empty())
        .ok_or_else(|| AdoptError::AgentNotDetected {
            pane_id: arguments.pane_id.clone(),
        })?
        .to_owned();
    let (launcher, unknown_agent) = adoption_launcher(launchers, &agent);
    let worker = WorkerSpec {
        name: arguments.name,
        agent,
        role: arguments.role,
        worktree: false,
        branch: None,
        brief: arguments.brief.clone().unwrap_or_default(),
    };

    let mut run = match target {
        RunTarget::Existing(run) => prepare_existing_run(*run, &worker, &pane)?,
        RunTarget::Bootstrap => bootstrap_run(
            state_dir,
            arguments.team.as_deref().unwrap_or(DEFAULT_TEAM),
            god_pane_id,
            &worker,
            &pane,
        )?,
    };
    run.state.herdr_session = current_herdr_session_identity();

    save_run(&run)?;
    let mut protocol_team = run.state.spec.clone();
    if let Some(cwd) = pane.cwd.clone() {
        protocol_team.cwd = cwd;
    }
    write_worker_protocol(&protocol_team, &worker, &run)?;

    let protocol_path = worker_protocol_path(&run, &worker);
    let prompt = adoption_prompt(
        &worker,
        &launcher,
        &protocol_path,
        arguments.brief.is_some(),
    );
    if let Err(error) = submit_worker_prompt(
        herdr,
        &worker,
        &pane.pane_id,
        &prompt,
        launcher.submit_verify,
    ) {
        run.state
            .workers
            .get_mut(&worker.name)
            .expect("adopted worker was inserted before protocol generation")
            .lifecycle = WorkerLifecycle::Failed;
        save_run(&run)?;
        return Err(error.into());
    }

    run.state
        .workers
        .get_mut(&worker.name)
        .expect("adopted worker was inserted before protocol generation")
        .lifecycle = WorkerLifecycle::Running;
    save_run(&run)?;

    // Keep the in-memory return value synchronized with the final persisted state.
    run = load_run(&run.dir)?;
    Ok(AdoptOutcome { run, unknown_agent })
}

fn prepare_existing_run(
    mut run: RunBoard,
    worker: &WorkerSpec,
    pane: &PaneInfo,
) -> Result<RunBoard, AdoptError> {
    if run.state.lifecycle != RunLifecycle::Active {
        return Err(AdoptError::InactiveRun {
            run_dir: run.dir.clone(),
        });
    }
    if run.state.spec.topology == Topology::Mesh {
        return Err(AdoptError::MeshRun {
            run_dir: run.dir.clone(),
        });
    }
    if run.state.workers.contains_key(&worker.name)
        || run
            .state
            .spec
            .workers
            .iter()
            .any(|existing| existing.name == worker.name)
    {
        return Err(AdoptError::DuplicateWorker {
            worker: worker.name.clone(),
            run_dir: run.dir.clone(),
        });
    }
    if run
        .state
        .workers
        .values()
        .any(|existing| existing.pane_id.as_deref() == Some(&pane.pane_id))
    {
        return Err(AdoptError::DuplicatePane {
            pane_id: pane.pane_id.clone(),
            run_dir: run.dir.clone(),
        });
    }

    insert_adopted_worker(&mut run.state, worker, pane);
    Ok(run)
}

fn bootstrap_run(
    state_dir: &Path,
    team_name: &str,
    god_pane_id: &str,
    worker: &WorkerSpec,
    pane: &PaneInfo,
) -> Result<RunBoard, AdoptError> {
    let cwd = pane
        .cwd
        .clone()
        .ok_or_else(|| AdoptError::PaneCwdNotDetected {
            pane_id: pane.pane_id.clone(),
        })?;
    let mut state = RunState {
        spec: TeamSpec {
            name: team_name.to_owned(),
            topology: Topology::Star,
            cwd,
            setup: Vec::new(),
            god: GodSpec::default(),
            workers: Vec::new(),
        },
        god_pane_id: god_pane_id.to_owned(),
        herdr_session: current_herdr_session_identity(),
        workers: BTreeMap::new(),
        lifecycle: RunLifecycle::Active,
    };
    insert_adopted_worker(&mut state, worker, pane);
    Ok(create_run(state_dir, state)?)
}

fn insert_adopted_worker(state: &mut RunState, worker: &WorkerSpec, pane: &PaneInfo) {
    state.spec.workers.push(worker.clone());
    state.workers.insert(
        worker.name.clone(),
        WorkerRunState {
            workspace_id: Some(pane.workspace_id.clone()),
            pane_id: Some(pane.pane_id.clone()),
            agent_id: pane.agent_id.clone(),
            agent_session: pane.agent_session.clone(),
            worktree_path: None,
            adopted: true,
            lifecycle: WorkerLifecycle::Pending,
        },
    );
}

fn adoption_launcher(table: &LauncherTable, agent: &str) -> (LauncherEntry, Option<String>) {
    match table.get(agent) {
        Some(launcher) => (launcher.clone(), None),
        None => (conservative_adopted_launcher(agent), Some(agent.to_owned())),
    }
}

fn unknown_agent_warning(agent: &str) -> String {
    let key = if agent
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        agent.to_owned()
    } else {
        format!("{:?}", agent)
    };
    let command = format!("{:?}", agent);
    format!(
        "warning: detected unknown agent kind '{agent}'; adopting with a conservative policy. Add this exact entry to agents.toml:\n[{key}]\ncommand = [{command}]\nsubmit_verify = true\nreads_agents_md = \"pointer\"\nqueues_midturn = false"
    )
}

fn absolutize(path: &Path, base: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::herdr::{WaitOutcome, WorkspaceRef, WorktreeRef};
    use crate::types::AgentsMdMode;
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("test clock should follow Unix epoch")
                .as_nanos();
            let path = env::temp_dir().join(format!(
                "herdr-adopt-tests-{}-{nanos}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("create adopt test directory");
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

    struct FakeHerdr {
        pane: PaneInfo,
        calls: RefCell<Vec<String>>,
    }

    impl FakeHerdr {
        fn new(root: &Path, agent: Option<&str>) -> Self {
            Self {
                pane: PaneInfo {
                    pane_id: "pane-adopted".to_owned(),
                    workspace_id: "workspace-borrowed".to_owned(),
                    agent: agent.map(str::to_owned),
                    agent_id: Some("session-adopted".to_owned()),
                    agent_session: Some(crate::herdr::AgentSession {
                        source: "herdr:claude".to_owned(),
                        agent: "claude".to_owned(),
                        kind: "id".to_owned(),
                        value: "session-adopted".to_owned(),
                    }),
                    agent_status: Some("idle".to_owned()),
                    cwd: Some(root.to_path_buf()),
                },
                calls: RefCell::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
    }

    impl HerdrApi for FakeHerdr {
        fn health_check(&self) -> Result<(), HerdrError> {
            unreachable!("adoption does not perform spawn preflight")
        }

        fn worktree_create(&self, _repo: &Path, _branch: &str) -> Result<WorktreeRef, HerdrError> {
            unreachable!("adoption does not create worktrees")
        }

        fn workspace_create(&self, _cwd: &Path, _label: &str) -> Result<WorkspaceRef, HerdrError> {
            unreachable!("adoption does not create workspaces")
        }

        fn pane_run(&self, pane_id: &str, input: &str) -> Result<(), HerdrError> {
            self.calls
                .borrow_mut()
                .push(format!("pane_run:{pane_id}:{input}"));
            Ok(())
        }

        fn agent_wait(
            &self,
            pane_id: &str,
            status: &str,
            _timeout: Duration,
        ) -> Result<WaitOutcome, HerdrError> {
            self.calls
                .borrow_mut()
                .push(format!("agent_wait:{pane_id}:{status}"));
            Ok(WaitOutcome::Reached)
        }

        fn pane_get(&self, pane_id: &str) -> Result<PaneInfo, HerdrError> {
            self.calls.borrow_mut().push(format!("pane_get:{pane_id}"));
            Ok(self.pane.clone())
        }
    }

    fn adopted_arguments(name: &str) -> AdoptArguments {
        AdoptArguments {
            pane_id: "pane-adopted".to_owned(),
            name: name.to_owned(),
            role: DEFAULT_ROLE.to_owned(),
            brief: None,
            run_dir: None,
            team: None,
        }
    }

    fn existing_run(root: &Path, topology: Topology) -> RunBoard {
        let existing = WorkerSpec {
            name: "existing".to_owned(),
            agent: "claude".to_owned(),
            role: "existing worker".to_owned(),
            worktree: false,
            branch: None,
            brief: root.join("existing.md"),
        };
        create_run(
            root,
            RunState {
                spec: TeamSpec {
                    name: "active-team".to_owned(),
                    topology,
                    cwd: root.to_path_buf(),
                    setup: Vec::new(),
                    god: GodSpec::default(),
                    workers: vec![existing],
                },
                god_pane_id: "god-pane".to_owned(),
                herdr_session: Default::default(),
                workers: BTreeMap::from([(
                    "existing".to_owned(),
                    WorkerRunState {
                        workspace_id: Some("workspace-existing".to_owned()),
                        pane_id: Some("pane-existing".to_owned()),
                        agent_id: Some("session-existing".to_owned()),
                        agent_session: None,
                        worktree_path: None,
                        adopted: false,
                        lifecycle: WorkerLifecycle::Running,
                    },
                )]),
                lifecycle: RunLifecycle::Active,
            },
        )
        .expect("create active test run")
    }

    #[test]
    fn parses_the_documented_cli_and_defaults_role() {
        let parsed = parse_adopt_arguments(&[
            "w1:p7".to_owned(),
            "--name".to_owned(),
            "researcher".to_owned(),
            "--team".to_owned(),
            "scratch".to_owned(),
        ])
        .expect("parse adopt arguments");

        assert_eq!(parsed.pane_id, "w1:p7");
        assert_eq!(parsed.name, "researcher");
        assert_eq!(parsed.role, DEFAULT_ROLE);
        assert_eq!(parsed.team.as_deref(), Some("scratch"));
        assert!(parse_adopt_arguments(&["w1:p7".to_owned()]).is_err());
        assert!(parse_adopt_arguments(&[
            "w1:p7".to_owned(),
            "--name".to_owned(),
            "../escape".to_owned(),
        ])
        .is_err());
        assert!(parse_adopt_arguments(&[
            "w1:p7".to_owned(),
            "--name".to_owned(),
            "worker".to_owned(),
            "--run".to_owned(),
            "/run/one".to_owned(),
            "--run".to_owned(),
            "/run/two".to_owned(),
        ])
        .is_err());
    }

    #[test]
    fn several_active_runs_are_ambiguous_and_list_every_candidate() {
        let first_temp = TempDir::new();
        let second_temp = TempDir::new();
        let first = existing_run(first_temp.path(), Topology::Star);
        let second = existing_run(second_temp.path(), Topology::Star);

        let error = choose_run(None, vec![first.clone(), second.clone()])
            .expect_err("multiple implicit candidates must be refused")
            .to_string();

        assert!(error.contains(&first.dir.display().to_string()));
        assert!(error.contains(&second.dir.display().to_string()));
        assert!(error.contains("pass --run"));
    }

    #[test]
    fn team_is_rejected_when_an_active_run_exists() {
        let temp = TempDir::new();
        let active = existing_run(temp.path(), Topology::Star);

        let error = select_run_target(None, Some("scratch"), temp.path(), temp.path())
            .expect_err("--team must not silently select an active run")
            .to_string();

        assert_eq!(
            error,
            format!(
                "--team names a new ad-hoc team, but run {} is active; pass --run <dir> or kill it",
                active.dir.display()
            )
        );
    }

    #[test]
    fn team_and_explicit_run_are_contradictory() {
        let temp = TempDir::new();
        let run = existing_run(temp.path(), Topology::Star);

        let error = select_run_target(Some(&run.dir), Some("scratch"), temp.path(), temp.path())
            .expect_err("--team and --run must be refused")
            .to_string();

        assert!(error.contains("--team and --run cannot be used together"));
    }

    #[test]
    fn team_bootstraps_when_no_run_is_active() {
        let temp = TempDir::new();

        let target = select_run_target(None, Some("scratch"), temp.path(), temp.path())
            .expect("--team should bootstrap only without an active run");

        assert!(matches!(target, RunTarget::Bootstrap));
    }

    #[test]
    fn adopts_into_an_active_star_run_with_a_fresh_protocol() {
        let temp = TempDir::new();
        let run = existing_run(temp.path(), Topology::Star);
        let fake = FakeHerdr::new(temp.path(), Some("codex"));

        let outcome = adopt_resolved(
            adopted_arguments("newcomer"),
            RunTarget::Existing(Box::new(run)),
            temp.path(),
            "god-pane",
            &default_launcher_table(),
            &fake,
        )
        .expect("adopt into active run");

        let persisted = load_run(&outcome.run.dir).expect("load adopted run");
        assert_eq!(persisted.state.spec.workers.len(), 2);
        assert_eq!(persisted.state.spec.workers[1].name, "newcomer");
        assert_eq!(persisted.state.spec.workers[1].agent, "codex");
        assert!(persisted.state.workers["newcomer"].adopted);
        assert_eq!(
            persisted.state.workers["newcomer"].lifecycle,
            WorkerLifecycle::Running
        );
        let protocol = fs::read_to_string(outcome.run.dir.join("protocols/newcomer.md"))
            .expect("read immutable adopted protocol");
        assert!(protocol.contains("- Worker: `newcomer`"));
        assert!(protocol.contains("- Workspace: `workspace-borrowed`"));
        let calls = fake.calls();
        assert_eq!(calls[0], "pane_get:pane-adopted");
        assert!(calls.iter().any(|call| {
            call.starts_with("pane_run:pane-adopted:The repository's authored AGENTS.md")
                && call.contains("protocols/newcomer.md")
                && !call.contains("Read your brief")
        }));
        assert!(calls
            .iter()
            .any(|call| call == "agent_wait:pane-adopted:working"));
    }

    #[test]
    fn optional_brief_uses_the_launch_style_brief_and_protocol_pair() {
        let temp = TempDir::new();
        let run = existing_run(temp.path(), Topology::Star);
        let brief = temp.path().join("adopted-brief.md");
        fs::write(&brief, "adopted task").expect("write adopted brief");
        let mut arguments = adopted_arguments("briefed");
        arguments.brief = Some(brief.clone());
        let fake = FakeHerdr::new(temp.path(), Some("claude"));

        adopt_resolved(
            arguments,
            RunTarget::Existing(Box::new(run)),
            temp.path(),
            "god-pane",
            &default_launcher_table(),
            &fake,
        )
        .expect("adopt briefed worker");

        assert!(fake.calls().iter().any(|call| {
            call.starts_with(&format!(
                "pane_run:pane-adopted:Read your brief at {} and execute it fully.",
                brief.display()
            )) && call.contains("Read the generated team protocol at")
                && call.contains("protocols/briefed.md")
        }));
    }

    #[test]
    fn bootstraps_a_minimal_adhoc_star_run_from_pane_metadata() {
        let temp = TempDir::new();
        let fake = FakeHerdr::new(temp.path(), Some("claude"));

        let outcome = adopt_resolved(
            adopted_arguments("researcher"),
            RunTarget::Bootstrap,
            temp.path(),
            "god-current",
            &default_launcher_table(),
            &fake,
        )
        .expect("bootstrap adopted run");

        let persisted = load_run(&outcome.run.dir).expect("load bootstrap run");
        assert_eq!(persisted.state.spec.name, DEFAULT_TEAM);
        assert_eq!(persisted.state.spec.topology, Topology::Star);
        assert_eq!(persisted.state.spec.cwd, temp.path());
        assert_eq!(persisted.state.god_pane_id, "god-current");
        assert_eq!(persisted.state.spec.workers.len(), 1);
        assert!(persisted.state.workers["researcher"].adopted);
        assert_eq!(
            persisted.state.workers["researcher"]
                .agent_session
                .as_ref()
                .map(|session| session.source.as_str()),
            Some("herdr:claude")
        );
        let run_toml = fs::read_to_string(outcome.run.dir.join("run.toml"))
            .expect("read reconstructed run spec");
        assert!(run_toml.contains("name = \"adhoc\""));
        assert!(run_toml.contains("topology = \"star\""));
        assert!(run_toml.contains("adopted = true"));
    }

    #[test]
    fn refuses_mesh_runs_before_protocol_or_prompt_mutation() {
        let temp = TempDir::new();
        let run = existing_run(temp.path(), Topology::Mesh);
        let fake = FakeHerdr::new(temp.path(), Some("codex"));

        let error = adopt_resolved(
            adopted_arguments("newcomer"),
            RunTarget::Existing(Box::new(run.clone())),
            temp.path(),
            "god-pane",
            &default_launcher_table(),
            &fake,
        )
        .expect_err("mesh adoption must fail");

        assert!(matches!(error, AdoptError::MeshRun { .. }));
        assert_eq!(fake.calls(), ["pane_get:pane-adopted"]);
        assert!(!run.dir.join("protocols/newcomer.md").exists());
        assert_eq!(load_run(&run.dir).unwrap().state.spec.workers.len(), 1);
    }

    #[test]
    fn unknown_agent_uses_conservative_policy_and_names_exact_config_entry() {
        let temp = TempDir::new();
        let fake = FakeHerdr::new(temp.path(), Some("opencode"));

        let outcome = adopt_resolved(
            adopted_arguments("unknown-kind"),
            RunTarget::Bootstrap,
            temp.path(),
            "god-pane",
            &default_launcher_table(),
            &fake,
        )
        .expect("unknown detected agent should still be adopted");

        assert_eq!(outcome.unknown_agent.as_deref(), Some("opencode"));
        let (launcher, unknown) = adoption_launcher(&default_launcher_table(), "opencode");
        assert!(launcher.submit_verify);
        assert!(!launcher.queues_midturn);
        assert_eq!(launcher.reads_agents_md, AgentsMdMode::Pointer);
        assert_eq!(unknown.as_deref(), Some("opencode"));
        assert_eq!(
            unknown_agent_warning("opencode"),
            concat!(
                "warning: detected unknown agent kind 'opencode'; adopting with a conservative policy. Add this exact entry to agents.toml:\n",
                "[opencode]\n",
                "command = [\"opencode\"]\n",
                "submit_verify = true\n",
                "reads_agents_md = \"pointer\"\n",
                "queues_midturn = false"
            )
        );
        assert!(fake.calls().iter().any(|call| {
            call.starts_with("pane_run:pane-adopted:Read the generated team protocol")
        }));
    }

    #[test]
    fn refuses_a_pane_without_a_detected_agent_before_creating_a_run() {
        let temp = TempDir::new();
        let fake = FakeHerdr::new(temp.path(), None);

        let error = adopt_resolved(
            adopted_arguments("no-agent"),
            RunTarget::Bootstrap,
            temp.path(),
            "god-pane",
            &default_launcher_table(),
            &fake,
        )
        .expect_err("pane without agent must be refused");

        assert!(matches!(error, AdoptError::AgentNotDetected { .. }));
        assert_eq!(fake.calls(), ["pane_get:pane-adopted"]);
        assert!(!temp.path().join("runs").exists());
    }
}
