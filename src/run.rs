//! Durable run-board storage and matching from `docs/spec.md` sections 4 through 6.

use crate::reconcile::HookMetadata;
use crate::types::{RunLifecycle, RunState};
use serde::Deserialize;
use serde_json::Value;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, SystemTimeError, UNIX_EPOCH};
use thiserror::Error;

const RUNS_DIR: &str = "runs";
const RUN_FILE: &str = "run.toml";
const INBOX_DIR: &str = "inbox";
const EVENTS_FILE: &str = "events.jsonl";

#[derive(Debug, Error)]
pub enum RunError {
    #[error("run-board I/O failed: {0}")]
    Io(#[from] io::Error),

    #[error("failed to serialize run.toml: {0}")]
    SerializeToml(#[from] toml::ser::Error),

    #[error("failed to deserialize run.toml: {0}")]
    DeserializeToml(#[from] toml::de::Error),

    #[error("failed to serialize event JSON: {0}")]
    SerializeJson(#[from] serde_json::Error),

    #[error("system clock is before the Unix epoch: {0}")]
    Clock(#[from] SystemTimeError),

    #[error("team name must be one non-empty path component: {0:?}")]
    InvalidTeamName(String),

    #[error("event must be a JSON object")]
    InvalidEvent,

    #[error("could not allocate a unique run directory")]
    RunDirectoryCollision,
}

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

#[derive(Debug, Deserialize)]
struct HookEnvelope {
    #[serde(default)]
    hook: HookMetadata,
}

pub fn create_run(state_dir: &Path, state: RunState) -> Result<RunBoard, RunError> {
    validate_team_name(&state.spec.name)?;

    let runs_dir = state_dir.join(RUNS_DIR);
    fs::create_dir_all(&runs_dir)?;
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let run_dir = allocate_run_dir(&runs_dir, &state.spec.name, timestamp)?;

    if let Err(error) = fs::create_dir(run_dir.join(INBOX_DIR)) {
        let _ = fs::remove_dir(&run_dir);
        return Err(error.into());
    }

    let run = RunBoard {
        dir: run_dir,
        state,
    };
    if let Err(error) = save_run_with_hook(&run, &HookMetadata::default()) {
        let _ = fs::remove_dir_all(&run.dir);
        return Err(error);
    }

    Ok(run)
}

pub fn load_run(run_dir: &Path) -> Result<RunBoard, RunError> {
    let contents = fs::read_to_string(run_dir.join(RUN_FILE))?;
    let state = toml::from_str(&contents)?;
    Ok(RunBoard {
        dir: run_dir.to_path_buf(),
        state,
    })
}

pub fn save_run(run: &RunBoard) -> Result<(), RunError> {
    let hook = load_hook_metadata(&run.dir)?;
    save_run_with_hook(run, &hook)
}

pub fn load_hook_metadata(run_dir: &Path) -> Result<HookMetadata, RunError> {
    let contents = fs::read_to_string(run_dir.join(RUN_FILE))?;
    Ok(toml::from_str::<HookEnvelope>(&contents)?.hook)
}

pub fn save_run_with_hook(run: &RunBoard, hook: &HookMetadata) -> Result<(), RunError> {
    let mut contents = toml::to_string_pretty(&run.state)?;
    if hook != &HookMetadata::default() {
        if !hook.worker_status.is_empty() {
            contents.push_str("\n[hook.worker_status]\n");
            contents.push_str(&toml::to_string(&hook.worker_status)?);
        }
        if !hook.worker_agent_identity.is_empty() {
            contents.push_str("\n[hook.worker_agent_identity]\n");
            contents.push_str(&toml::to_string(&hook.worker_agent_identity)?);
        }
    }
    write_run_contents(&run.dir, &contents)
}

fn write_run_contents(run_dir: &Path, contents: &str) -> Result<(), RunError> {
    let path = run_dir.join(RUN_FILE);
    let temporary = run_dir.join(format!(".{RUN_FILE}.{}.tmp", std::process::id()));
    let mut file = File::create(&temporary)?;
    file.write_all(contents.as_bytes())?;
    file.flush()?;
    fs::rename(&temporary, &path)?;
    Ok(())
}

pub fn list_active_runs(state_dir: &Path) -> Result<Vec<RunBoard>, RunError> {
    let entries = match fs::read_dir(state_dir.join(RUNS_DIR)) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };

    let mut runs = Vec::new();
    for entry in entries.flatten() {
        let is_directory = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
        if !is_directory {
            continue;
        }

        if let Ok(run) = load_run(&entry.path()) {
            if run.state.lifecycle == RunLifecycle::Active {
                runs.push(run);
            }
        }
    }
    runs.sort_unstable_by(|left, right| left.dir.cmp(&right.dir));
    Ok(runs)
}

pub fn match_pane(state_dir: &Path, pane_id: &str) -> Result<Option<MatchedWorker>, RunError> {
    if pane_id.is_empty() {
        return Ok(None);
    }

    for run in list_active_runs(state_dir)? {
        let worker_name = run.state.workers.iter().find_map(|(worker_name, worker)| {
            (worker.pane_id.as_deref() == Some(pane_id)).then(|| worker_name.clone())
        });
        if let Some(worker_name) = worker_name {
            return Ok(Some(MatchedWorker { run, worker_name }));
        }
    }

    Ok(None)
}

pub fn append_event(run_dir: &Path, event: &Value) -> Result<(), RunError> {
    if !event.is_object() {
        return Err(RunError::InvalidEvent);
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(run_dir.join(INBOX_DIR).join(EVENTS_FILE))?;
    serde_json::to_writer(&mut file, event)?;
    file.write_all(b"\n")?;
    file.flush()?;
    Ok(())
}

pub fn mark_ended(run: &mut RunBoard) -> Result<(), RunError> {
    let previous = run.state.lifecycle;
    run.state.lifecycle = RunLifecycle::Ended;
    if let Err(error) = save_run(run) {
        run.state.lifecycle = previous;
        return Err(error);
    }
    Ok(())
}

fn validate_team_name(team_name: &str) -> Result<(), RunError> {
    let mut components = Path::new(team_name).components();
    let valid = !team_name.is_empty()
        && matches!(components.next(), Some(Component::Normal(_)))
        && components.next().is_none();
    if valid {
        Ok(())
    } else {
        Err(RunError::InvalidTeamName(team_name.to_owned()))
    }
}

fn allocate_run_dir(
    runs_dir: &Path,
    team_name: &str,
    timestamp: u128,
) -> Result<PathBuf, RunError> {
    for offset in 0..1_000_u128 {
        let candidate = runs_dir.join(format!("{team_name}-{}", timestamp + offset));
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(RunError::RunDirectoryCollision)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{GodSpec, TeamSpec, Topology, WorkerLifecycle, WorkerRunState, WorkerSpec};
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicU64, Ordering};

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
                "herdr-run-tests-{}-{nanos}-{sequence}",
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

    fn run_state(team_name: &str, pane_id: &str) -> RunState {
        let worker_name = "builder".to_owned();
        let mut workers = BTreeMap::new();
        workers.insert(
            worker_name.clone(),
            WorkerRunState {
                workspace_id: Some("workspace-1".to_owned()),
                pane_id: Some(pane_id.to_owned()),
                agent_id: Some("agent-1".to_owned()),
                worktree_path: Some(PathBuf::from("/tmp/worktree")),
                adopted: false,
                lifecycle: WorkerLifecycle::Running,
            },
        );
        RunState {
            spec: TeamSpec {
                name: team_name.to_owned(),
                topology: Topology::Star,
                cwd: PathBuf::from("/tmp/project"),
                setup: vec!["cargo check".to_owned()],
                god: GodSpec {
                    target: "self".to_owned(),
                },
                workers: vec![WorkerSpec {
                    name: worker_name,
                    agent: "codex".to_owned(),
                    role: "builder".to_owned(),
                    worktree: true,
                    branch: Some("ticket-06".to_owned()),
                    brief: PathBuf::from("brief.md"),
                }],
            },
            god_pane_id: "god-pane".to_owned(),
            workers,
            lifecycle: RunLifecycle::Active,
        }
    }

    #[test]
    fn create_and_load_round_trip_state_and_layout() {
        let temp = TempDir::new();
        let state = run_state("alpha", "pane-alpha");

        let run = create_run(temp.path(), state.clone()).expect("create run");

        assert_eq!(run.state, state);
        assert_eq!(run.dir.parent(), Some(temp.path().join(RUNS_DIR).as_path()));
        assert!(run
            .dir
            .file_name()
            .expect("run directory name")
            .to_string_lossy()
            .starts_with("alpha-"));
        assert!(run.dir.join(RUN_FILE).is_file());
        assert!(run.dir.join(INBOX_DIR).is_dir());
        assert_eq!(load_run(&run.dir).expect("load run"), run);
    }

    #[test]
    fn create_allocates_distinct_directories_for_same_team() {
        let temp = TempDir::new();

        let first = create_run(temp.path(), run_state("alpha", "pane-1")).expect("first run");
        let second = create_run(temp.path(), run_state("alpha", "pane-2")).expect("second run");

        assert_ne!(first.dir, second.dir);
    }

    #[test]
    fn pane_matching_spans_active_runs_and_skips_corrupt_directories() {
        let temp = TempDir::new();
        create_run(temp.path(), run_state("alpha", "pane-alpha")).expect("alpha run");
        let beta = create_run(temp.path(), run_state("beta", "pane-beta")).expect("beta run");
        let corrupt = temp.path().join(RUNS_DIR).join("corrupt-0");
        fs::create_dir(&corrupt).expect("corrupt run directory");
        fs::write(corrupt.join(RUN_FILE), "not valid toml = [").expect("corrupt run file");

        let matched = match_pane(temp.path(), "pane-beta")
            .expect("match pane")
            .expect("known pane");

        assert_eq!(matched.worker_name, "builder");
        assert_eq!(matched.run, beta);
        assert!(match_pane(temp.path(), "unknown")
            .expect("unknown pane lookup")
            .is_none());
        assert!(match_pane(temp.path(), "")
            .expect("empty pane lookup")
            .is_none());
    }

    #[test]
    fn events_are_appended_as_complete_json_lines() {
        let temp = TempDir::new();
        let run = create_run(temp.path(), run_state("alpha", "pane-alpha")).expect("create run");
        let first = json!({"worker": "builder", "status": "blocked"});
        let second = json!({"worker": "builder", "status": "done"});

        append_event(&run.dir, &first).expect("append first event");
        let events_path = run.dir.join(INBOX_DIR).join(EVENTS_FILE);
        let first_contents = fs::read_to_string(&events_path).expect("read first event");
        append_event(&run.dir, &second).expect("append second event");
        let contents = fs::read_to_string(events_path).expect("read events");

        assert!(contents.starts_with(&first_contents));
        let lines: Vec<_> = contents.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(serde_json::from_str::<Value>(lines[0]).unwrap(), first);
        assert_eq!(serde_json::from_str::<Value>(lines[1]).unwrap(), second);
        assert!(contents.ends_with('\n'));
    }

    #[test]
    fn ending_a_run_is_persisted_and_excludes_it_from_matching() {
        let temp = TempDir::new();
        let mut run =
            create_run(temp.path(), run_state("alpha", "pane-alpha")).expect("create run");

        mark_ended(&mut run).expect("end run");

        assert_eq!(run.state.lifecycle, RunLifecycle::Ended);
        assert_eq!(
            load_run(&run.dir).expect("load ended run").state.lifecycle,
            RunLifecycle::Ended
        );
        assert!(list_active_runs(temp.path())
            .expect("list active runs")
            .is_empty());
        assert!(match_pane(temp.path(), "pane-alpha")
            .expect("match ended pane")
            .is_none());
    }

    #[test]
    fn missing_runs_directory_is_an_empty_board() {
        let temp = TempDir::new();

        assert!(list_active_runs(temp.path())
            .expect("list missing runs directory")
            .is_empty());
    }

    #[test]
    fn legacy_worker_state_without_adopted_defaults_to_owned() {
        let worker: WorkerRunState = toml::from_str(
            r#"
workspace_id = "workspace-legacy"
pane_id = "pane-legacy"
lifecycle = "running"
"#,
        )
        .expect("parse pre-adoption worker state");

        assert!(!worker.adopted);
        assert_eq!(worker.lifecycle, WorkerLifecycle::Running);
    }

    #[test]
    fn invalid_team_names_and_non_object_events_are_rejected() {
        let temp = TempDir::new();

        assert!(matches!(
            create_run(temp.path(), run_state("../escape", "pane")),
            Err(RunError::InvalidTeamName(_))
        ));

        let run = create_run(temp.path(), run_state("alpha", "pane")).expect("create run");
        assert!(matches!(
            append_event(&run.dir, &json!(["not", "an", "object"])),
            Err(RunError::InvalidEvent)
        ));
    }
}
