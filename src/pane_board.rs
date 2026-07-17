//! Read-only TUI plugin pane (issue #98, ADR-0013 §93 stage 3, docs/spec.md
//! §4 stage 3: "Read-only TUI plugin pane" / dogfood = "full-screen board").
//! Opened via `plugin.pane.open --entrypoint pane-board --placement split`
//! (herdr-plugin.toml).
//!
//! Full-screen view over one team: overview header, per-agent rows (status
//! glyph + `signal_engine` reason badge), a flat native task list, a
//! mailbox tail, and a metadata placeholder row (layout stays stable for
//! the post-v1 JSONL tier, ADR-0013 cut line — that row never gets real
//! data in v1). All reads, no writes to any team state — distinct from
//! `board.rs` (legacy, frozen v1.1.0, a different data model entirely).
//!
//! Two-layer split (same seam as `focus_pane.rs`): [`build_model`] is a
//! pure function from already-gathered facts to a [`BoardModel`] — no
//! I/O, no ratatui types — tested directly; [`gather_board`] is the sole
//! impure caller that does the actual file/herdr reads (reusing
//! `gather::gather_team` and `signal_engine::classify`/`reason_badge`
//! verbatim, no forked logic); `draw` renders a `BoardModel` and only a
//! `BoardModel`.
//!
//! Refresh is a fixed-tick poll (~1s default), same pattern as
//! `recorder::tick` (gather + classify) crossed with `focus_pane`'s
//! poll-with-timeout event loop — no socket events (ADR-0013 cut line:
//! v1 data sources are team files + herdr CLI + transcript mtime-stat
//! only).

