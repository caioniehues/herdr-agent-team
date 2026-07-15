//! Name-addressed worker messaging and deferred delivery from `docs/spec.md` section 11.

use crate::herdr::{HerdrApi, HerdrClient, HerdrError, WaitOutcome};
use crate::launcher::{
    conservative_adopted_launcher, launcher_entry, load_from_env, LauncherError,
};
use crate::run::{
    list_active_runs, load_hook_metadata, load_run, save_run_with_hook, RunBoard, RunError,
};
use crate::types::{LauncherEntry, LauncherTable, RunLifecycle};
use std::collections::BTreeSet;
use std::env;
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;
use thiserror::Error;

const MSG_USAGE: &str =
    "usage: herdr-agent-team msg <target> <text> [--attention] [--run <run-dir>]";
const SUBMIT_GRACE_TIMEOUT: Duration = Duration::from_secs(2);
const SUBMIT_VERIFY_TIMEOUT: Duration = Duration::from_secs(30);
const OUTBOX_DIR: &str = "outbox";
const SEQUENCE_WIDTH: usize = 20;

#[derive(Debug, Error)]
pub enum MsgError {
    #[error("invalid msg arguments: {0}")]
    Arguments(String),

    #[error("required environment variable {0} is not set")]
    MissingEnvironment(&'static str),

    #[error(transparent)]
    Run(#[from] RunError),

    #[error(transparent)]
    Launcher(#[from] LauncherError),

    #[error(transparent)]
    Herdr(#[from] HerdrError),

    #[error("no active team run found under {state_dir}")]
    NoActiveRun { state_dir: PathBuf },

    #[error("run is not active: {run_dir}")]
    InactiveRun { run_dir: PathBuf },

    #[error("unknown message target `{target}`; candidates: {candidates}")]
    UnknownTarget { target: String, candidates: String },

    #[error("ambiguous message target `{target}`; candidates: {candidates}")]
    AmbiguousTarget { target: String, candidates: String },

    #[error("message target `{target}` has no recorded pane id")]
    MissingPaneId { target: String },

    #[error("message target `{target}` has no detectable agent kind in pane `{pane_id}`")]
    MissingAgentKind { target: String, pane_id: String },

    #[error("message target `{target}` is not a safe outbox path component")]
    UnsafeTarget { target: String },

    #[error("failed to {action} `{path}`: {source}")]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("outbox sequence is exhausted for target `{target}`")]
    SequenceExhausted { target: String },

    #[error("message submission to `{target}` timed out waiting for agent status `working`")]
    SubmissionTimeout { target: String },

    #[error("--attention is only valid for a worker message to god")]
    AttentionTarget,

    #[error("--attention requires HERDR_PANE_ID for a recorded worker pane")]
    AttentionSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MsgArguments {
    target: String,
    text: String,
    run_dir: Option<PathBuf>,
    attention: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedTarget {
    name: String,
    pane_id: String,
    agent: Option<String>,
    adopted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeliveryDecision {
    Deliver,
    Enqueue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MessageOutcome {
    Delivered,
    Enqueued(PathBuf),
}

pub fn msg_command(args: &[String]) -> Result<(), MsgError> {
    let arguments = parse_msg_arguments(args)?;
    if arguments.attention && arguments.target != "god" {
        return Err(MsgError::AttentionTarget);
    }
    let run = select_run(arguments.run_dir.as_deref())?;
    let launchers = load_launchers()?;
    let herdr = HerdrClient::from_env();
    send_message(&run, &launchers, &arguments.target, &arguments.text, &herdr)?;
    if arguments.attention {
        request_attention(&run, &arguments.target, &arguments.text, &herdr)?;
    }
    Ok(())
}

pub(crate) fn deliver_queued_message<H: HerdrApi>(
    run: &RunBoard,
    target_name: &str,
    text: &str,
    herdr: &H,
) -> Result<(), MsgError> {
    let target = resolve_target(run, target_name)?;
    let agent = target
        .agent
        .as_deref()
        .ok_or_else(|| MsgError::MissingAgentKind {
            target: target.name.clone(),
            pane_id: target.pane_id.clone(),
        })?;
    let launchers = load_launchers()?;
    let launcher = target_launcher(&launchers, &target, agent)?;
    deliver_message(herdr, &target, text, &launcher)
}

fn parse_msg_arguments(args: &[String]) -> Result<MsgArguments, MsgError> {
    let mut positional = Vec::new();
    let mut run_dir = None;
    let mut attention = false;
    let mut index = 0;

    while index < args.len() {
        if args[index] == "--attention" {
            if attention {
                return Err(arguments("--attention may only be supplied once"));
            }
            attention = true;
        } else if args[index] == "--run" {
            if run_dir.is_some() {
                return Err(arguments("--run may only be supplied once"));
            }
            index += 1;
            let path = args
                .get(index)
                .ok_or_else(|| arguments("--run requires a run directory"))?;
            run_dir = Some(PathBuf::from(path));
        } else if args[index].starts_with('-') {
            return Err(arguments(format!("unknown option {}", args[index])));
        } else {
            positional.push(args[index].clone());
        }
        index += 1;
    }

    if positional.len() != 2 {
        return Err(arguments(
            "expected exactly one target and one text argument",
        ));
    }

    Ok(MsgArguments {
        target: positional.remove(0),
        text: positional.remove(0),
        run_dir,
        attention,
    })
}

fn request_attention<H: HerdrApi>(
    run: &RunBoard,
    target: &str,
    text: &str,
    herdr: &H,
) -> Result<(), MsgError> {
    if target != "god" {
        return Err(MsgError::AttentionTarget);
    }
    let source_pane = env::var("HERDR_PANE_ID").map_err(|_| MsgError::AttentionSource)?;
    request_attention_from_pane(run, text, &source_pane, herdr)
}

fn request_attention_from_pane<H: HerdrApi>(
    run: &RunBoard,
    text: &str,
    source_pane: &str,
    herdr: &H,
) -> Result<(), MsgError> {
    let worker_name = run
        .state
        .workers
        .iter()
        .find_map(|(name, worker)| {
            (worker.pane_id.as_deref() == Some(source_pane)).then(|| name.clone())
        })
        .ok_or(MsgError::AttentionSource)?;
    let mut metadata = load_hook_metadata(&run.dir)?;
    metadata.attention_pending.insert(worker_name.clone(), true);
    let first = metadata
        .aggregate_notifications
        .insert(format!("attention:{worker_name}"), true)
        .is_none();
    save_run_with_hook(run, &metadata)?;
    if first {
        herdr.notification_show(
            "Worker needs attention",
            &format!("{worker_name}: {}", strip_escape_sequences(text)),
            "request",
        )?;
    }
    Ok(())
}

fn arguments(detail: impl AsRef<str>) -> MsgError {
    MsgError::Arguments(format!("{}; {MSG_USAGE}", detail.as_ref()))
}

fn select_run(run_dir: Option<&Path>) -> Result<RunBoard, MsgError> {
    let run = match run_dir {
        Some(run_dir) => load_run(run_dir)?,
        None => {
            let state_dir = env::var_os("HERDR_PLUGIN_STATE_DIR")
                .map(PathBuf::from)
                .ok_or(MsgError::MissingEnvironment("HERDR_PLUGIN_STATE_DIR"))?;
            newest_active_run(&state_dir)?
        }
    };

    if run.state.lifecycle != RunLifecycle::Active {
        return Err(MsgError::InactiveRun {
            run_dir: run.dir.clone(),
        });
    }
    Ok(run)
}

fn newest_active_run(state_dir: &Path) -> Result<RunBoard, MsgError> {
    list_active_runs(state_dir)?
        .pop()
        .ok_or_else(|| MsgError::NoActiveRun {
            state_dir: state_dir.to_path_buf(),
        })
}

fn load_launchers() -> Result<LauncherTable, MsgError> {
    Ok(load_from_env()?)
}

fn send_message<H: HerdrApi>(
    run: &RunBoard,
    launchers: &LauncherTable,
    target_name: &str,
    text: &str,
    herdr: &H,
) -> Result<MessageOutcome, MsgError> {
    let target = resolve_target(run, target_name)?;
    let mut pane = None;
    let agent = match target.agent.as_deref() {
        Some(agent) => agent,
        None => {
            pane = Some(herdr.pane_get(&target.pane_id)?);
            pane.as_ref()
                .and_then(|pane| pane.agent.as_deref())
                .ok_or_else(|| MsgError::MissingAgentKind {
                    target: target.name.clone(),
                    pane_id: target.pane_id.clone(),
                })?
        }
    };
    let launcher = target_launcher(launchers, &target, agent)?;

    if !launcher.queues_midturn && pane.is_none() {
        pane = Some(herdr.pane_get(&target.pane_id)?);
    }
    let status = pane.as_ref().and_then(|pane| pane.agent_status.as_deref());
    let sanitized = strip_escape_sequences(text);

    match delivery_decision(launcher.queues_midturn, status) {
        DeliveryDecision::Deliver => {
            deliver_message(herdr, &target, &sanitized, &launcher)?;
            Ok(MessageOutcome::Delivered)
        }
        DeliveryDecision::Enqueue => {
            let path = enqueue_message(&run.dir, &target.name, &sanitized)?;
            Ok(MessageOutcome::Enqueued(path))
        }
    }
}

fn resolve_target(run: &RunBoard, target_name: &str) -> Result<ResolvedTarget, MsgError> {
    let mut matches = Vec::new();

    if target_name == "god" {
        matches.push(ResolvedTarget {
            name: "god".to_owned(),
            pane_id: run.state.god_pane_id.clone(),
            agent: None,
            adopted: false,
        });
    }

    if let Some(worker_state) = run.state.workers.get(target_name) {
        let agent = run
            .state
            .spec
            .workers
            .iter()
            .find(|worker| worker.name == target_name)
            .map(|worker| worker.agent.clone());
        matches.push(ResolvedTarget {
            name: target_name.to_owned(),
            pane_id: worker_state
                .pane_id
                .clone()
                .ok_or_else(|| MsgError::MissingPaneId {
                    target: strip_escape_sequences(target_name),
                })?,
            agent,
            adopted: worker_state.adopted,
        });
    }

    match matches.len() {
        0 => Err(MsgError::UnknownTarget {
            target: strip_escape_sequences(target_name),
            candidates: target_candidates(run),
        }),
        1 => Ok(matches.remove(0)),
        _ => Err(MsgError::AmbiguousTarget {
            target: strip_escape_sequences(target_name),
            candidates: "god (coordinator), god (worker)".to_owned(),
        }),
    }
}

fn target_launcher(
    launchers: &LauncherTable,
    target: &ResolvedTarget,
    agent: &str,
) -> Result<LauncherEntry, MsgError> {
    if target.adopted && !launchers.contains_key(agent) {
        Ok(conservative_adopted_launcher(agent))
    } else {
        Ok(launcher_entry(launchers, agent)?.clone())
    }
}

fn target_candidates(run: &RunBoard) -> String {
    std::iter::once("god".to_owned())
        .chain(run.state.workers.keys().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join(", ")
}

fn delivery_decision(queues_midturn: bool, status: Option<&str>) -> DeliveryDecision {
    if queues_midturn || matches!(status, None | Some("idle") | Some("done") | Some("unknown")) {
        DeliveryDecision::Deliver
    } else {
        DeliveryDecision::Enqueue
    }
}

fn deliver_message<H: HerdrApi>(
    herdr: &H,
    target: &ResolvedTarget,
    text: &str,
    launcher: &LauncherEntry,
) -> Result<(), MsgError> {
    herdr.pane_run(&target.pane_id, text)?;
    if !launcher.submit_verify {
        return Ok(());
    }

    if herdr.agent_wait(&target.pane_id, "working", SUBMIT_GRACE_TIMEOUT)? == WaitOutcome::Reached {
        return Ok(());
    }

    herdr.pane_run(&target.pane_id, "")?;
    if herdr.agent_wait(&target.pane_id, "working", SUBMIT_VERIFY_TIMEOUT)? == WaitOutcome::Reached
    {
        Ok(())
    } else {
        Err(MsgError::SubmissionTimeout {
            target: target.name.clone(),
        })
    }
}

fn enqueue_message(run_dir: &Path, target: &str, text: &str) -> Result<PathBuf, MsgError> {
    if !is_safe_component(target) {
        return Err(MsgError::UnsafeTarget {
            target: strip_escape_sequences(target),
        });
    }

    let outbox_dir = run_dir.join(OUTBOX_DIR).join(target);
    fs::create_dir_all(&outbox_dir).map_err(|source| MsgError::Io {
        action: "create message outbox",
        path: outbox_dir.clone(),
        source,
    })?;

    loop {
        let file_names = fs::read_dir(&outbox_dir)
            .map_err(|source| MsgError::Io {
                action: "read message outbox",
                path: outbox_dir.clone(),
                source,
            })?
            .filter_map(Result::ok)
            .map(|entry| entry.file_name())
            .collect::<Vec<_>>();
        let sequence =
            next_sequence(file_names.iter().map(|name| name.as_os_str())).ok_or_else(|| {
                MsgError::SequenceExhausted {
                    target: target.to_owned(),
                }
            })?;
        let path = outbox_dir.join(format!("{sequence:0SEQUENCE_WIDTH$}.msg"));
        let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(MsgError::Io {
                    action: "create queued message",
                    path,
                    source,
                })
            }
        };

        if let Err(source) = file.write_all(text.as_bytes()).and_then(|()| file.flush()) {
            let _ = fs::remove_file(&path);
            return Err(MsgError::Io {
                action: "write queued message",
                path,
                source,
            });
        }
        return Ok(path);
    }
}

fn next_sequence<'a>(file_names: impl Iterator<Item = &'a OsStr>) -> Option<u64> {
    file_names
        .filter_map(OsStr::to_str)
        .filter_map(|name| name.strip_suffix(".msg"))
        .filter_map(|sequence| sequence.parse::<u64>().ok())
        .max()
        .unwrap_or(0)
        .checked_add(1)
}

fn is_safe_component(value: &str) -> bool {
    let mut components = Path::new(value).components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

fn strip_escape_sequences(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(character) = chars.next() {
        match character {
            '\u{1b}' => match chars.next() {
                Some('[') => consume_csi(&mut chars),
                Some(']') => consume_control_string(&mut chars, true),
                Some('P' | 'X' | '^' | '_') => consume_control_string(&mut chars, false),
                Some(_) | None => {}
            },
            '\u{009b}' => consume_csi(&mut chars),
            '\u{009d}' => consume_control_string(&mut chars, true),
            '\u{0090}' | '\u{0098}' | '\u{009e}' | '\u{009f}' => {
                consume_control_string(&mut chars, false)
            }
            _ => output.push(character),
        }
    }

    output
}

fn consume_csi(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    for character in chars.by_ref() {
        if ('\u{0040}'..='\u{007e}').contains(&character) {
            break;
        }
    }
}

fn consume_control_string(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    bell_terminated: bool,
) {
    while let Some(character) = chars.next() {
        if bell_terminated && character == '\u{0007}' {
            return;
        }
        if character == '\u{1b}' && chars.peek() == Some(&'\\') {
            chars.next();
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::herdr::{test_support::FakeHerdr, PaneInfo};
    use crate::launcher::default_launcher_table;
    use crate::types::{
        GodSpec, RunState, TeamSpec, Topology, WorkerLifecycle, WorkerRunState, WorkerSpec,
    };
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = env::temp_dir().join(format!(
                "herdr-agent-team-msg-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("create msg test directory");
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

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Call {
        PaneGet(String),
        PaneRun(String, String),
        AgentWait(String, String),
        Notification(String, String, String),
    }

    trait FakeCalls {
        fn typed_calls(&self) -> Vec<Call>;
    }
    impl FakeCalls for FakeHerdr {
        fn typed_calls(&self) -> Vec<Call> {
            self.calls
                .borrow()
                .iter()
                .filter_map(|call| {
                    let parts = call.splitn(4, ':').collect::<Vec<_>>();
                    match parts.as_slice() {
                        ["pane_get", pane] => Some(Call::PaneGet((*pane).to_owned())),
                        ["pane_run", pane, input] => {
                            Some(Call::PaneRun((*pane).to_owned(), (*input).to_owned()))
                        }
                        ["agent_wait", pane, status] => {
                            Some(Call::AgentWait((*pane).to_owned(), (*status).to_owned()))
                        }
                        ["notification", title, body, sound] => Some(Call::Notification(
                            (*title).to_owned(),
                            (*body).to_owned(),
                            (*sound).to_owned(),
                        )),
                        _ => None,
                    }
                })
                .collect()
        }
    }

    fn fake_herdr(
        agent: &str,
        status: Option<&str>,
        waits: impl IntoIterator<Item = WaitOutcome>,
    ) -> FakeHerdr {
        let fake = FakeHerdr::default();
        *fake.pane.borrow_mut() = Some(PaneInfo {
            pane_id: "worker-pane".to_owned(),
            workspace_id: "workspace".to_owned(),
            agent: Some(agent.to_owned()),
            agent_id: Some("session".to_owned()),
            agent_session: None,
            agent_status: status.map(str::to_owned),
            cwd: None,
        });
        *fake.waits.borrow_mut() = waits.into_iter().collect();
        fake
    }

    fn conservative_table() -> LauncherTable {
        let mut table = default_launcher_table();
        table
            .get_mut("codex")
            .expect("shipped codex entry")
            .queues_midturn = false;
        table
    }

    fn fixture_run(root: &Path, worker_name: &str, agent: &str) -> RunBoard {
        let worker = WorkerSpec {
            name: worker_name.to_owned(),
            agent: agent.to_owned(),
            role: "builder".to_owned(),
            task: None,
            worktree: false,
            branch: None,
            brief: PathBuf::from("brief.md"),
        };
        RunBoard {
            dir: root.to_path_buf(),
            state: RunState {
                spec: TeamSpec {
                    name: "alpha".to_owned(),
                    topology: Topology::Star,
                    cwd: PathBuf::from("/tmp/project"),
                    setup: Vec::new(),
                    god: GodSpec::default(),
                    workers: vec![worker],
                },
                god_pane_id: "god-pane".to_owned(),
                herdr_session: Default::default(),
                workers: BTreeMap::from([(
                    worker_name.to_owned(),
                    WorkerRunState {
                        task: None,
                        workspace_id: Some("workspace".to_owned()),
                        pane_id: Some("worker-pane".to_owned()),
                        agent_id: Some("session".to_owned()),
                        agent_session: None,
                        worktree_path: None,
                        adopted: false,
                        lifecycle: WorkerLifecycle::Running,
                    },
                )]),
                lifecycle: RunLifecycle::Active,
            },
        }
    }

    #[test]
    fn resolves_god_and_worker_and_rejects_unknown_or_ambiguous_names() {
        let temp = TempDir::new();
        let run = fixture_run(temp.path(), "builder", "codex");

        assert_eq!(resolve_target(&run, "god").unwrap().pane_id, "god-pane");
        assert_eq!(
            resolve_target(&run, "builder").unwrap().pane_id,
            "worker-pane"
        );

        let unknown = resolve_target(&run, "reviewer").unwrap_err().to_string();
        assert!(unknown.contains("candidates: builder, god"));

        let ambiguous = fixture_run(temp.path(), "god", "codex");
        let error = resolve_target(&ambiguous, "god").unwrap_err().to_string();
        assert!(error.contains("ambiguous message target"));
        assert!(error.contains("god (coordinator), god (worker)"));
    }

    #[test]
    fn queues_midturn_target_delivers_immediately_and_verifies() {
        let temp = TempDir::new();
        let run = fixture_run(temp.path(), "builder", "claude");
        let herdr = fake_herdr("claude", Some("working"), [WaitOutcome::Reached]);

        let outcome = send_message(&run, &default_launcher_table(), "builder", "hello", &herdr)
            .expect("deliver message");

        assert_eq!(outcome, MessageOutcome::Delivered);
        assert_eq!(
            herdr.typed_calls(),
            [
                Call::PaneRun("worker-pane".to_owned(), "hello".to_owned()),
                Call::AgentWait("worker-pane".to_owned(), "working".to_owned()),
            ]
        );
    }

    #[test]
    fn adopted_unknown_agent_remains_messageable_with_conservative_policy() {
        let temp = TempDir::new();
        let mut run = fixture_run(temp.path(), "borrowed", "opencode");
        run.state.workers.get_mut("borrowed").unwrap().adopted = true;
        let herdr = fake_herdr("opencode", Some("idle"), [WaitOutcome::Reached]);

        let outcome = send_message(&run, &default_launcher_table(), "borrowed", "hello", &herdr)
            .expect("synthetic adopted policy should support msg");

        assert_eq!(outcome, MessageOutcome::Delivered);
        assert_eq!(
            herdr.typed_calls(),
            [
                Call::PaneGet("worker-pane".to_owned()),
                Call::PaneRun("worker-pane".to_owned(), "hello".to_owned()),
                Call::AgentWait("worker-pane".to_owned(), "working".to_owned()),
            ]
        );
    }

    #[test]
    fn non_queueing_working_target_writes_increasing_outbox_files_without_delivery() {
        let temp = TempDir::new();
        let run = fixture_run(temp.path(), "builder", "codex");
        let herdr = fake_herdr("codex", Some("working"), []);

        let first = send_message(&run, &conservative_table(), "builder", "first", &herdr)
            .expect("queue first message");
        let second = send_message(&run, &conservative_table(), "builder", "second", &herdr)
            .expect("queue second message");

        let MessageOutcome::Enqueued(first_path) = first else {
            panic!("first message should be queued");
        };
        let MessageOutcome::Enqueued(second_path) = second else {
            panic!("second message should be queued");
        };
        assert_eq!(
            first_path.file_name().unwrap(),
            OsStr::new("00000000000000000001.msg")
        );
        assert_eq!(
            second_path.file_name().unwrap(),
            OsStr::new("00000000000000000002.msg")
        );
        assert_eq!(fs::read_to_string(first_path).unwrap(), "first");
        assert_eq!(fs::read_to_string(second_path).unwrap(), "second");
        assert_eq!(
            herdr.typed_calls(),
            [
                Call::PaneGet("worker-pane".to_owned()),
                Call::PaneGet("worker-pane".to_owned()),
            ]
        );
    }

    #[test]
    fn non_queueing_ready_statuses_deliver_immediately() {
        for status in [Some("idle"), Some("done"), Some("unknown"), None] {
            let temp = TempDir::new();
            let run = fixture_run(temp.path(), "builder", "codex");
            let herdr = fake_herdr("codex", status, [WaitOutcome::Reached]);

            assert_eq!(
                send_message(&run, &conservative_table(), "builder", "ready", &herdr,)
                    .expect("deliver to ready target"),
                MessageOutcome::Delivered
            );
            assert!(herdr
                .typed_calls()
                .iter()
                .any(|call| matches!(call, Call::PaneRun(_, text) if text == "ready")));
        }
    }

    #[test]
    fn timed_out_submission_gets_one_empty_retry_and_second_verification() {
        let temp = TempDir::new();
        let run = fixture_run(temp.path(), "builder", "codex");
        let herdr = fake_herdr(
            "codex",
            Some("idle"),
            [WaitOutcome::TimedOut, WaitOutcome::Reached],
        );

        send_message(&run, &conservative_table(), "builder", "hello", &herdr)
            .expect("deliver with retry");

        assert_eq!(
            herdr.typed_calls(),
            [
                Call::PaneGet("worker-pane".to_owned()),
                Call::PaneRun("worker-pane".to_owned(), "hello".to_owned()),
                Call::AgentWait("worker-pane".to_owned(), "working".to_owned()),
                Call::PaneRun("worker-pane".to_owned(), String::new()),
                Call::AgentWait("worker-pane".to_owned(), "working".to_owned()),
            ]
        );
    }

    #[test]
    fn strips_escape_sequences_before_delivery_or_queueing() {
        assert_eq!(
            strip_escape_sequences("safe\u{1b}[31mred\u{1b}[0m\u{1b}]0;title\u{7} text\u{1b}7done"),
            "safered textdone"
        );

        let temp = TempDir::new();
        let run = fixture_run(temp.path(), "builder", "claude");
        let herdr = fake_herdr("claude", Some("working"), [WaitOutcome::Reached]);
        send_message(
            &run,
            &default_launcher_table(),
            "builder",
            "hello\u{1b}[2Jworld",
            &herdr,
        )
        .expect("deliver sanitized message");
        assert!(herdr.typed_calls().contains(&Call::PaneRun(
            "worker-pane".to_owned(),
            "helloworld".to_owned()
        )));
    }

    #[test]
    fn parses_msg_arguments_and_rejects_invalid_forms() {
        assert_eq!(
            parse_msg_arguments(&[
                "builder".to_owned(),
                "hello".to_owned(),
                "--run".to_owned(),
                "/tmp/run".to_owned(),
            ])
            .unwrap(),
            MsgArguments {
                target: "builder".to_owned(),
                text: "hello".to_owned(),
                run_dir: Some(PathBuf::from("/tmp/run")),
                attention: false,
            }
        );
        assert!(parse_msg_arguments(&["builder".to_owned()]).is_err());
        assert!(parse_msg_arguments(&[
            "builder".to_owned(),
            "hello".to_owned(),
            "--run".to_owned(),
        ])
        .is_err());
    }

    #[test]
    fn attention_request_notifies_once_and_persists_a_metadata_ping() {
        let temp = TempDir::new();
        let run = crate::run::create_run(
            temp.path(),
            fixture_run(temp.path(), "builder", "claude").state,
        )
        .expect("persist attention fixture");
        let herdr = fake_herdr("claude", Some("working"), []);

        request_attention_from_pane(&run, "please review", "worker-pane", &herdr)
            .expect("first attention request");
        request_attention_from_pane(&run, "please review", "worker-pane", &herdr)
            .expect("repeat attention request");

        assert_eq!(
            herdr
                .typed_calls()
                .iter()
                .filter(|call| matches!(call, Call::Notification(..)))
                .count(),
            1,
        );
        let metadata = crate::run::load_hook_metadata(&run.dir).expect("load persisted attention");
        assert_eq!(metadata.attention_pending.get("builder"), Some(&true));
        assert_eq!(
            metadata.aggregate_notifications.get("attention:builder"),
            Some(&true)
        );
    }

    #[test]
    fn sequence_numbering_ignores_unrelated_files_and_detects_exhaustion() {
        let names = [
            OsStr::new("00000000000000000009.msg"),
            OsStr::new("notes.txt"),
            OsStr::new("12.msg"),
        ];
        assert_eq!(next_sequence(names.into_iter()), Some(13));

        let exhausted = format!("{}.msg", u64::MAX);
        assert_eq!(next_sequence([OsStr::new(&exhausted)].into_iter()), None);
    }
}
