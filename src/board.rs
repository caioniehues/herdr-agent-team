//! Keyboard-driven human control deck (spec section 8, roadmap step 4).

use crate::run::{list_active_runs, load_run, RunError};
use crate::types::WorkerLifecycle;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode},
    execute,
    terminal::{self, ClearType},
};
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use thiserror::Error;

const BOARD_USAGE: &str = "usage: herdr-agent-team board [--run <run-dir>]";

#[derive(Debug, Error)]
pub enum BoardError {
    #[error("{0}")]
    Usage(String),
    #[error(transparent)]
    Run(#[from] RunError),
    #[error("cannot read board inbox {path}: {source}")]
    Inbox { path: PathBuf, source: io::Error },
    #[error("cannot enter board terminal mode: {0}")]
    Terminal(#[from] io::Error),
    #[error("no active team run found under {0}")]
    NoActiveRun(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardWorker {
    pub name: String,
    pub lifecycle: WorkerLifecycle,
    pub pane_id: Option<String>,
    pub task: Option<String>,
    pub report: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardSnapshot {
    pub team: String,
    pub run_dir: PathBuf,
    pub lifecycle: String,
    pub workers: Vec<BoardWorker>,
    pub mailbox_events: usize,
}

/// Collection stays behind this seam so socket snapshots can replace polling in #8.
pub trait BoardCollector {
    fn collect(&self) -> Result<BoardSnapshot, BoardError>;
    fn wait_for_change(&self, timeout: Duration) -> bool {
        std::thread::sleep(timeout);
        true
    }
    fn subscription_panes(&self) -> Vec<String> {
        Vec::new()
    }
}

pub struct RunCollector {
    pub run_dir: PathBuf,
}

impl BoardCollector for RunCollector {
    fn collect(&self) -> Result<BoardSnapshot, BoardError> {
        collect_run(&self.run_dir)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardKey {
    Up,
    Down,
    Msg,
    Acknowledge,
    Kill,
    Open,
    Adopt,
    Quit,
    Refresh,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardAction {
    Msg { worker: String },
    Acknowledge { worker: String },
    Kill { worker: String },
    Open { report: Option<PathBuf> },
    Adopt,
    Quit,
    Refresh,
    None,
}

pub fn apply_key(selection: &mut usize, worker_count: usize, key: BoardKey) -> BoardAction {
    if worker_count == 0 {
        return if key == BoardKey::Quit {
            BoardAction::Quit
        } else {
            BoardAction::None
        };
    }
    match key {
        BoardKey::Down => {
            *selection = (*selection + 1) % worker_count;
            BoardAction::None
        }
        BoardKey::Up => {
            *selection = (*selection + worker_count - 1) % worker_count;
            BoardAction::None
        }
        BoardKey::Msg => BoardAction::Msg {
            worker: String::new(),
        },
        BoardKey::Acknowledge => BoardAction::Acknowledge {
            worker: String::new(),
        },
        BoardKey::Kill => BoardAction::Kill {
            worker: String::new(),
        },
        BoardKey::Open => BoardAction::Open { report: None },
        BoardKey::Adopt => BoardAction::Adopt,
        BoardKey::Quit => BoardAction::Quit,
        BoardKey::Refresh => BoardAction::Refresh,
        BoardKey::Other => BoardAction::None,
    }
}

pub fn action_args(
    action: &BoardAction,
    run_dir: &Path,
    text: Option<&str>,
    pane: Option<&str>,
    name: Option<&str>,
) -> Option<Vec<String>> {
    let run = run_dir.display().to_string();
    match action {
        BoardAction::Msg { worker } => Some(vec![
            "msg".into(),
            worker.clone(),
            text.unwrap_or_default().into(),
            "--run".into(),
            run,
        ]),
        BoardAction::Acknowledge { worker } => Some(vec![
            "msg".into(),
            worker.clone(),
            "acknowledged".into(),
            "--run".into(),
            run,
        ]),
        BoardAction::Kill { worker } => {
            Some(vec!["kill".into(), run, "--worker".into(), worker.clone()])
        }
        BoardAction::Adopt => Some(vec![
            "adopt".into(),
            pane.unwrap_or_default().into(),
            "--name".into(),
            name.unwrap_or_default().into(),
            "--run".into(),
            run,
        ]),
        _ => None,
    }
}

pub fn render(snapshot: &BoardSnapshot, selection: usize) -> String {
    let mut output = format!(" BOARD · {} · {} · {} workers\n\n     WORKER       STATE      PANE      TASK                       REPORT\n", snapshot.team, snapshot.lifecycle, snapshot.workers.len());
    for (index, worker) in snapshot.workers.iter().enumerate() {
        let marker = if index == selection { " ▶ " } else { "   " };
        let report = worker
            .report
            .as_ref()
            .map(|path| format!("report:{}", path.display()))
            .unwrap_or_else(|| "-".into());
        output.push_str(&format!(
            "{marker}{} {:<12} {:<10} {:<9} {:<26} {report}\n",
            glyph(worker.lifecycle),
            worker.name,
            lifecycle_name(worker.lifecycle),
            worker.pane_id.as_deref().unwrap_or("-"),
            worker.task.as_deref().unwrap_or("")
        ));
    }
    output.push_str(&format!("\n team {}   mailbox: {} events\n────────────────────────────────────────────────────────\n [j/k] select  [m] msg  [g] ack  [K] kill  [o] open report  [p] adopt  [q] quit\n", team_strip(&snapshot.workers), snapshot.mailbox_events));
    output
}

pub fn board_command(args: &[String]) -> Result<(), BoardError> {
    let run_dir = select_run(args)?;
    let fallback = RunCollector { run_dir };
    if let Some(socket) = crate::socket::SocketClient::try_from_env() {
        run_board(crate::socket_backend::SocketBoardCollector::new(
            socket, fallback,
        ))
    } else {
        run_board(fallback)
    }
}

pub fn open_report_command(args: &[String]) -> Result<(), BoardError> {
    let path = if let Ok(url) = env::var("HERDR_PLUGIN_CLICKED_URL") {
        PathBuf::from(url.trim_start_matches("report:"))
    } else {
        args.first().map(PathBuf::from).ok_or_else(|| {
            BoardError::Usage("usage: herdr-agent-team open-report <report-path>".into())
        })?
    };
    open_report(&path);
    Ok(())
}

fn select_run(args: &[String]) -> Result<PathBuf, BoardError> {
    let state_dir = env::var_os("HERDR_PLUGIN_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    match args {
        [] => list_active_runs(&state_dir)?
            .pop()
            .map(|run| run.dir)
            .ok_or(BoardError::NoActiveRun(state_dir)),
        [flag, path] if flag == "--run" => Ok(PathBuf::from(path)),
        _ => Err(BoardError::Usage(BOARD_USAGE.into())),
    }
}

fn collect_run(run_dir: &Path) -> Result<BoardSnapshot, BoardError> {
    let run = load_run(run_dir)?;
    let mailbox = run.dir.join("inbox").join("events.jsonl");
    let mailbox_events = match fs::read_to_string(&mailbox) {
        Ok(events) => events.lines().count(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => 0,
        Err(source) => {
            return Err(BoardError::Inbox {
                path: mailbox,
                source,
            })
        }
    };
    let workers = run
        .state
        .spec
        .workers
        .iter()
        .map(|spec| {
            let state = run
                .state
                .workers
                .get(&spec.name)
                .expect("run states are keyed by spec worker");
            let report_path = run.dir.join("inbox").join(format!("{}.md", spec.name));
            BoardWorker {
                name: spec.name.clone(),
                lifecycle: state.lifecycle,
                pane_id: state.pane_id.clone(),
                task: state.task.clone().or_else(|| spec.task.clone()),
                report: report_path.is_file().then_some(report_path),
            }
        })
        .collect();
    Ok(BoardSnapshot {
        team: run.state.spec.name,
        run_dir: run.dir,
        lifecycle: format!("{:?}", run.state.lifecycle).to_lowercase(),
        workers,
        mailbox_events,
    })
}

fn run_board(collector: impl BoardCollector) -> Result<(), BoardError> {
    let mut stdout = io::stdout();
    let mut selection = 0;
    let mut snapshot = collector.collect()?;
    terminal::enable_raw_mode()?;
    let result = (|| -> Result<(), BoardError> {
        loop {
            selection %= snapshot.workers.len().max(1);
            execute!(
                stdout,
                terminal::Clear(ClearType::All),
                cursor::MoveTo(0, 0)
            )?;
            print!("{}", render(&snapshot, selection));
            stdout.flush()?;
            if !event::poll(Duration::from_millis(100))? {
                if collector.wait_for_change(Duration::from_millis(100)) {
                    snapshot = collector.collect()?;
                }
                continue;
            }
            let Event::Key(key) = event::read()? else {
                continue;
            };
            let raw_action = apply_key(
                &mut selection,
                snapshot.workers.len(),
                key_from_code(key.code),
            );
            let action = bind_worker(raw_action, snapshot.workers.get(selection));
            match action {
                BoardAction::Quit => break,
                BoardAction::Refresh | BoardAction::None => {}
                BoardAction::Open { report: Some(path) } => {
                    terminal::disable_raw_mode()?;
                    open_report(&path);
                    terminal::enable_raw_mode()?;
                }
                BoardAction::Open { report: None } => {}
                action => {
                    terminal::disable_raw_mode()?;
                    execute_action(&action, &snapshot.run_dir)?;
                    terminal::enable_raw_mode()?;
                }
            }
            snapshot = collector.collect()?;
        }
        Ok(())
    })();
    terminal::disable_raw_mode()?;
    result
}

fn bind_worker(action: BoardAction, worker: Option<&BoardWorker>) -> BoardAction {
    let Some(worker) = worker else { return action };
    match action {
        BoardAction::Msg { .. } => BoardAction::Msg {
            worker: worker.name.clone(),
        },
        BoardAction::Acknowledge { .. } => BoardAction::Acknowledge {
            worker: worker.name.clone(),
        },
        BoardAction::Kill { .. } => BoardAction::Kill {
            worker: worker.name.clone(),
        },
        BoardAction::Open { .. } => BoardAction::Open {
            report: worker.report.clone(),
        },
        other => other,
    }
}

fn execute_action(action: &BoardAction, run_dir: &Path) -> Result<(), BoardError> {
    let (text, pane, name) = match action {
        BoardAction::Msg { .. } => (Some(prompt("message: ")?), None, None),
        BoardAction::Adopt => (
            None,
            Some(prompt("pane id: ")?),
            Some(prompt("worker name: ")?),
        ),
        _ => (None, None, None),
    };
    let Some(args) = action_args(
        action,
        run_dir,
        text.as_deref(),
        pane.as_deref(),
        name.as_deref(),
    ) else {
        return Ok(());
    };
    let status = Command::new(env::current_exe().map_err(BoardError::Terminal)?)
        .args(args)
        .status()
        .map_err(BoardError::Terminal)?;
    if !status.success() {
        eprintln!("board action failed: {status}");
    }
    Ok(())
}

fn prompt(label: &str) -> Result<String, BoardError> {
    print!("{label}");
    io::stdout().flush()?;
    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    Ok(value.trim().to_owned())
}
fn open_report(path: &Path) {
    let pager = env::var("PAGER").unwrap_or_else(|_| "less".into());
    let _ = Command::new(pager).arg(path).status();
}
fn key_from_code(code: KeyCode) -> BoardKey {
    match code {
        KeyCode::Char('j') | KeyCode::Down => BoardKey::Down,
        KeyCode::Char('k') | KeyCode::Up => BoardKey::Up,
        KeyCode::Char('m') => BoardKey::Msg,
        KeyCode::Char('g') => BoardKey::Acknowledge,
        KeyCode::Char('K') => BoardKey::Kill,
        KeyCode::Char('o') => BoardKey::Open,
        KeyCode::Char('p') => BoardKey::Adopt,
        KeyCode::Char('q') | KeyCode::Esc => BoardKey::Quit,
        KeyCode::Char('r') => BoardKey::Refresh,
        _ => BoardKey::Other,
    }
}
fn glyph(state: WorkerLifecycle) -> char {
    match state {
        WorkerLifecycle::Running => '●',
        WorkerLifecycle::Pending => '◌',
        WorkerLifecycle::Ended => '○',
        WorkerLifecycle::Released => '◇',
        WorkerLifecycle::Failed | WorkerLifecycle::Orphaned => '✖',
    }
}
fn lifecycle_name(state: WorkerLifecycle) -> &'static str {
    match state {
        WorkerLifecycle::Pending => "pending",
        WorkerLifecycle::Running => "running",
        WorkerLifecycle::Failed => "failed",
        WorkerLifecycle::Ended => "ended",
        WorkerLifecycle::Released => "released",
        WorkerLifecycle::Orphaned => "orphaned",
    }
}
fn team_strip(workers: &[BoardWorker]) -> String {
    workers
        .iter()
        .map(|worker| glyph(worker.lifecycle).to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_wraps() {
        let mut selected = 0;
        apply_key(&mut selected, 2, BoardKey::Up);
        assert_eq!(selected, 1);
        apply_key(&mut selected, 2, BoardKey::Down);
        assert_eq!(selected, 0);
    }
    #[test]
    fn action_keys_map_to_exact_cli_arguments() {
        let run = Path::new("/runs/team");
        assert_eq!(
            action_args(
                &BoardAction::Kill { worker: "f".into() },
                run,
                None,
                None,
                None
            ),
            Some(
                vec!["kill", "/runs/team", "--worker", "f"]
                    .into_iter()
                    .map(String::from)
                    .collect()
            )
        );
        assert_eq!(
            action_args(
                &BoardAction::Msg { worker: "f".into() },
                run,
                Some("hello"),
                None,
                None
            ),
            Some(
                vec!["msg", "f", "hello", "--run", "/runs/team"]
                    .into_iter()
                    .map(String::from)
                    .collect()
            )
        );
        assert_eq!(
            action_args(
                &BoardAction::Acknowledge { worker: "f".into() },
                run,
                None,
                None,
                None
            ),
            Some(
                vec!["msg", "f", "acknowledged", "--run", "/runs/team"]
                    .into_iter()
                    .map(String::from)
                    .collect()
            )
        );
        assert_eq!(
            action_args(&BoardAction::Adopt, run, None, Some("w1:p2"), Some("new")),
            Some(
                vec!["adopt", "w1:p2", "--name", "new", "--run", "/runs/team"]
                    .into_iter()
                    .map(String::from)
                    .collect()
            )
        );
    }
    #[test]
    fn render_snapshot_has_table_strip_and_footer() {
        let snapshot = BoardSnapshot {
            team: "wave".into(),
            run_dir: PathBuf::from("/run"),
            lifecycle: "active".into(),
            mailbox_events: 3,
            workers: vec![BoardWorker {
                name: "builder".into(),
                lifecycle: WorkerLifecycle::Running,
                pane_id: Some("w1:p2".into()),
                task: Some("ship board".into()),
                report: Some(PathBuf::from("/run/inbox/builder.md")),
            }],
        };
        let output = render(&snapshot, 0);
        assert!(output.contains("WORKER"));
        assert!(output.contains("mailbox: 3 events"));
        assert!(output.contains("[j/k] select"));
        assert!(output.contains("report:/run/inbox/builder.md"));
    }
}