use crate::gather::{self, GatherPaths, MailboxEntry, TaskDisplay, TeammateFacts};
use crate::herdr::{HerdrApi, HerdrClient, HerdrError};
use crate::signal_engine::{self, AgentActivity, StalledThresholds};
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event, KeyCode};
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::crossterm::{execute, ExecutableCommand};
use ratatui::layout::{Constraint, Layout};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::{Frame, Terminal};
use std::io;
use std::time::{Duration, Instant, SystemTime};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PaneBoardError {
    #[error("--interval-secs requires a value")]
    MissingIntervalValue,
    #[error("invalid --interval-secs value: {0}")]
    InvalidInterval(String),
    #[error("cannot resolve team-file paths: set HOME")]
    UnresolvedGatherPaths,
    #[error(transparent)]
    ResolveTeam(#[from] gather::ResolveTeamError),
    #[error(transparent)]
    Herdr(#[from] HerdrError),
    #[error("terminal I/O error: {0}")]
    Io(#[from] io::Error),
}

// ─── Pure render model ──────────────────────────────────────────────────────

/// Overview header: honest counts only (ADR-0013 ETA ban — done/total,
/// never a time prediction).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardHeader {
    pub team: String,
    pub agent_count: usize,
    pub tasks_done: usize,
    pub tasks_total: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRow {
    pub name: String,
    pub is_lead: bool,
    pub glyph: char,
    /// `None` for the unbadged-default classes (`TurnComplete`, reason-less
    /// `Waiting`) — same as `signal_engine::reason_badge`'s contract.
    pub badge: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRow {
    pub id: String,
    pub subject: Option<String>,
    pub status: String,
    /// Time since the task *file* last changed on disk (mtime-derived,
    /// ADR-0013 ETA ban). Any status flip or edit resets this — it is
    /// NOT "how long this task has existed" or "time since it started"
    /// (Caio correction, #98 milestone review: the native task schema has
    /// no creation timestamp at all, so this can only ever mean "last
    /// write", never task age). Rendered as "updated Xs/Xm ago", never as
    /// a bare duration, to keep that distinction visible in the UI.
    pub seconds_since_modified: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailboxLine {
    pub agent: String,
    pub from: Option<String>,
    pub text: Option<String>,
    pub age_secs: Option<u64>,
}

/// Full board render model. `metadata_placeholder` is always this fixed
/// string in v1 — the row exists so the layout doesn't reshuffle when the
/// post-v1 JSONL tier (context bar, cost footer, activity tail) graduates
/// (ADR-0013 cut line).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardModel {
    pub header: BoardHeader,
    pub agents: Vec<AgentRow>,
    pub tasks: Vec<TaskRow>,
    pub mailbox: Vec<MailboxLine>,
    pub metadata_placeholder: &'static str,
}

const METADATA_PLACEHOLDER: &str = "metadata: (post-v1 JSONL tier)";

/// Coarse glyph for an agent's raw activity — distinct from the reason
/// badge (which names *why* it's waiting); this is *what state it's in*.
/// Pure, no `signal_engine` changes.
fn status_glyph(activity: AgentActivity) -> char {
    match activity {
        AgentActivity::Working => '●',
        AgentActivity::Idle => '○',
        AgentActivity::Done => '✓',
        AgentActivity::Blocked => '!',
        AgentActivity::Unknown => '·',
    }
}

/// Assemble a [`BoardModel`] from already-gathered facts. Pure — no I/O,
/// no ratatui types in the signature — testable directly on plain fixture
/// data (mirrors `focus_pane.rs`'s `draw`/`AppState` split).
pub fn build_model(
    team: &str,
    teammate_facts: &[TeammateFacts],
    tasks: &[TaskDisplay],
    mailbox: &[MailboxEntry],
    thresholds: &StalledThresholds,
) -> BoardModel {
    let agents: Vec<AgentRow> = teammate_facts
        .iter()
        .map(|teammate| {
            let reason = signal_engine::classify(&teammate.facts, thresholds);
            let badge = signal_engine::reason_badge(reason, None);
            AgentRow {
                name: teammate.name.clone(),
                is_lead: teammate.is_lead,
                glyph: status_glyph(teammate.facts.agent_status),
                badge,
            }
        })
        .collect();

    let tasks_total = tasks.len();
    let tasks_done = tasks
        .iter()
        .filter(|task| task.status == "completed")
        .count();

    let task_rows = tasks
        .iter()
        .map(|task| TaskRow {
            id: task.id.clone(),
            subject: task.subject.clone(),
            status: task.status.clone(),
            seconds_since_modified: task.seconds_since_modified,
        })
        .collect();

    let mailbox_lines = mailbox
        .iter()
        .map(|entry| MailboxLine {
            agent: entry.agent.clone(),
            from: entry.from.clone(),
            text: entry.text.clone(),
            age_secs: entry.seconds_ago,
        })
        .collect();

    BoardModel {
        header: BoardHeader {
            team: team.to_owned(),
            agent_count: teammate_facts.len(),
            tasks_done,
            tasks_total,
        },
        agents,
        tasks: task_rows,
        mailbox: mailbox_lines,
        metadata_placeholder: METADATA_PLACEHOLDER,
    }
}

/// Anything unresolvable degrades to `WaitingReason::Waiting` upstream
/// (never-wrong-reason doctrine) — this function is only for badge tests
/// that need the reason directly, not exposed outside this module.
#[cfg(test)]
fn classify_for_test(facts: &signal_engine::ObservedFacts) -> signal_engine::WaitingReason {
    signal_engine::classify(facts, &StalledThresholds::default())
}

// ─── Impure gather layer ────────────────────────────────────────────────────

const MAILBOX_TAIL_LIMIT: usize = 20;

/// The sole I/O entrypoint for this module: gathers agent facts, task
/// files, and mailbox entries, then hands them to the pure [`build_model`].
/// Reuses `gather::gather_team` verbatim (no forked classification logic).
fn gather_board<H: HerdrApi>(
    paths: &GatherPaths,
    team: &str,
    herdr: &H,
    thresholds: &StalledThresholds,
    now: SystemTime,
) -> BoardModel {
    let teammate_facts = gather::gather_team(paths, team, herdr, now);
    let tasks = gather::team_task_displays(paths, team, now);
    let member_names: Vec<String> = teammate_facts.iter().map(|f| f.name.clone()).collect();
    let mailbox = gather::team_mailbox_tail(paths, team, &member_names, now, MAILBOX_TAIL_LIMIT);
    build_model(team, &teammate_facts, &tasks, &mailbox, thresholds)
}

// ─── Args ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PaneBoardArgs {
    /// `None` when `--team` wasn't passed — a manifest-launched
    /// `plugin.pane.open` invocation has no way to supply one, since its
    /// argv is fixed at declaration time. [`gather::resolve_team`] is the
    /// fallback: exactly one team dir under `teams_root` resolves
    /// silently, anything else is a hard error (never a guess).
    pub team: Option<String>,
    pub interval_secs: u64,
}

const DEFAULT_INTERVAL_SECS: u64 = 1;

pub(crate) fn parse_pane_board_args(args: &[String]) -> Result<PaneBoardArgs, PaneBoardError> {
    let mut team = None;
    let mut interval_secs = DEFAULT_INTERVAL_SECS;

    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--team" => team = iter.next().cloned(),
            "--interval-secs" => {
                let value = iter.next().ok_or(PaneBoardError::MissingIntervalValue)?;
                interval_secs = value
                    .parse()
                    .map_err(|_| PaneBoardError::InvalidInterval(value.clone()))?;
            }
            _ => {}
        }
    }

    Ok(PaneBoardArgs {
        team,
        interval_secs,
    })
}

// ─── Terminal loop ───────────────────────────────────────────────────────────

const POLL_INTERVAL: Duration = Duration::from_millis(300);

/// `herdmates pane-board [--team <name>] [--interval-secs N]`: full-screen
/// read-only board, refreshed on a fixed poll tick (default 1s). Quits on
/// `q`/`Esc`. Runs until quit or a fatal terminal I/O error — never writes
/// to any team file. `--team` is optional: omitted (as it always is when
/// launched via the manifest's `plugin.pane.open` entrypoint) falls back
/// to [`gather::resolve_team`] — the sole team dir if exactly one exists,
/// else a hard error naming the candidates.
pub fn pane_board_command(args: &[String]) -> Result<(), PaneBoardError> {
    let parsed = parse_pane_board_args(args)?;
    let paths = GatherPaths::from_env().ok_or(PaneBoardError::UnresolvedGatherPaths)?;
    let team = gather::resolve_team(&paths, parsed.team.as_deref())?;
    let herdr = HerdrClient::from_env();
    let thresholds = StalledThresholds::default();
    let interval = Duration::from_secs(parsed.interval_secs.max(1));

    enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;

    let result = run_until_quit(&mut terminal, &paths, &team, &herdr, &thresholds, interval);

    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;

    result
}

fn run_until_quit<H: HerdrApi>(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    paths: &GatherPaths,
    team: &str,
    herdr: &H,
    thresholds: &StalledThresholds,
    interval: Duration,
) -> Result<(), PaneBoardError> {
    let mut model = gather_board(paths, team, herdr, thresholds, SystemTime::now());
    let mut last_refresh = Instant::now();

    loop {
        terminal.draw(|frame| draw(frame, &model))?;

        if event::poll(POLL_INTERVAL)? {
            if let Event::Key(key) = event::read()? {
                if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) {
                    return Ok(());
                }
            }
        } else if last_refresh.elapsed() >= interval {
            model = gather_board(paths, team, herdr, thresholds, SystemTime::now());
            last_refresh = Instant::now();
        }
    }
}

