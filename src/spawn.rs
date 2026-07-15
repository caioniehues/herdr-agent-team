//! Team preflight and worker launch flow from `docs/spec.md` section 4.

use crate::agents_md::{render_agents_md, AgentsMdError};
use crate::herdr::{HerdrApi, HerdrClient, HerdrError, PaneInfo, WaitOutcome};
use crate::launcher::{launcher_entry, load_from_env, LauncherError};
use crate::run::{create_run, save_run, RunBoard, RunError};
use crate::spec::{
    load_team_spec, spawn_command as dry_run_command, team_spec_from_agents, validate_team_spec,
    SpecError,
};
use crate::types::{
    current_herdr_session_identity, AgentsMdMode, LauncherEntry, LauncherTable, RunLifecycle,
    RunState, TeamSpec, WorkerLifecycle, WorkerRunState, WorkerSpec,
};
use std::collections::BTreeMap;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};
use thiserror::Error;

const PROTOCOLS_DIR: &str = "protocols";
// DoD run 1: a loaded Claude session reported its agent id shortly after 30 s.
const AGENT_START_TIMEOUT: Duration = Duration::from_secs(90);
const SUBMIT_GRACE_TIMEOUT: Duration = Duration::from_secs(2);
const SUBMIT_VERIFY_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Error)]
pub enum SpawnError {
    #[error(transparent)]
    Spec(#[from] SpecError),
    #[error(transparent)]
    Launcher(#[from] LauncherError),
    #[error(transparent)]
    Herdr(#[from] HerdrError),
    #[error(transparent)]
    Run(#[from] RunError),
    #[error(transparent)]
    AgentsMd(#[from] AgentsMdError),
    #[error("invalid spawn arguments: {0}")]
    Arguments(String),
    #[error("required environment variable {0} is not set")]
    MissingEnvironment(&'static str),
    #[error("team must contain at least one worker")]
    EmptyTeam,
    #[error("team cwd does not exist or is inaccessible `{path}`: {source}")]
    TeamCwd {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("worker '{worker}' brief does not exist or is inaccessible `{path}`: {source}")]
    Brief {
        worker: String,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("worker '{worker}' launcher command is empty")]
    EmptyLauncher { worker: String },
    #[error("worker name '{worker}' must be a safe single filename component")]
    UnsafeWorkerName { worker: String },
    #[error("worker '{worker}' agent CLI is not executable on PATH: {command}")]
    AgentCliMissing { worker: String, command: String },
    #[error("worker '{worker}' failed to start setup command `{command}` in `{cwd}`: {source}")]
    SetupSpawn {
        worker: String,
        command: String,
        cwd: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "worker '{worker}' setup command `{command}` failed in `{cwd}` with status {status:?}; stdout:\n{stdout}\nstderr:\n{stderr}"
    )]
    SetupFailed {
        worker: String,
        command: String,
        cwd: PathBuf,
        status: Option<i32>,
        stdout: Box<str>,
        stderr: Box<str>,
    },
    #[error("failed to {action} `{path}`: {source}")]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("worker '{worker}' failed during {step}: {source}")]
    WorkerHerdr {
        worker: String,
        step: &'static str,
        #[source]
        source: HerdrError,
    },
    #[error("worker '{worker}' timed out waiting for agent status '{status}' during {step}")]
    WorkerTimeout {
        worker: String,
        status: &'static str,
        step: &'static str,
    },
    #[error("worker '{worker}' launched in pane '{pane_id}', but Herdr did not detect an agent")]
    AgentNotDetected { worker: String, pane_id: String },
}

#[derive(Debug, PartialEq, Eq)]
struct SpawnArguments {
    spec_path: Option<PathBuf>,
    agents: Option<String>,
}

struct SpawnContext {
    spec: TeamSpec,
    launchers: LauncherTable,
    state_dir: PathBuf,
    god_pane_id: String,
}

#[derive(Debug, PartialEq, Eq)]
struct SetupOutput {
    success: bool,
    status: Option<i32>,
    stdout: String,
    stderr: String,
}

trait SetupRunner {
    fn run(&self, cwd: &Path, command: &str) -> Result<SetupOutput, std::io::Error>;
}

struct ProcessSetupRunner;

impl SetupRunner for ProcessSetupRunner {
    fn run(&self, cwd: &Path, command: &str) -> Result<SetupOutput, std::io::Error> {
        let output = Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(cwd)
            .output()?;
        Ok(SetupOutput {
            success: output.status.success(),
            status: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

pub fn spawn_command(args: &[String]) -> Result<(), SpawnError> {
    if args.iter().any(|argument| argument == "--dry-run") {
        return dry_run_command(args).map_err(SpawnError::from);
    }

    let arguments = parse_spawn_arguments(args)?;
    spawn_team(arguments.spec_path.as_deref(), arguments.agents.as_deref())
}

pub fn spawn_team(spec_path: Option<&Path>, agents: Option<&str>) -> Result<(), SpawnError> {
    let current_dir = env::current_dir().map_err(|source| SpawnError::Io {
        action: "read current directory",
        path: PathBuf::from("."),
        source,
    })?;
    let context = load_context(spec_path, agents, &current_dir)?;
    let herdr = HerdrClient::from_env();
    let run = spawn_resolved(
        context.spec,
        &context.launchers,
        &context.state_dir,
        context.god_pane_id,
        &herdr,
        command_exists,
    )?;
    println!("team run created: {}", run.dir.display());
    Ok(())
}

fn parse_spawn_arguments(args: &[String]) -> Result<SpawnArguments, SpawnError> {
    let mut spec_path = None;
    let mut agents = None;
    let mut index = 0;

    while index < args.len() {
        let argument = &args[index];
        match argument.as_str() {
            "--agents" => {
                index += 1;
                let value = args.get(index).ok_or_else(|| {
                    SpawnError::Arguments("--agents requires a comma-separated value".to_owned())
                })?;
                set_agents(&mut agents, value)?;
            }
            value if value.starts_with("--agents=") => {
                set_agents(&mut agents, &value["--agents=".len()..])?;
            }
            value if value.starts_with('-') => {
                return Err(SpawnError::Arguments(format!("unknown option '{value}'")));
            }
            value => {
                if spec_path.replace(PathBuf::from(value)).is_some() {
                    return Err(SpawnError::Arguments(
                        "only one team spec path may be supplied".to_owned(),
                    ));
                }
            }
        }
        index += 1;
    }

    if agents.is_some() && spec_path.is_some() {
        return Err(SpawnError::Arguments(
            "--agents and a team spec path are mutually exclusive".to_owned(),
        ));
    }
    Ok(SpawnArguments { spec_path, agents })
}

fn set_agents(slot: &mut Option<String>, value: &str) -> Result<(), SpawnError> {
    if slot.replace(value.to_owned()).is_some() {
        return Err(SpawnError::Arguments(
            "--agents may only be supplied once".to_owned(),
        ));
    }
    Ok(())
}

fn load_context(
    spec_path: Option<&Path>,
    agents: Option<&str>,
    current_dir: &Path,
) -> Result<SpawnContext, SpawnError> {
    if spec_path.is_some() && agents.is_some() {
        return Err(SpawnError::Arguments(
            "--agents and a team spec path are mutually exclusive".to_owned(),
        ));
    }

    let launchers = load_from_env()?;

    let (mut spec, spec_base) = match agents {
        Some(agents) => (
            team_spec_from_agents(agents, current_dir, &launchers)?,
            current_dir.to_path_buf(),
        ),
        None => {
            let path = spec_path.unwrap_or_else(|| Path::new("herdr-team.toml"));
            let absolute_path = absolutize(path, current_dir);
            let base = absolute_path.parent().unwrap_or(current_dir).to_path_buf();
            (load_team_spec(&absolute_path, &launchers)?, base)
        }
    };
    resolve_paths(&mut spec, &spec_base)?;

    let state_dir = env::var_os("HERDR_PLUGIN_STATE_DIR")
        .map(|path| absolutize(Path::new(&path), current_dir))
        .ok_or(SpawnError::MissingEnvironment("HERDR_PLUGIN_STATE_DIR"))?;
    let god_pane_id = if spec.god.target == "self" {
        env::var("HERDR_PANE_ID").map_err(|_| SpawnError::MissingEnvironment("HERDR_PANE_ID"))?
    } else {
        spec.god.target.clone()
    };

    Ok(SpawnContext {
        spec,
        launchers,
        state_dir,
        god_pane_id,
    })
}

fn resolve_paths(spec: &mut TeamSpec, spec_base: &Path) -> Result<(), SpawnError> {
    let unresolved_cwd = absolutize(&spec.cwd, spec_base);
    spec.cwd = fs::canonicalize(&unresolved_cwd).map_err(|source| SpawnError::TeamCwd {
        path: unresolved_cwd,
        source,
    })?;

    for worker in &mut spec.workers {
        let unresolved_brief = absolutize(&worker.brief, &spec.cwd);
        worker.brief = fs::canonicalize(&unresolved_brief).map_err(|source| SpawnError::Brief {
            worker: worker.name.clone(),
            path: unresolved_brief,
            source,
        })?;
    }
    Ok(())
}

fn absolutize(path: &Path, base: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn spawn_resolved<H, F>(
    spec: TeamSpec,
    launchers: &LauncherTable,
    state_dir: &Path,
    god_pane_id: String,
    herdr: &H,
    command_available: F,
) -> Result<RunBoard, SpawnError>
where
    H: HerdrApi,
    F: Fn(&str) -> bool,
{
    spawn_resolved_with_setup(
        spec,
        launchers,
        state_dir,
        god_pane_id,
        herdr,
        &command_available,
        &ProcessSetupRunner,
    )
}

#[allow(clippy::too_many_arguments)]
fn spawn_resolved_with_setup<H, F, S>(
    spec: TeamSpec,
    launchers: &LauncherTable,
    state_dir: &Path,
    god_pane_id: String,
    herdr: &H,
    command_available: &F,
    setup_runner: &S,
) -> Result<RunBoard, SpawnError>
where
    H: HerdrApi,
    F: Fn(&str) -> bool,
    S: SetupRunner,
{
    spawn_resolved_with_setup_and_agent_info_timeout(
        spec,
        launchers,
        state_dir,
        god_pane_id,
        herdr,
        command_available,
        setup_runner,
        AGENT_START_TIMEOUT,
    )
}

#[allow(clippy::too_many_arguments)]
fn spawn_resolved_with_setup_and_agent_info_timeout<H, F, S>(
    spec: TeamSpec,
    launchers: &LauncherTable,
    state_dir: &Path,
    god_pane_id: String,
    herdr: &H,
    command_available: &F,
    setup_runner: &S,
    agent_info_timeout: Duration,
) -> Result<RunBoard, SpawnError>
where
    H: HerdrApi,
    F: Fn(&str) -> bool,
    S: SetupRunner,
{
    preflight(&spec, launchers, herdr, command_available)?;

    let workers = spec
        .workers
        .iter()
        .map(|worker| {
            (
                worker.name.clone(),
                WorkerRunState {
                    task: worker.task.clone(),
                    workspace_id: None,
                    pane_id: None,
                    agent_id: None,
                    agent_session: None,
                    worktree_path: None,
                    adopted: false,
                    lifecycle: WorkerLifecycle::Pending,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let state = RunState {
        spec: spec.clone(),
        god_pane_id,
        herdr_session: current_herdr_session_identity(),
        workers,
        lifecycle: RunLifecycle::Active,
    };
    let mut run = create_run(state_dir, state)?;

    // Allocate every workspace first so immutable mesh protocols can contain
    // the complete set of opaque live workspace IDs.
    for worker in &spec.workers {
        if let Err(error) = allocate_worker_workspace(&spec, worker, &mut run, herdr, setup_runner)
        {
            if let Some(worker_state) = run.state.workers.get_mut(&worker.name) {
                worker_state.lifecycle = WorkerLifecycle::Failed;
            }
            save_run(&run)?;
            return Err(error);
        }
    }

    // Keeping generated files in the run dir leaves an authored repository
    // AGENTS.md intact. create_new in write_worker_protocol makes each snapshot
    // immutable, and no agent is launched until all snapshots exist.
    for worker in &spec.workers {
        write_worker_protocol(&spec, worker, &run)?;
    }

    for worker in &spec.workers {
        if let Err(error) = launch_worker(worker, launchers, &mut run, herdr, agent_info_timeout) {
            if let Some(worker_state) = run.state.workers.get_mut(&worker.name) {
                worker_state.lifecycle = WorkerLifecycle::Failed;
            }
            save_run(&run)?;
            return Err(error);
        }
    }

    Ok(run)
}

fn preflight<H, F>(
    spec: &TeamSpec,
    launchers: &LauncherTable,
    herdr: &H,
    command_available: &F,
) -> Result<(), SpawnError>
where
    H: HerdrApi,
    F: Fn(&str) -> bool,
{
    validate_team_spec(spec, launchers)?;
    if spec.workers.is_empty() {
        return Err(SpawnError::EmptyTeam);
    }
    if !spec.cwd.is_dir() {
        return Err(SpawnError::TeamCwd {
            path: spec.cwd.clone(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "not a directory"),
        });
    }

    for worker in &spec.workers {
        if !is_safe_worker_filename(&worker.name) {
            return Err(SpawnError::UnsafeWorkerName {
                worker: worker.name.clone(),
            });
        }
        if !worker.brief.is_file() {
            return Err(SpawnError::Brief {
                worker: worker.name.clone(),
                path: worker.brief.clone(),
                source: std::io::Error::new(std::io::ErrorKind::NotFound, "not a file"),
            });
        }
        let launcher = launcher_entry(launchers, &worker.agent)?;
        let command = launcher
            .command
            .first()
            .ok_or_else(|| SpawnError::EmptyLauncher {
                worker: worker.name.clone(),
            })?;
        if !command_available(command) {
            return Err(SpawnError::AgentCliMissing {
                worker: worker.name.clone(),
                command: command.clone(),
            });
        }
    }

    // Read-only health check. This is deliberately the final preflight step.
    herdr.health_check()?;
    Ok(())
}

pub(crate) fn is_safe_worker_filename(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !Path::new(name).is_absolute()
        && !name.contains('/')
        && !name.contains('\\')
}

fn allocate_worker_workspace<H: HerdrApi, S: SetupRunner>(
    spec: &TeamSpec,
    worker: &WorkerSpec,
    run: &mut RunBoard,
    herdr: &H,
    setup_runner: &S,
) -> Result<(), SpawnError> {
    let cwd = prepare_worker_cwd(spec, worker, run, herdr, setup_runner)?;
    let workspace = herdr
        .workspace_create(&cwd, &worker.name)
        .map_err(|source| worker_herdr(worker, "workspace create", source))?;
    {
        let state = run
            .state
            .workers
            .get_mut(&worker.name)
            .expect("run state is initialized from the same team spec");
        state.workspace_id = Some(workspace.workspace_id.clone());
        state.pane_id = Some(workspace.pane_id.clone());
    }
    save_run(run)?;
    Ok(())
}

fn prepare_worker_cwd<H: HerdrApi, S: SetupRunner>(
    spec: &TeamSpec,
    worker: &WorkerSpec,
    run: &mut RunBoard,
    herdr: &H,
    setup_runner: &S,
) -> Result<PathBuf, SpawnError> {
    if !worker.worktree {
        return Ok(spec.cwd.clone());
    }

    let branch = worker
        .branch
        .as_deref()
        .expect("validated worktree workers always have a branch");
    let worktree = herdr
        .worktree_create(&spec.cwd, branch)
        .map_err(|source| worker_herdr(worker, "worktree create", source))?;
    run.state
        .workers
        .get_mut(&worker.name)
        .expect("run state is initialized from the same team spec")
        .worktree_path = Some(worktree.path.clone());
    save_run(run)?;

    run_setup_commands(worker, &worktree.path, &spec.setup, setup_runner)?;
    Ok(worktree.path)
}

fn run_setup_commands<S: SetupRunner>(
    worker: &WorkerSpec,
    cwd: &Path,
    commands: &[String],
    setup_runner: &S,
) -> Result<(), SpawnError> {
    for command in commands {
        let output = setup_runner
            .run(cwd, command)
            .map_err(|source| SpawnError::SetupSpawn {
                worker: worker.name.clone(),
                command: command.clone(),
                cwd: cwd.to_path_buf(),
                source,
            })?;
        if !output.success {
            return Err(SpawnError::SetupFailed {
                worker: worker.name.clone(),
                command: command.clone(),
                cwd: cwd.to_path_buf(),
                status: output.status,
                stdout: output.stdout.into_boxed_str(),
                stderr: output.stderr.into_boxed_str(),
            });
        }
    }
    Ok(())
}

fn launch_worker<H: HerdrApi>(
    worker: &WorkerSpec,
    launchers: &LauncherTable,
    run: &mut RunBoard,
    herdr: &H,
    agent_info_timeout: Duration,
) -> Result<(), SpawnError> {
    let pane_id = run
        .state
        .workers
        .get(&worker.name)
        .and_then(|state| state.pane_id.clone())
        .expect("workspace allocation records a root pane before launch");
    let launcher = launcher_entry(launchers, &worker.agent)?;
    herdr
        .pane_run(&pane_id, &shell_join(&launcher.command))
        .map_err(|source| worker_herdr(worker, "agent launch", source))?;
    wait_for(
        herdr,
        worker,
        &pane_id,
        "idle",
        AGENT_START_TIMEOUT,
        "agent startup",
    )?;

    let pane = wait_for_agent_info(herdr, worker, &pane_id, agent_info_timeout)?;
    let state = run
        .state
        .workers
        .get_mut(&worker.name)
        .expect("run state is initialized from the same team spec");
    state.agent_id = pane.agent_id.clone();
    state.agent_session = pane.agent_session.clone();
    save_run(run)?;

    let prompt = launch_prompt(worker, launcher, &worker_protocol_path(run, worker));
    submit_worker_prompt(
        herdr,
        worker,
        &pane.pane_id,
        &prompt,
        launcher.submit_verify,
    )?;

    run.state
        .workers
        .get_mut(&worker.name)
        .expect("run state is initialized from the same team spec")
        .lifecycle = WorkerLifecycle::Running;
    save_run(run)?;
    Ok(())
}

pub(crate) fn write_worker_protocol(
    spec: &TeamSpec,
    worker: &WorkerSpec,
    run: &RunBoard,
) -> Result<(), SpawnError> {
    let protocols_dir = run.dir.join(PROTOCOLS_DIR);
    fs::create_dir_all(&protocols_dir).map_err(|source| SpawnError::Io {
        action: "create generated protocol directory",
        path: protocols_dir,
        source,
    })?;
    let path = worker_protocol_path(run, worker);
    let contents = render_agents_md(spec, worker, &run.state, &run.dir)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|source| SpawnError::Io {
            action: "create immutable generated protocol",
            path: path.clone(),
            source,
        })?;
    file.write_all(contents.as_bytes())
        .map_err(|source| SpawnError::Io {
            action: "write generated protocol",
            path,
            source,
        })
}

pub(crate) fn worker_protocol_path(run: &RunBoard, worker: &WorkerSpec) -> PathBuf {
    run.dir
        .join(PROTOCOLS_DIR)
        .join(format!("{}.md", worker.name))
}

fn launch_prompt(worker: &WorkerSpec, launcher: &LauncherEntry, protocol_path: &Path) -> String {
    match launcher.reads_agents_md {
        AgentsMdMode::Native => format!(
            "Read your brief at {} and execute it fully. The repository's authored AGENTS.md remains in effect. Read the generated team protocol at {}.",
            worker.brief.display(),
            protocol_path.display()
        ),
        AgentsMdMode::Pointer => format!(
            "Read your brief at {} and execute it fully. Read the generated team protocol at {}.",
            worker.brief.display(),
            protocol_path.display()
        ),
    }
}

pub(crate) fn adoption_prompt(
    worker: &WorkerSpec,
    launcher: &LauncherEntry,
    protocol_path: &Path,
    include_brief: bool,
) -> String {
    if include_brief {
        return launch_prompt(worker, launcher, protocol_path);
    }

    match launcher.reads_agents_md {
        AgentsMdMode::Native => format!(
            "The repository's authored AGENTS.md remains in effect. Read the generated team protocol at {}.",
            protocol_path.display()
        ),
        AgentsMdMode::Pointer => format!(
            "Read the generated team protocol at {}.",
            protocol_path.display()
        ),
    }
}

pub(crate) fn submit_worker_prompt<H: HerdrApi>(
    herdr: &H,
    worker: &WorkerSpec,
    pane_id: &str,
    prompt: &str,
    verify: bool,
) -> Result<(), SpawnError> {
    submit_prompt(
        herdr,
        worker,
        pane_id,
        prompt,
        verify,
        SUBMIT_GRACE_TIMEOUT,
        SUBMIT_VERIFY_TIMEOUT,
    )
}

#[allow(clippy::too_many_arguments)]
fn submit_prompt<H: HerdrApi>(
    herdr: &H,
    worker: &WorkerSpec,
    pane_id: &str,
    prompt: &str,
    verify: bool,
    grace_timeout: Duration,
    verify_timeout: Duration,
) -> Result<(), SpawnError> {
    herdr
        .pane_run(pane_id, prompt)
        .map_err(|source| worker_herdr(worker, "brief injection", source))?;
    if !verify {
        return Ok(());
    }

    match wait_for(
        herdr,
        worker,
        pane_id,
        "working",
        grace_timeout,
        "submission verification",
    ) {
        Ok(()) => Ok(()),
        Err(SpawnError::WorkerTimeout { .. }) => {
            // Some TUIs accept the pasted pointer but swallow pane-run's Enter.
            // An empty pane-run submits the existing composer without duplicating it.
            herdr
                .pane_run(pane_id, "")
                .map_err(|source| worker_herdr(worker, "submission fallback", source))?;
            wait_for(
                herdr,
                worker,
                pane_id,
                "working",
                verify_timeout,
                "submission verification",
            )
        }
        Err(error) => Err(error),
    }
}

fn wait_for<H: HerdrApi>(
    herdr: &H,
    worker: &WorkerSpec,
    pane_id: &str,
    status: &'static str,
    timeout: Duration,
    step: &'static str,
) -> Result<(), SpawnError> {
    let started = Instant::now();
    loop {
        let remaining = timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err(SpawnError::WorkerTimeout {
                worker: worker.name.clone(),
                status,
                step,
            });
        }

        match herdr
            .agent_wait(pane_id, status, remaining)
            .map_err(|source| worker_herdr(worker, step, source))?
        {
            WaitOutcome::Reached => return Ok(()),
            WaitOutcome::TimedOut if started.elapsed() < timeout => {
                // Immediately after `pane run`, Herdr may not yet resolve the
                // pane as an agent target and exits 1 without waiting.
                thread::sleep(Duration::from_millis(100).min(remaining));
            }
            WaitOutcome::TimedOut => {
                return Err(SpawnError::WorkerTimeout {
                    worker: worker.name.clone(),
                    status,
                    step,
                });
            }
        }
    }
}

fn wait_for_agent_info<H: HerdrApi>(
    herdr: &H,
    worker: &WorkerSpec,
    pane_id: &str,
    timeout: Duration,
) -> Result<PaneInfo, SpawnError> {
    let started = Instant::now();
    loop {
        let pane = herdr
            .pane_get(pane_id)
            .map_err(|source| worker_herdr(worker, "agent detection", source))?;
        if pane.agent.is_some() && pane.agent_id.is_some() {
            return Ok(pane);
        }

        let remaining = timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            if pane.agent.is_none() {
                return Err(SpawnError::AgentNotDetected {
                    worker: worker.name.clone(),
                    pane_id: pane_id.to_owned(),
                });
            }
            eprintln!("{}", missing_agent_id_warning(worker, pane_id));
            return Ok(pane);
        }
        thread::sleep(Duration::from_millis(100).min(remaining));
    }
}

fn missing_agent_id_warning(worker: &WorkerSpec, pane_id: &str) -> String {
    format!(
        "warning: worker '{}' launched in pane '{}', but Herdr did not return agent_session.value before timeout; continuing without agent id",
        worker.name, pane_id
    )
}

fn worker_herdr(worker: &WorkerSpec, step: &'static str, source: HerdrError) -> SpawnError {
    SpawnError::WorkerHerdr {
        worker: worker.name.clone(),
        step,
        source,
    }
}

fn shell_join(command: &[String]) -> String {
    command
        .iter()
        .map(|argument| shell_quote(argument))
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote(argument: &str) -> String {
    format!("'{}'", argument.replace('\'', "'\"'\"'"))
}

fn command_exists(command: &str) -> bool {
    let command_path = Path::new(command);
    if command_path.components().count() > 1 {
        return is_executable(command_path);
    }

    env::var_os("PATH")
        .map(|path| {
            env::split_paths(&path).any(|directory| is_executable(&directory.join(command)))
        })
        .unwrap_or(false)
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::herdr::test_support::FakeHerdr;
    use crate::launcher::default_launcher_table;
    use crate::run::load_run;
    use crate::types::{GodSpec, Topology};
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

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
                "herdr-spawn-tests-{}-{nanos}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("create spawn test directory");
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

    #[derive(Default)]
    struct FakeSetupRunner {
        calls: RefCell<Vec<String>>,
        failure: RefCell<Option<(String, SetupOutput)>>,
    }

    impl FakeSetupRunner {
        fn fail(&self, command: &str, status: i32, stdout: &str, stderr: &str) {
            *self.failure.borrow_mut() = Some((
                command.to_owned(),
                SetupOutput {
                    success: false,
                    status: Some(status),
                    stdout: stdout.to_owned(),
                    stderr: stderr.to_owned(),
                },
            ));
        }

        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
    }

    impl SetupRunner for FakeSetupRunner {
        fn run(&self, cwd: &Path, command: &str) -> Result<SetupOutput, std::io::Error> {
            self.calls
                .borrow_mut()
                .push(format!("setup:{command}:{}", cwd.display()));
            if let Some((failed_command, output)) = self.failure.borrow().as_ref() {
                if failed_command == command {
                    return Ok(SetupOutput {
                        success: output.success,
                        status: output.status,
                        stdout: output.stdout.clone(),
                        stderr: output.stderr.clone(),
                    });
                }
            }
            Ok(SetupOutput {
                success: true,
                status: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }

    fn launchers() -> LauncherTable {
        default_launcher_table()
    }

    fn command_is_available(_: &str) -> bool {
        true
    }

    fn worker(root: &Path, name: &str, agent: &str) -> WorkerSpec {
        let brief = root.join(format!("{name}.md"));
        fs::write(&brief, format!("brief for {name}")).expect("write worker brief");
        WorkerSpec {
            name: name.to_owned(),
            agent: agent.to_owned(),
            role: "reviewer".to_owned(),
            task: None,
            worktree: false,
            branch: None,
            brief,
        }
    }

    fn team(root: &Path, workers: Vec<WorkerSpec>) -> TeamSpec {
        TeamSpec {
            name: "spawn-test".to_owned(),
            topology: Topology::Star,
            cwd: root.to_path_buf(),
            setup: Vec::new(),
            god: GodSpec::default(),
            workers,
        }
    }

    fn only_run_dir(state_dir: &Path) -> PathBuf {
        let entries = fs::read_dir(state_dir.join("runs"))
            .expect("read runs")
            .collect::<Result<Vec<_>, _>>()
            .expect("read run entries");
        assert_eq!(entries.len(), 1);
        entries[0].path()
    }

    fn read_protocol_snapshot(state_dir: &Path) -> BTreeMap<PathBuf, String> {
        let protocols_dir = only_run_dir(state_dir).join(PROTOCOLS_DIR);
        fs::read_dir(protocols_dir)
            .expect("read generated protocols")
            .map(|entry| {
                let path = entry.expect("read protocol entry").path();
                let contents = fs::read_to_string(&path).expect("read generated protocol");
                (path, contents)
            })
            .collect()
    }

    #[test]
    fn parses_real_spawn_arguments_without_changing_the_fixed_seam() {
        assert_eq!(
            parse_spawn_arguments(&["--agents=claude,codex".to_owned()]).unwrap(),
            SpawnArguments {
                spec_path: None,
                agents: Some("claude,codex".to_owned()),
            }
        );
        assert_eq!(
            parse_spawn_arguments(&["team.toml".to_owned()]).unwrap(),
            SpawnArguments {
                spec_path: Some(PathBuf::from("team.toml")),
                agents: None,
            }
        );
    }

    #[test]
    fn preflight_failure_performs_no_herdr_mutation_and_names_worker_and_cli() {
        let temp = TempDir::new();
        let fake = FakeHerdr::default();
        let spec = team(
            temp.path(),
            vec![worker(temp.path(), "missing-cli-worker", "claude")],
        );

        let error = spawn_resolved(
            spec,
            &launchers(),
            &temp.path().join("state"),
            "god-pane".to_owned(),
            &fake,
            |_| false,
        )
        .expect_err("missing CLI must fail preflight")
        .to_string();

        assert!(error.contains("missing-cli-worker"));
        assert!(error.contains("claude"));
        assert!(fake.calls().is_empty());
        assert!(!temp.path().join("state/runs").exists());
    }

    #[test]
    fn worktree_workers_are_prepared_before_launch_with_persisted_path_and_cwd() {
        let temp = TempDir::new();
        let state_dir = temp.path().join("state");
        let fake = FakeHerdr::default();
        let setup = FakeSetupRunner::default();
        let mut builder = worker(temp.path(), "builder", "claude");
        builder.role = "builder".to_owned();
        builder.worktree = true;
        builder.branch = Some("feat/builder".to_owned());
        let reviewer = worker(temp.path(), "reviewer", "codex");
        let mut spec = team(temp.path(), vec![builder, reviewer]);
        spec.setup = vec!["./setup-one".to_owned(), "./setup-two --flag".to_owned()];

        let run = spawn_resolved_with_setup(
            spec,
            &launchers(),
            &state_dir,
            "god-pane".to_owned(),
            &fake,
            &command_is_available,
            &setup,
        )
        .expect("spawn mixed-cwd workers");

        let worktree_path = temp.path().join("worktree-1");
        let persisted = load_run(&run.dir).expect("load persisted worktree run");
        assert_eq!(
            persisted.state.workers["builder"].worktree_path.as_deref(),
            Some(worktree_path.as_path())
        );
        assert_eq!(persisted.state.workers["reviewer"].worktree_path, None);
        assert_eq!(
            setup.calls(),
            [
                format!("setup:./setup-one:{}", worktree_path.display()),
                format!("setup:./setup-two --flag:{}", worktree_path.display()),
            ]
        );

        let calls = fake.calls();
        assert!(calls.contains(&format!(
            "worktree_create:feat/builder:{}",
            temp.path().display()
        )));
        assert!(calls.contains(&format!(
            "workspace_create:builder:{}",
            worktree_path.display()
        )));
        assert!(calls.contains(&format!(
            "workspace_create:reviewer:{}",
            temp.path().display()
        )));
        let last_allocation = calls
            .iter()
            .rposition(|call| call.starts_with("workspace_create:"))
            .expect("workspace allocation call");
        let first_launch = calls
            .iter()
            .position(|call| call.starts_with("pane_run:"))
            .expect("agent launch call");
        assert!(
            last_allocation < first_launch,
            "all workers must be allocated before any agent launch: {calls:?}"
        );
    }

    #[test]
    fn setup_failure_captures_output_and_aborts_before_workspace_or_launch() {
        let temp = TempDir::new();
        let state_dir = temp.path().join("state");
        let fake = FakeHerdr::default();
        let setup = FakeSetupRunner::default();
        setup.fail("./prepare", 23, "setup stdout\n", "setup stderr\n");
        let mut builder = worker(temp.path(), "builder", "claude");
        builder.role = "builder".to_owned();
        builder.worktree = true;
        builder.branch = Some("feat/builder".to_owned());
        let mut spec = team(temp.path(), vec![builder]);
        spec.setup = vec!["./prepare".to_owned()];

        let error = spawn_resolved_with_setup(
            spec,
            &launchers(),
            &state_dir,
            "god-pane".to_owned(),
            &fake,
            &command_is_available,
            &setup,
        )
        .expect_err("failed setup must abort allocation")
        .to_string();

        assert!(error.contains("worker 'builder'"));
        assert!(error.contains("./prepare"));
        assert!(error.contains("status Some(23)"));
        assert!(error.contains("setup stdout"));
        assert!(error.contains("setup stderr"));
        assert!(!fake
            .calls()
            .iter()
            .any(|call| call.starts_with("workspace_create:") || call.starts_with("pane_run:")));

        let run = load_run(&only_run_dir(&state_dir)).expect("load failed setup run");
        assert_eq!(
            run.state.workers["builder"].worktree_path.as_deref(),
            Some(temp.path().join("worktree-1").as_path())
        );
        assert_eq!(
            run.state.workers["builder"].lifecycle,
            WorkerLifecycle::Failed
        );
    }

    #[test]
    fn process_setup_runner_uses_worktree_cwd_and_captures_both_streams() {
        let temp = TempDir::new();

        let output = ProcessSetupRunner
            .run(
                temp.path(),
                "printf 'stdout:%s' \"$PWD\"; printf 'stderr:evidence' >&2; exit 19",
            )
            .expect("run setup process");

        assert!(!output.success);
        assert_eq!(output.status, Some(19));
        assert_eq!(output.stdout, format!("stdout:{}", temp.path().display()));
        assert_eq!(output.stderr, "stderr:evidence");
    }

    #[test]
    fn worktree_create_failure_names_worker_and_prevents_all_agent_launches() {
        let temp = TempDir::new();
        let state_dir = temp.path().join("state");
        let fake = FakeHerdr::default();
        *fake.fail_worktree_branch.borrow_mut() = Some("feat/builder".to_owned());
        let setup = FakeSetupRunner::default();
        let reviewer = worker(temp.path(), "reviewer", "codex");
        let mut builder = worker(temp.path(), "builder", "claude");
        builder.role = "builder".to_owned();
        builder.worktree = true;
        builder.branch = Some("feat/builder".to_owned());

        let error = spawn_resolved_with_setup(
            team(temp.path(), vec![reviewer, builder]),
            &launchers(),
            &state_dir,
            "god-pane".to_owned(),
            &fake,
            &command_is_available,
            &setup,
        )
        .expect_err("worktree creation must fail allocation")
        .to_string();

        assert!(error.contains("worker 'builder'"));
        assert!(error.contains("worktree create"));
        assert!(!fake
            .calls()
            .iter()
            .any(|call| call.starts_with("pane_run:")));
        let run = load_run(&only_run_dir(&state_dir)).expect("load failed worktree run");
        assert_eq!(
            run.state.workers["reviewer"].workspace_id.as_deref(),
            Some("workspace-1")
        );
        assert_eq!(
            run.state.workers["builder"].lifecycle,
            WorkerLifecycle::Failed
        );
    }

    #[test]
    fn unreachable_herdr_fails_preflight_before_workspace_mutation() {
        let temp = TempDir::new();
        let fake = FakeHerdr::default();
        fake.fail_health.set(true);
        let spec = team(
            temp.path(),
            vec![worker(temp.path(), "claude-worker", "claude")],
        );

        let error = spawn_resolved(
            spec,
            &launchers(),
            &temp.path().join("state"),
            "god-pane".to_owned(),
            &fake,
            |_| true,
        )
        .expect_err("unreachable Herdr must fail")
        .to_string();

        assert!(error.contains("fake pane run"));
        assert_eq!(fake.calls(), ["health_check"]);
        assert!(!temp.path().join("state/runs").exists());
    }

    #[test]
    fn early_unresolved_agent_wait_is_retried_within_the_deadline() {
        let temp = TempDir::new();
        let fake = FakeHerdr::default();
        fake.wait_timeouts.set(1);
        let worker = worker(temp.path(), "claude-worker", "claude");

        wait_for(
            &fake,
            &worker,
            "pane-1",
            "idle",
            Duration::from_millis(500),
            "agent startup",
        )
        .expect("second wait should observe the agent");

        assert_eq!(
            fake.calls(),
            ["agent_wait:pane-1:idle", "agent_wait:pane-1:idle"]
        );
    }

    #[test]
    fn swallowed_submit_uses_empty_pane_run_without_duplicating_prompt() {
        let temp = TempDir::new();
        let fake = FakeHerdr::default();
        fake.require_empty_submit.set(true);
        let worker = worker(temp.path(), "claude-worker", "claude");

        submit_prompt(
            &fake,
            &worker,
            "pane-1",
            "Read the brief pointer.",
            true,
            Duration::from_millis(1),
            Duration::from_millis(100),
        )
        .expect("empty pane run should submit the existing composer");

        let calls = fake.calls();
        assert_eq!(calls[0], "pane_run:pane-1:Read the brief pointer.");
        assert!(calls.iter().any(|call| call == "pane_run:pane-1:"));
        assert_eq!(
            calls
                .iter()
                .filter(|call| call.contains("Read the brief pointer."))
                .count(),
            1,
            "the fallback must not duplicate the prompt"
        );
        assert_eq!(calls.last().unwrap(), "agent_wait:pane-1:working");
    }

    #[test]
    fn successful_worker_uses_root_pane_and_persists_returned_ids() {
        let temp = TempDir::new();
        let state_dir = temp.path().join("state");
        let fake = FakeHerdr::default();
        *fake.protocols_state_dir.borrow_mut() = Some(state_dir.clone());
        let spec = team(
            temp.path(),
            vec![worker(temp.path(), "claude-worker", "claude")],
        );

        let run = spawn_resolved(
            spec,
            &launchers(),
            &state_dir,
            "god-pane".to_owned(),
            &fake,
            |_| true,
        )
        .expect("spawn worker");

        let persisted = load_run(&run.dir).expect("load persisted run");
        let worker = &persisted.state.workers["claude-worker"];
        assert_eq!(worker.workspace_id.as_deref(), Some("workspace-1"));
        assert_eq!(worker.pane_id.as_deref(), Some("pane-1"));
        assert_eq!(
            worker.agent_id.as_deref(),
            Some("agent-session-pane-1"),
            "persist the opaque ID returned by the typed client"
        );
        assert_eq!(worker.lifecycle, WorkerLifecycle::Running);
        assert_eq!(
            worker
                .agent_session
                .as_ref()
                .map(|session| session.source.as_str()),
            Some("herdr:test"),
            "persist the complete agent-session reference"
        );
        let calls = fake.calls();
        assert_eq!(calls[0], "health_check");
        assert!(calls.iter().any(|call| call == "pane_run:pane-1:'claude'"));
        assert!(!calls.iter().any(|call| call.contains("pane_split")));
        assert!(calls.iter().any(|call| {
            call.starts_with("pane_run:pane-1:Read your brief at ")
                && call.contains("Read the generated team protocol at")
        }));
        assert!(calls.iter().any(|call| call == "agent_wait:pane-1:working"));
    }

    #[test]
    fn partial_failure_keeps_first_worker_recorded_and_marks_second_failed() {
        let temp = TempDir::new();
        let state_dir = temp.path().join("state");
        let fake = FakeHerdr::default();
        *fake.fail_launch_pane.borrow_mut() = Some("pane-2".to_owned());
        let spec = team(
            temp.path(),
            vec![
                worker(temp.path(), "first", "claude"),
                worker(temp.path(), "second", "codex"),
            ],
        );

        let error = spawn_resolved(
            spec,
            &launchers(),
            &state_dir,
            "god-pane".to_owned(),
            &fake,
            |_| true,
        )
        .expect_err("second launch should fail")
        .to_string();

        assert!(error.contains("worker 'second'"));
        assert!(error.contains("agent launch"));
        let run = load_run(&only_run_dir(&state_dir)).expect("load partial run");
        assert_eq!(
            run.state.workers["first"].lifecycle,
            WorkerLifecycle::Running
        );
        assert_eq!(
            run.state.workers["first"].workspace_id.as_deref(),
            Some("workspace-1")
        );
        assert_eq!(
            run.state.workers["second"].lifecycle,
            WorkerLifecycle::Failed
        );
        assert_eq!(
            run.state.workers["second"].workspace_id.as_deref(),
            Some("workspace-2")
        );
        assert!(!fake.calls().iter().any(|call| call.contains("close")));
    }

    #[test]
    fn detected_agent_without_session_id_warns_and_later_workers_still_launch() {
        let temp = TempDir::new();
        let state_dir = temp.path().join("state");
        let fake = FakeHerdr::default();
        fake.omit_agent_id.set(true);
        let setup = FakeSetupRunner::default();
        let spec = team(
            temp.path(),
            vec![
                worker(temp.path(), "first", "claude"),
                worker(temp.path(), "second", "codex"),
            ],
        );

        let run = spawn_resolved_with_setup_and_agent_info_timeout(
            spec,
            &launchers(),
            &state_dir,
            "god-pane".to_owned(),
            &fake,
            &command_is_available,
            &setup,
            Duration::ZERO,
        )
        .expect("missing agent ids should degrade without aborting the team");

        let persisted = load_run(&run.dir).expect("load degraded run");
        assert_eq!(persisted.state.workers["first"].agent_id, None);
        assert_eq!(persisted.state.workers["second"].agent_id, None);
        assert_eq!(
            persisted.state.workers["first"].lifecycle,
            WorkerLifecycle::Running
        );
        assert_eq!(
            persisted.state.workers["second"].lifecycle,
            WorkerLifecycle::Running
        );
        assert!(fake
            .calls()
            .iter()
            .any(|call| call == "pane_run:pane-2:'codex'"));

        let warning = missing_agent_id_warning(&run.state.spec.workers[0], "pane-1");
        assert!(warning.contains("worker 'first'"));
        assert!(warning.contains("pane 'pane-1'"));
        assert!(warning.contains("continuing without agent id"));
        assert!(!warning.contains('\n'), "warning must remain one line");
        assert_eq!(AGENT_START_TIMEOUT, Duration::from_secs(90));
    }

    #[test]
    fn no_agent_at_timeout_remains_a_hard_per_worker_failure() {
        let temp = TempDir::new();
        let state_dir = temp.path().join("state");
        let fake = FakeHerdr::default();
        fake.omit_agent.set(true);
        let setup = FakeSetupRunner::default();

        let error = spawn_resolved_with_setup_and_agent_info_timeout(
            team(
                temp.path(),
                vec![worker(temp.path(), "missing-agent", "claude")],
            ),
            &launchers(),
            &state_dir,
            "god-pane".to_owned(),
            &fake,
            &command_is_available,
            &setup,
            Duration::ZERO,
        )
        .expect_err("undetected agent must fail the worker");

        assert!(matches!(
            error,
            SpawnError::AgentNotDetected {
                ref worker,
                ref pane_id,
            } if worker == "missing-agent" && pane_id == "pane-1"
        ));
        let run = load_run(&only_run_dir(&state_dir)).expect("load failed agent run");
        assert_eq!(
            run.state.workers["missing-agent"].lifecycle,
            WorkerLifecycle::Failed
        );
        assert_eq!(run.state.workers["missing-agent"].agent_id, None);
    }

    #[test]
    fn delayed_agent_session_id_is_retried_and_returned_verbatim() {
        let temp = TempDir::new();
        let fake = FakeHerdr::default();
        fake.agent_id_delays.set(1);
        let worker = worker(temp.path(), "claude-worker", "claude");

        let pane = wait_for_agent_info(&fake, &worker, "pane-1", Duration::from_millis(500))
            .expect("second pane read should include the opaque ID");

        assert_eq!(pane.agent_id.as_deref(), Some("agent-session-pane-1"));
        assert_eq!(fake.calls(), ["pane_get:pane-1", "pane_get:pane-1"]);
    }

    #[test]
    fn shell_join_preserves_argument_boundaries() {
        assert_eq!(
            shell_join(&[
                "claude".to_owned(),
                "--name".to_owned(),
                "O'Brien worker".to_owned(),
            ]),
            "'claude' '--name' 'O'\"'\"'Brien worker'"
        );
    }

    #[test]
    fn shared_cwd_workers_get_distinct_protocols_without_overwriting_authored_agents_md() {
        let temp = TempDir::new();
        let state_dir = temp.path().join("state");
        let authored_agents = temp.path().join("AGENTS.md");
        fs::write(&authored_agents, "# Authored repository instructions\n")
            .expect("write authored AGENTS.md");
        let fake = FakeHerdr::default();
        let spec = team(
            temp.path(),
            vec![
                worker(temp.path(), "pointer-worker", "claude"),
                worker(temp.path(), "native-worker", "codex"),
            ],
        );

        let run = spawn_resolved(
            spec,
            &launchers(),
            &state_dir,
            "god-pane".to_owned(),
            &fake,
            |_| true,
        )
        .expect("spawn shared-cwd workers");

        assert_eq!(
            fs::read_to_string(authored_agents).expect("read authored AGENTS.md"),
            "# Authored repository instructions\n"
        );

        let marker = "Read the generated team protocol at ";
        let protocol_paths = fake
            .calls()
            .into_iter()
            .filter_map(|call| {
                let (_, path) = call.split_once(marker)?;
                Some(PathBuf::from(path.strip_suffix('.').unwrap_or(path)))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            protocol_paths.len(),
            2,
            "every launcher mode needs a pointer"
        );
        assert_ne!(protocol_paths[0], protocol_paths[1]);
        let running_binary = env::current_exe().expect("resolve test executable");

        for (path, worker_name) in protocol_paths
            .iter()
            .zip(["pointer-worker", "native-worker"])
        {
            assert!(path.is_absolute());
            assert!(path.starts_with(&run.dir));
            let protocol = fs::read_to_string(path).expect("read generated protocol");
            assert!(protocol.contains(&format!("- Worker: `{worker_name}`")));
            assert!(protocol.contains(&format!("/inbox/{worker_name}.md")));
            assert!(protocol.contains(&running_binary.display().to_string()));
            assert!(protocol.contains(&format!("--run '{}'", run.dir.display())));
            let other = if worker_name == "pointer-worker" {
                "native-worker"
            } else {
                "pointer-worker"
            };
            assert!(!protocol.contains(&format!("- Worker: `{other}`")));
            assert!(!protocol.contains(&format!("/inbox/{other}.md")));
        }
    }

    #[test]
    fn per_worker_protocols_remain_unchanged_across_shared_cwd_launches() {
        let temp = TempDir::new();
        let state_dir = temp.path().join("state");
        let fake = FakeHerdr::default();
        *fake.protocols_state_dir.borrow_mut() = Some(state_dir.clone());
        let spec = team(
            temp.path(),
            vec![
                worker(temp.path(), "first", "claude"),
                worker(temp.path(), "second", "codex"),
            ],
        );

        spawn_resolved(
            spec,
            &launchers(),
            &state_dir,
            "god-pane".to_owned(),
            &fake,
            |_| true,
        )
        .expect("spawn shared-cwd workers");

        let snapshots = fake.protocol_snapshots();
        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[0], snapshots[1]);
        assert_eq!(snapshots[0], read_protocol_snapshot(&state_dir));
    }

    #[test]
    fn mesh_protocols_are_immutable_snapshots_of_all_allocated_workspaces() {
        let temp = TempDir::new();
        let state_dir = temp.path().join("state");
        let fake = FakeHerdr::default();
        let mut spec = team(
            temp.path(),
            vec![
                worker(temp.path(), "first", "claude"),
                worker(temp.path(), "second", "codex"),
            ],
        );
        spec.topology = Topology::Mesh;

        let run = spawn_resolved(
            spec,
            &launchers(),
            &state_dir,
            "god-pane".to_owned(),
            &fake,
            |_| true,
        )
        .expect("spawn mesh workers");

        let first = fs::read_to_string(worker_protocol_path(&run, &run.state.spec.workers[0]))
            .expect("read first protocol");
        let second = fs::read_to_string(worker_protocol_path(&run, &run.state.spec.workers[1]))
            .expect("read second protocol");

        assert!(first.contains("- Workspace: `workspace-1`"));
        assert!(first.contains("| `second` |"));
        assert!(second.contains("- Workspace: `workspace-2`"));
        assert!(second.contains("| `first` |"));
        assert!(!first.contains("`pending`"));
        assert!(!second.contains("`pending`"));
    }

    #[test]
    fn unsafe_worker_filename_components_fail_preflight_before_herdr() {
        for (index, unsafe_name) in [
            "",
            ".",
            "..",
            "/absolute",
            "nested/worker",
            "nested\\worker",
        ]
        .into_iter()
        .enumerate()
        {
            let temp = TempDir::new();
            let fake = FakeHerdr::default();
            let mut unsafe_worker = worker(temp.path(), &format!("safe-{index}"), "claude");
            unsafe_worker.name = unsafe_name.to_owned();
            let spec = team(temp.path(), vec![unsafe_worker]);

            let error = preflight(&spec, &launchers(), &fake, &|_| true)
                .expect_err("unsafe protocol filename must fail preflight")
                .to_string();

            assert!(error.contains("safe single filename component"));
            assert!(
                fake.calls().is_empty(),
                "unsafe name reached Herdr: {unsafe_name:?}"
            );
        }
    }
}
