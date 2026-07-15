//! Durable god-side inbox, report, and wait verbs (spec section 13).

use crate::agents_md::COMPLETION_SENTINEL;
use crate::paths;
use crate::reconcile::HookMetadata;
use crate::run::{list_active_runs, load_hook_metadata, load_run, update_run_with_hook, RunError};
use crate::types::{RunLifecycle, WorkerLifecycle};
use serde::Serialize;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, UNIX_EPOCH};
use thiserror::Error;

const DEFAULT_TIMEOUT_SECS: u64 = 300;
const POLL_INTERVAL: Duration = Duration::from_millis(200);

#[derive(Debug, Error)]
pub enum GodCliError {
    #[error("invalid arguments: {0}")]
    Usage(String),
    #[error(transparent)]
    Run(#[from] RunError),
    #[error(transparent)]
    Paths(#[from] paths::PathError),
    #[error("no active team run found under {0}")]
    NoActiveRun(PathBuf),
    #[error("unknown worker `{0}`")]
    UnknownWorker(String),
    #[error("unknown worker `{worker}`; candidates: {candidates}")]
    UnknownWaitWorker { worker: String, candidates: String },
    #[error("run is not active: {0}")]
    InactiveRun(PathBuf),
    #[error("report is not present for worker `{0}`")]
    MissingReport(String),
    #[error("report I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("cannot serialize JSON verdict: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InboxRow {
    pub worker: String,
    pub report_present: bool,
    pub report_ready: bool,
    pub report_mtime_ms: Option<u64>,
    pub attention: bool,
    pub read: bool,
    pub stopped_not_done: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GodSnapshot {
    pub run_dir: PathBuf,
    pub lifecycle: RunLifecycle,
    pub rows: Vec<InboxRow>,
    pub worker_lifecycles: Vec<(String, WorkerLifecycle)>,
    pub statuses: Vec<(String, String)>,
}

/// Polling seam intentionally mirrors `BoardCollector`; #8 can replace it.
pub trait GodCollector {
    fn collect(&self) -> Result<GodSnapshot, GodCliError>;
    fn wait_for_change(&self, timeout: Duration) {
        thread::sleep(timeout);
    }
    fn subscription_panes(&self) -> Vec<String> {
        Vec::new()
    }
}

pub struct RunGodCollector {
    pub run_dir: PathBuf,
}

impl GodCollector for RunGodCollector {
    fn collect(&self) -> Result<GodSnapshot, GodCliError> {
        collect_snapshot(&self.run_dir)
    }
    fn subscription_panes(&self) -> Vec<String> {
        load_run(&self.run_dir)
            .map(|run| {
                run.state
                    .workers
                    .values()
                    .filter_map(|worker| worker.pane_id.clone())
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Until {
    AnyReport,
    Report(String),
    AllReports,
    Blocked,
    Attention,
    AllTerminal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerdictKind {
    Reached,
    DeadWorker,
    InactiveRun,
    Timeout,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WaitVerdict {
    pub verdict: VerdictKind,
    pub until: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker: Option<String>,
    pub elapsed_ms: u64,
}

impl WaitVerdict {
    pub fn exit_code(&self) -> u8 {
        match self.verdict {
            VerdictKind::Reached => 0,
            VerdictKind::Timeout => 2,
            VerdictKind::DeadWorker => 3,
            VerdictKind::InactiveRun => 4,
        }
    }
}

pub fn inbox_command(args: &[String]) -> Result<(), GodCliError> {
    let (run_dir, unread, json) = parse_inbox(args)?;
    let rows = collect_snapshot(&select_run(run_dir.as_deref())?)?.rows;
    let rows = rows
        .into_iter()
        .filter(|row| !unread || !row.read)
        .collect::<Vec<_>>();
    if json {
        println!("{}", serde_json::to_string(&rows)?);
    } else {
        for row in rows {
            let state = if row.stopped_not_done {
                "STOPPED-NOT-DONE"
            } else if row.report_present {
                "REPORT"
            } else {
                "-"
            };
            println!(
                "{}\t{}\tattention={}\tread={}\tmtime={}",
                row.worker,
                state,
                row.attention,
                row.read,
                row.report_mtime_ms
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "-".into())
            );
        }
    }
    Ok(())
}

pub fn report_command(args: &[String]) -> Result<(), GodCliError> {
    let (worker, run_dir, head) = parse_report(args)?;
    let run_dir = select_run(run_dir.as_deref())?;
    let run = load_run(&run_dir)?;
    if !run.state.workers.contains_key(&worker) {
        return Err(GodCliError::UnknownWorker(worker));
    }
    let path = run_dir.join("inbox").join(format!("{worker}.md"));
    if !path.is_file() {
        return Err(GodCliError::MissingReport(worker));
    }
    let mtime = report_mtime_ms(&path)?.unwrap_or(0);
    update_run_with_hook::<_, GodCliError>(&run_dir, |_, hook| {
        hook.report_read_mtime_ms.insert(worker.clone(), mtime);
        Ok(())
    })?;
    match head {
        None => println!("{}", path.display()),
        Some(lines) => {
            let contents = fs::read_to_string(path)?;
            let mut stdout = io::stdout().lock();
            for line in contents.lines().take(lines) {
                writeln!(stdout, "{line}")?;
            }
        }
    }
    Ok(())
}

pub fn wait_command(args: &[String]) -> Result<WaitVerdict, GodCliError> {
    let (run_dir, until, timeout, json) = parse_wait(args)?;
    let run_dir = select_wait_run(run_dir.as_deref())?;
    let fallback = RunGodCollector { run_dir };
    let collector: Box<dyn GodCollector> = match crate::socket::SocketClient::try_from_env() {
        Some(socket) => Box::new(crate::socket_backend::SocketGodCollector::new(
            socket, fallback,
        )),
        None => Box::new(fallback),
    };
    validate_until(&collector.collect()?, &until)?;
    let verdict = wait_with(collector.as_ref(), &until, timeout)?;
    if json {
        println!("{}", serde_json::to_string(&verdict)?);
    } else {
        println!("{:?}: {}", verdict.verdict, verdict.until);
    }
    Ok(verdict)
}

pub fn wait_with(
    collector: &(impl GodCollector + ?Sized),
    until: &Until,
    timeout: Duration,
) -> Result<WaitVerdict, GodCliError> {
    let start = Instant::now();
    let deadline = start + timeout;
    let mut first = true;
    loop {
        if !first && Instant::now() >= deadline {
            return Ok(verdict(VerdictKind::Timeout, until, None, start));
        }
        first = false;
        let snapshot = collector.collect()?;
        if snapshot.lifecycle != RunLifecycle::Active {
            return Ok(verdict(VerdictKind::InactiveRun, until, None, start));
        }
        if let Some(worker) = dead_worker(&snapshot, until) {
            return Ok(verdict(VerdictKind::DeadWorker, until, Some(worker), start));
        }
        if condition_met(&snapshot, until) {
            return Ok(verdict(VerdictKind::Reached, until, None, start));
        }
        if Instant::now() >= deadline {
            return Ok(verdict(VerdictKind::Timeout, until, None, start));
        }
        collector
            .wait_for_change(POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())));
    }
}

fn verdict(
    kind: VerdictKind,
    until: &Until,
    worker: Option<String>,
    start: Instant,
) -> WaitVerdict {
    WaitVerdict {
        verdict: kind,
        until: until_name(until),
        worker,
        elapsed_ms: start.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
    }
}

fn condition_met(s: &GodSnapshot, until: &Until) -> bool {
    match until {
        Until::AnyReport => s.rows.iter().any(|r| r.report_ready),
        Until::Report(name) => s.rows.iter().any(|r| r.worker == *name && r.report_ready),
        Until::AllReports => !s.rows.is_empty() && s.rows.iter().all(|r| r.report_ready),
        Until::Blocked => s.statuses.iter().any(|(_, status)| status == "blocked"),
        Until::Attention => s.rows.iter().any(|r| r.attention),
        Until::AllTerminal => s
            .worker_lifecycles
            .iter()
            .all(|(_, state)| terminal(*state)),
    }
}

fn dead_worker(s: &GodSnapshot, until: &Until) -> Option<String> {
    let required = |name: &str| match until {
        Until::Report(w) => w == name,
        Until::AnyReport => !s.rows.iter().any(|r| r.report_ready),
        Until::AllReports => true,
        Until::Blocked | Until::Attention => all_terminal_without_reports(s),
        Until::AllTerminal => false,
    };
    s.worker_lifecycles.iter().find_map(|(name, state)| {
        (required(name)
            && terminal(*state)
            && !s.rows.iter().any(|r| r.worker == *name && r.report_ready))
        .then(|| name.clone())
    })
}

fn all_terminal_without_reports(snapshot: &GodSnapshot) -> bool {
    !snapshot.worker_lifecycles.is_empty()
        && snapshot
            .worker_lifecycles
            .iter()
            .all(|(_, lifecycle)| terminal(*lifecycle))
        && snapshot.rows.iter().all(|row| !row.report_ready)
}

fn validate_until(snapshot: &GodSnapshot, until: &Until) -> Result<(), GodCliError> {
    if let Until::Report(worker) = until {
        if !snapshot.rows.iter().any(|row| row.worker == *worker) {
            return Err(GodCliError::UnknownWaitWorker {
                worker: worker.clone(),
                candidates: snapshot
                    .rows
                    .iter()
                    .map(|row| row.worker.clone())
                    .collect::<Vec<_>>()
                    .join(", "),
            });
        }
    }
    Ok(())
}

fn terminal(state: WorkerLifecycle) -> bool {
    matches!(
        state,
        WorkerLifecycle::Failed
            | WorkerLifecycle::Ended
            | WorkerLifecycle::Released
            | WorkerLifecycle::Orphaned
    )
}

fn collect_snapshot(run_dir: &Path) -> Result<GodSnapshot, GodCliError> {
    let run = load_run(run_dir)?;
    let hook = load_hook_metadata(run_dir)?;
    let rows = run
        .state
        .spec
        .workers
        .iter()
        .map(|worker| row(run_dir, &hook, &worker.name))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(GodSnapshot {
        run_dir: run_dir.to_path_buf(),
        lifecycle: run.state.lifecycle,
        rows,
        worker_lifecycles: run
            .state
            .workers
            .iter()
            .map(|(n, w)| (n.clone(), w.lifecycle))
            .collect(),
        statuses: hook.worker_status.into_iter().collect(),
    })
}

fn row(run_dir: &Path, hook: &HookMetadata, worker: &str) -> Result<InboxRow, GodCliError> {
    let path = run_dir.join("inbox").join(format!("{worker}.md"));
    let mtime = report_mtime_ms(&path)?;
    let status = hook.worker_status.get(worker).map(String::as_str);
    Ok(InboxRow {
        worker: worker.into(),
        report_present: mtime.is_some(),
        report_ready: report_ready(&path)?,
        report_mtime_ms: mtime,
        attention: hook.attention_pending.get(worker).copied().unwrap_or(false)
            || status == Some("blocked"),
        read: mtime
            .is_some_and(|m| hook.report_read_mtime_ms.get(worker).copied().unwrap_or(0) >= m),
        stopped_not_done: mtime.is_none() && matches!(status, Some("idle" | "done")),
    })
}

fn report_ready(path: &Path) -> Result<bool, io::Error> {
    match fs::read_to_string(path) {
        Ok(contents) => Ok(contents
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .is_some_and(|line| line == COMPLETION_SENTINEL)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn report_mtime_ms(path: &Path) -> Result<Option<u64>, io::Error> {
    match fs::metadata(path) {
        Ok(meta) => Ok(Some(
            meta.modified()?
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .min(u128::from(u64::MAX)) as u64,
        )),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

fn select_run(explicit: Option<&Path>) -> Result<PathBuf, GodCliError> {
    if let Some(path) = explicit {
        return Ok(path.to_path_buf());
    }
    let state = paths::state_dir()?;
    list_active_runs(&state)?
        .pop()
        .map(|r| r.dir)
        .ok_or(GodCliError::NoActiveRun(state))
}

fn select_wait_run(explicit: Option<&Path>) -> Result<PathBuf, GodCliError> {
    let path = select_run(explicit)?;
    if load_run(&path)?.state.lifecycle != RunLifecycle::Active {
        return Err(GodCliError::InactiveRun(path));
    }
    Ok(path)
}

fn parse_inbox(args: &[String]) -> Result<(Option<PathBuf>, bool, bool), GodCliError> {
    let mut run = None;
    let mut unread = false;
    let mut json = false;
    parse_flags(args, |flag, value| match flag {
        "--run" => {
            run =
                Some(PathBuf::from(value.ok_or_else(|| {
                    usage("inbox [--run <dir>] [--unread] [--json]")
                })?));
            Ok(true)
        }
        "--unread" => {
            unread = true;
            Ok(false)
        }
        "--json" => {
            json = true;
            Ok(false)
        }
        _ => Err(usage("inbox [--run <dir>] [--unread] [--json]")),
    })?;
    Ok((run, unread, json))
}
fn parse_report(args: &[String]) -> Result<(String, Option<PathBuf>, Option<usize>), GodCliError> {
    let worker = args
        .first()
        .filter(|s| !s.starts_with('-'))
        .cloned()
        .ok_or_else(|| usage("report <worker> [--run <dir>] [--head N]"))?;
    let mut run = None;
    let mut head = None;
    parse_flags(&args[1..], |f, v| match f {
        "--run" => {
            run = Some(PathBuf::from(v.ok_or_else(|| {
                usage("report <worker> [--run <dir>] [--head N]")
            })?));
            Ok(true)
        }
        "--head" => {
            head = Some(
                v.ok_or_else(|| usage("--head requires N"))?
                    .parse()
                    .map_err(|_| usage("--head requires an integer"))?,
            );
            Ok(true)
        }
        _ => Err(usage("report <worker> [--run <dir>] [--head N]")),
    })?;
    Ok((worker, run, head))
}
fn parse_wait(args: &[String]) -> Result<(Option<PathBuf>, Until, Duration, bool), GodCliError> {
    let mut run = None;
    let mut until = None;
    let mut timeout = DEFAULT_TIMEOUT_SECS;
    let mut json = false;
    parse_flags(args, |f, v| match f {
        "--run" => {
            run = Some(PathBuf::from(v.ok_or_else(|| {
                usage("wait --until <condition> [--run <dir>] [--timeout <s>] [--json]")
            })?));
            Ok(true)
        }
        "--until" => {
            until = Some(parse_until(
                v.ok_or_else(|| usage("--until requires a condition"))?,
            )?);
            Ok(true)
        }
        "--timeout" => {
            timeout = v
                .ok_or_else(|| usage("--timeout requires seconds"))?
                .parse()
                .map_err(|_| usage("--timeout requires integer seconds"))?;
            Ok(true)
        }
        "--json" => {
            json = true;
            Ok(false)
        }
        _ => Err(usage(
            "wait --until <condition> [--run <dir>] [--timeout <s>] [--json]",
        )),
    })?;
    Ok((
        run,
        until.ok_or_else(|| usage("--until is required"))?,
        Duration::from_secs(timeout),
        json,
    ))
}
fn parse_flags<F>(args: &[String], mut f: F) -> Result<(), GodCliError>
where
    F: FnMut(&str, Option<&str>) -> Result<bool, GodCliError>,
{
    let mut i = 0;
    while i < args.len() {
        let consumed = f(&args[i], args.get(i + 1).map(String::as_str))?;
        i += if consumed { 2 } else { 1 };
    }
    Ok(())
}
fn parse_until(v: &str) -> Result<Until, GodCliError> {
    Ok(match v {
        "any-report" => Until::AnyReport,
        "all-reports" => Until::AllReports,
        "blocked" => Until::Blocked,
        "attention" => Until::Attention,
        "all-terminal" => Until::AllTerminal,
        _ if v.starts_with("report:") && v.len() > 7 => Until::Report(v[7..].into()),
        _ => return Err(usage("unknown --until condition")),
    })
}
fn until_name(v: &Until) -> String {
    match v {
        Until::AnyReport => "any-report".into(),
        Until::Report(w) => format!("report:{w}"),
        Until::AllReports => "all-reports".into(),
        Until::Blocked => "blocked".into(),
        Until::Attention => "attention".into(),
        Until::AllTerminal => "all-terminal".into(),
    }
}
fn usage(s: &str) -> GodCliError {
    GodCliError::Usage(s.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{GodSpec, RunState, TeamSpec, Topology, WorkerRunState, WorkerSpec};
    use std::collections::BTreeMap;
    struct Fake {
        snapshots: std::sync::Mutex<Vec<GodSnapshot>>,
    }
    impl GodCollector for Fake {
        fn collect(&self) -> Result<GodSnapshot, GodCliError> {
            let mut v = self.snapshots.lock().unwrap();
            Ok(if v.len() > 1 {
                v.remove(0)
            } else {
                v[0].clone()
            })
        }
    }
    fn snap(report: bool, status: &str, life: WorkerLifecycle) -> GodSnapshot {
        GodSnapshot {
            run_dir: "/run".into(),
            lifecycle: RunLifecycle::Active,
            rows: vec![InboxRow {
                worker: "a".into(),
                report_present: report,
                report_ready: report,
                report_mtime_ms: report.then_some(1),
                attention: status == "attention",
                read: false,
                stopped_not_done: false,
            }],
            worker_lifecycles: vec![("a".into(), life)],
            statuses: vec![("a".into(), status.into())],
        }
    }
    struct DeadlineCollector;
    impl GodCollector for DeadlineCollector {
        fn collect(&self) -> Result<GodSnapshot, GodCliError> {
            Ok(snap(false, "working", WorkerLifecycle::Running))
        }
        fn wait_for_change(&self, timeout: Duration) {
            std::thread::sleep(timeout);
        }
    }
    #[test]
    fn wait_preserves_total_deadline_without_second_fallback_sleep() {
        let timeout = Duration::from_millis(40);
        let started = Instant::now();
        let verdict = wait_with(&DeadlineCollector, &Until::AnyReport, timeout).unwrap();
        assert_eq!(verdict.verdict, VerdictKind::Timeout);
        assert!(started.elapsed() < Duration::from_millis(80));
    }
    #[test]
    fn every_until_mode_resolves() {
        for (until, s) in [
            (
                Until::AnyReport,
                snap(true, "working", WorkerLifecycle::Running),
            ),
            (
                Until::Report("a".into()),
                snap(true, "working", WorkerLifecycle::Running),
            ),
            (
                Until::AllReports,
                snap(true, "working", WorkerLifecycle::Running),
            ),
            (
                Until::Blocked,
                snap(false, "blocked", WorkerLifecycle::Running),
            ),
            (
                Until::Attention,
                snap(false, "attention", WorkerLifecycle::Running),
            ),
            (
                Until::AllTerminal,
                snap(false, "done", WorkerLifecycle::Ended),
            ),
        ] {
            let f = Fake {
                snapshots: std::sync::Mutex::new(vec![s]),
            };
            assert_eq!(
                wait_with(&f, &until, Duration::ZERO).unwrap().verdict,
                VerdictKind::Reached
            );
        }
    }
    #[test]
    fn dead_and_timeout_are_distinct_and_json_is_stable() {
        let dead = Fake {
            snapshots: std::sync::Mutex::new(vec![snap(false, "done", WorkerLifecycle::Orphaned)]),
        };
        let d = wait_with(&dead, &Until::AllReports, Duration::ZERO).unwrap();
        assert_eq!(d.exit_code(), 3);
        assert_eq!(
            serde_json::to_string(&d).unwrap(),
            r#"{"verdict":"dead_worker","until":"all-reports","worker":"a","elapsed_ms":0}"#
        );
        let live = Fake {
            snapshots: std::sync::Mutex::new(vec![snap(
                false,
                "working",
                WorkerLifecycle::Running,
            )]),
        };
        assert_eq!(
            wait_with(&live, &Until::AnyReport, Duration::ZERO)
                .unwrap()
                .exit_code(),
            2
        );
    }

    #[test]
    fn ended_worker_without_report_makes_report_waits_unsatisfiable() {
        let ended = snap(false, "done", WorkerLifecycle::Ended);

        for until in [
            Until::Report("a".into()),
            Until::AnyReport,
            Until::AllReports,
        ] {
            let fake = Fake {
                snapshots: std::sync::Mutex::new(vec![ended.clone()]),
            };
            let verdict = wait_with(&fake, &until, Duration::ZERO).unwrap();
            assert_eq!(verdict.verdict, VerdictKind::DeadWorker);
            assert_eq!(verdict.worker.as_deref(), Some("a"));
            assert_eq!(verdict.exit_code(), 3);
        }
    }

    #[test]
    fn inactive_run_unknown_report_and_terminal_conditions_are_distinct() {
        let mut inactive = snap(false, "working", WorkerLifecycle::Running);
        inactive.lifecycle = RunLifecycle::Ended;
        let fake = Fake {
            snapshots: std::sync::Mutex::new(vec![inactive]),
        };
        let verdict = wait_with(&fake, &Until::AnyReport, Duration::from_secs(1)).unwrap();
        assert_eq!(verdict.verdict, VerdictKind::InactiveRun);
        assert_eq!(verdict.exit_code(), 4);

        let running = snap(false, "working", WorkerLifecycle::Running);
        let error = validate_until(&running, &Until::Report("missing".into())).unwrap_err();
        assert!(error.to_string().contains("candidates: a"));

        let terminal = snap(false, "done", WorkerLifecycle::Orphaned);
        assert!(condition_met(&terminal, &Until::AllTerminal));
        assert_eq!(dead_worker(&terminal, &Until::AllTerminal), None);
        assert_eq!(dead_worker(&terminal, &Until::Blocked), Some("a".into()));
        assert_eq!(dead_worker(&terminal, &Until::Attention), Some("a".into()));
    }

    #[test]
    fn inbox_detects_stopped_not_done_and_read_marks_persist() {
        let root = std::env::temp_dir().join(format!("god-cli-inbox-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let worker = WorkerSpec {
            name: "a".into(),
            agent: "codex".into(),
            role: "builder".into(),
            task: None,
            worktree: false,
            branch: None,
            brief: "brief.md".into(),
        };
        let state = RunState {
            spec: TeamSpec {
                name: "team".into(),
                topology: Topology::Star,
                cwd: ".".into(),
                setup: vec![],
                god: GodSpec::default(),
                workers: vec![worker],
            },
            god_pane_id: "god".into(),
            herdr_session: Default::default(),
            workers: BTreeMap::from([(
                "a".into(),
                WorkerRunState {
                    task: None,
                    workspace_id: None,
                    pane_id: Some("pane".into()),
                    agent_id: None,
                    agent_session: None,
                    worktree_path: None,
                    adopted: false,
                    launch_checkpoint: Default::default(),
                    lifecycle: WorkerLifecycle::Running,
                },
            )]),
            lifecycle: RunLifecycle::Active,
        };
        let run = crate::run::create_run(&root, state).unwrap();
        let mut hook = load_hook_metadata(&run.dir).unwrap();
        hook.worker_status.insert("a".into(), "done".into());
        crate::run::save_run_with_hook(&run, &hook).unwrap();
        assert!(collect_snapshot(&run.dir).unwrap().rows[0].stopped_not_done);

        let report = run.dir.join("inbox/a.md");
        fs::write(&report, "one\ntwo\n").unwrap();
        let snapshot = collect_snapshot(&run.dir).unwrap();
        assert!(
            !condition_met(&snapshot, &Until::Report("a".into())),
            "an unfinished report must not satisfy wait report:a"
        );
        fs::write(&report, format!("one\ntwo\n{COMPLETION_SENTINEL}\n")).unwrap();
        assert!(condition_met(
            &collect_snapshot(&run.dir).unwrap(),
            &Until::Report("a".into())
        ));
        let mtime = report_mtime_ms(&report).unwrap().unwrap();
        update_run_with_hook::<_, GodCliError>(&run.dir, |_, hook| {
            hook.report_read_mtime_ms.insert("a".into(), mtime);
            Ok(())
        })
        .unwrap();
        let row = collect_snapshot(&run.dir).unwrap().rows.remove(0);
        assert!(row.read);
        assert!(!row.stopped_not_done);
        assert_eq!(
            load_hook_metadata(&run.dir).unwrap().report_read_mtime_ms["a"],
            mtime
        );
        fs::remove_dir_all(root).unwrap();
    }
}