// ─── Rendering ───────────────────────────────────────────────────────────────

/// Renders a duration as "how long ago", never a bare number — the bare
/// form reads as "elapsed since start", which task-file mtime and inbox
/// timestamps never mean (mtime resets on every write; Caio correction,
/// #98 milestone review).
fn format_age(secs: Option<u64>) -> String {
    match secs {
        None => "unknown".to_owned(),
        Some(secs) if secs < 60 => format!("{secs}s ago"),
        Some(secs) => format!("{}m ago", secs / 60),
    }
}

/// Pure render: header, agent rows, task list, mailbox tail, metadata
/// placeholder. No I/O — takes only a [`BoardModel`].
fn draw(frame: &mut Frame, model: &BoardModel) {
    let [header_area, agents_area, tasks_area, mailbox_area, metadata_area] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(4),
        Constraint::Min(4),
        Constraint::Min(4),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    frame.render_widget(header_paragraph(&model.header), header_area);
    frame.render_widget(agents_list(&model.agents), agents_area);
    frame.render_widget(tasks_list(&model.tasks), tasks_area);
    frame.render_widget(mailbox_list(&model.mailbox), mailbox_area);
    frame.render_widget(Paragraph::new(model.metadata_placeholder), metadata_area);
}

fn header_paragraph(header: &BoardHeader) -> Paragraph<'static> {
    let text = format!(
        "team {} — {} agents — tasks {}/{}",
        header.team, header.agent_count, header.tasks_done, header.tasks_total
    );
    Paragraph::new(text).block(Block::default().borders(Borders::ALL).title("Overview"))
}

fn agents_list(agents: &[AgentRow]) -> List<'static> {
    let rows: Vec<ListItem> = if agents.is_empty() {
        vec![ListItem::new("(no agents)")]
    } else {
        agents
            .iter()
            .map(|agent| {
                let lead = if agent.is_lead { " (lead)" } else { "" };
                let badge = agent.badge.as_deref().unwrap_or("");
                ListItem::new(format!("{} {}{lead}  {badge}", agent.glyph, agent.name))
            })
            .collect()
    };
    List::new(rows).block(Block::default().borders(Borders::ALL).title("Agents"))
}

fn tasks_list(tasks: &[TaskRow]) -> List<'static> {
    let rows: Vec<ListItem> = if tasks.is_empty() {
        vec![ListItem::new("(no tasks)")]
    } else {
        tasks
            .iter()
            .map(|task| {
                let subject = task.subject.as_deref().unwrap_or("(no subject)");
                ListItem::new(format!(
                    "#{} [{}] {} — updated {}",
                    task.id,
                    task.status,
                    subject,
                    format_age(task.seconds_since_modified)
                ))
            })
            .collect()
    };
    List::new(rows).block(Block::default().borders(Borders::ALL).title("Tasks"))
}

fn mailbox_list(mailbox: &[MailboxLine]) -> List<'static> {
    let rows: Vec<ListItem> = if mailbox.is_empty() {
        vec![ListItem::new("(no mailbox activity)")]
    } else {
        mailbox
            .iter()
            .map(|line| {
                let from = line.from.as_deref().unwrap_or("?");
                let text = line.text.as_deref().unwrap_or("");
                ListItem::new(Line::from(format!(
                    "[{} <- {}] {} ({})",
                    line.agent,
                    from,
                    text,
                    format_age(line.age_secs)
                )))
            })
            .collect()
    };
    List::new(rows).block(Block::default().borders(Borders::ALL).title("Mailbox"))
}

/// Off-screen render for tests — same seam as `focus_pane.rs`'s
/// `render_to_buffer`.
#[cfg(test)]
fn render_to_buffer(model: &BoardModel, width: u16, height: u16) -> ratatui::buffer::Buffer {
    use ratatui::backend::TestBackend;

    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test backend never fails to construct");
    terminal
        .draw(|frame| draw(frame, model))
        .expect("draw into TestBackend never fails");
    terminal.backend().buffer().clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal_engine::{ObservedFacts, WaitingReason};

    fn teammate(name: &str, is_lead: bool, facts: ObservedFacts) -> TeammateFacts {
        TeammateFacts {
            name: name.to_owned(),
            agent_id: format!("{name}@team-x"),
            is_lead,
            facts,
        }
    }

    // ── status_glyph ─────────────────────────────────────────────────────────

    #[test]
    fn status_glyph_covers_every_activity_distinctly() {
        let glyphs = [
            status_glyph(AgentActivity::Working),
            status_glyph(AgentActivity::Idle),
            status_glyph(AgentActivity::Done),
            status_glyph(AgentActivity::Blocked),
            status_glyph(AgentActivity::Unknown),
        ];
        let unique: std::collections::HashSet<_> = glyphs.iter().collect();
        assert_eq!(unique.len(), glyphs.len(), "glyphs must be distinct");
    }

    // ── build_model ──────────────────────────────────────────────────────────

    #[test]
    fn build_model_counts_agents_and_task_done_total_honestly() {
        let facts = vec![
            teammate("team-lead", true, ObservedFacts::default()),
            teammate("alpha", false, ObservedFacts::default()),
        ];
        let tasks = vec![
            TaskDisplay {
                id: "1".to_owned(),
                subject: Some("Ship it".to_owned()),
                status: "completed".to_owned(),
                blocked_by: vec![],
                owner: None,
                seconds_since_modified: Some(30),
            },
            TaskDisplay {
                id: "2".to_owned(),
                subject: None,
                status: "pending".to_owned(),
                blocked_by: vec![],
                owner: None,
                seconds_since_modified: None,
            },
        ];
        let model = build_model("team-x", &facts, &tasks, &[], &StalledThresholds::default());
        assert_eq!(model.header.team, "team-x");
        assert_eq!(model.header.agent_count, 2);
        assert_eq!(model.header.tasks_done, 1);
        assert_eq!(model.header.tasks_total, 2);
        assert_eq!(model.tasks[0].seconds_since_modified, Some(30));
        assert_eq!(model.tasks[1].seconds_since_modified, None);
    }

    #[test]
    fn build_model_badges_agent_rows_from_signal_engine_verbatim() {
        let blocked_facts = ObservedFacts {
            agent_status: AgentActivity::Blocked,
            ..Default::default()
        };
        let facts = vec![teammate("alpha", false, blocked_facts)];

        let model = build_model("team-x", &facts, &[], &[], &StalledThresholds::default());
        assert_eq!(model.agents[0].badge.as_deref(), Some("permission"));
        assert_eq!(
            classify_for_test(&blocked_facts),
            WaitingReason::PermissionPrompt
        );
    }

    #[test]
    fn build_model_unbadged_classes_render_no_badge() {
        let facts = vec![teammate("alpha", false, ObservedFacts::default())];
        let model = build_model("team-x", &facts, &[], &[], &StalledThresholds::default());
        assert_eq!(model.agents[0].badge, None);
    }

    #[test]
    fn build_model_carries_mailbox_lines_through_untouched() {
        let mailbox = vec![MailboxEntry {
            agent: "alpha".to_owned(),
            from: Some("team-lead".to_owned()),
            text: Some("go".to_owned()),
            seconds_ago: Some(90),
        }];
        let model = build_model("team-x", &[], &[], &mailbox, &StalledThresholds::default());
        assert_eq!(model.mailbox.len(), 1);
        assert_eq!(model.mailbox[0].agent, "alpha");
        assert_eq!(model.mailbox[0].text.as_deref(), Some("go"));
    }

    #[test]
    fn build_model_metadata_placeholder_is_stable() {
        let model = build_model("team-x", &[], &[], &[], &StalledThresholds::default());
        assert_eq!(model.metadata_placeholder, METADATA_PLACEHOLDER);
    }

    // ── format_age ───────────────────────────────────────────────────────────

    #[test]
    fn format_age_covers_none_seconds_and_minutes() {
        assert_eq!(format_age(None), "unknown");
        assert_eq!(format_age(Some(45)), "45s ago");
        assert_eq!(format_age(Some(125)), "2m ago");
    }

    // ── args parsing ─────────────────────────────────────────────────────────

    #[test]
    fn parses_team_with_default_interval() {
        let args = parse_pane_board_args(&["--team".to_owned(), "team-x".to_owned()]).unwrap();
        assert_eq!(args.team.as_deref(), Some("team-x"));
        assert_eq!(args.interval_secs, DEFAULT_INTERVAL_SECS);
    }

    #[test]
    fn missing_team_is_not_a_parse_error_it_defers_to_resolve_team() {
        let args = parse_pane_board_args(&[]).unwrap();
        assert_eq!(args.team, None);
    }

    #[test]
    fn invalid_interval_is_an_error() {
        assert!(matches!(
            parse_pane_board_args(&[
                "--team".to_owned(),
                "t".to_owned(),
                "--interval-secs".to_owned(),
                "not-a-number".to_owned(),
            ]),
            Err(PaneBoardError::InvalidInterval(_))
        ));
    }

    // ── draw / rendered content ──────────────────────────────────────────────

    #[test]
    fn empty_board_shows_placeholders() {
        let model = build_model("team-x", &[], &[], &[], &StalledThresholds::default());
        let buffer = render_to_buffer(&model, 80, 24);
        let text: String = buffer.content().iter().map(|cell| cell.symbol()).collect();
        assert!(text.contains("(no agents)"));
        assert!(text.contains("(no tasks)"));
        assert!(text.contains("(no mailbox activity)"));
        assert!(text.contains(METADATA_PLACEHOLDER));
    }

    #[test]
    fn populated_board_renders_header_agent_task_and_mailbox_content() {
        let facts = vec![teammate("team-lead", true, ObservedFacts::default())];
        let tasks = vec![TaskDisplay {
            id: "1".to_owned(),
            subject: Some("Ship the board".to_owned()),
            status: "in_progress".to_owned(),
            blocked_by: vec![],
            owner: None,
            seconds_since_modified: Some(45),
        }];
        let mailbox = vec![MailboxEntry {
            agent: "alpha".to_owned(),
            from: Some("team-lead".to_owned()),
            text: Some("go".to_owned()),
            seconds_ago: Some(90),
        }];
        let model = build_model(
            "team-x",
            &facts,
            &tasks,
            &mailbox,
            &StalledThresholds::default(),
        );
        let buffer = render_to_buffer(&model, 100, 30);
        let text: String = buffer.content().iter().map(|cell| cell.symbol()).collect();
        assert!(text.contains("team-x"));
        assert!(text.contains("1 agents"));
        assert!(text.contains("tasks 0/1"));
        assert!(text.contains("team-lead"));
        assert!(text.contains("(lead)"));
        assert!(text.contains("Ship the board"));
        assert!(
            text.contains("updated 45s ago"),
            "no doubled/mislabeled suffix: {text}"
        );
        assert!(text.contains("go"));
        assert!(
            text.contains("(1m ago)"),
            "no doubled/mislabeled suffix: {text}"
        );
    }
}
