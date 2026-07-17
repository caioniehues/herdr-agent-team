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
use crate::inbox_write::{self, InboxWriteError};
use crate::jump;
use crate::signal_engine::{self, AgentActivity, StalledThresholds};
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event, KeyCode};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
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
    pub agent_id: String,
    pub is_lead: bool,
    pub glyph: char,
    /// `None` for the unbadged-default classes (`TurnComplete`, reason-less
    /// `Waiting`) — same as `signal_engine::reason_badge`'s contract.
    pub badge: Option<String>,
    /// Herdr pane id, when resolvable (#99 jump-to-pane). `None` means the
    /// affordance is hidden for this row, not disabled — lead-only by
    /// construction today (`gather::TeammateFacts::pane_id` doc).
    pub pane_id: Option<String>,
}

/// Full board render model. Task and mailbox rows reuse the gather-layer
/// types directly — they are already pure data (no I/O handles, no ratatui
/// types), so a render-side copy would be duplication, not a seam.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardModel {
    pub header: BoardHeader,
    pub agents: Vec<AgentRow>,
    pub tasks: Vec<TaskDisplay>,
    pub mailbox: Vec<MailboxEntry>,
}

/// Fixed metadata row — exists so the layout doesn't reshuffle when the
/// post-v1 JSONL tier (context bar, cost footer, activity tail) graduates
/// (ADR-0013 cut line). Never varies in v1, so it lives in `draw`, not
/// the model.
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
                agent_id: teammate.agent_id.clone(),
                is_lead: teammate.is_lead,
                glyph: status_glyph(teammate.facts.agent_status),
                badge,
                pane_id: teammate.pane_id.clone(),
            }
        })
        .collect();

    let tasks_total = tasks.len();
    let tasks_done = tasks
        .iter()
        .filter(|task| task.status == "completed")
        .count();

    BoardModel {
        header: BoardHeader {
            team: team.to_owned(),
            agent_count: teammate_facts.len(),
            tasks_done,
            tasks_total,
        },
        agents,
        tasks: tasks.to_vec(),
        mailbox: mailbox.to_vec(),
    }
}

// ─── Affordances: selection, nudge confirm overlay (issue #99) ─────────────

/// Modal overlay state. `None` is the normal board view; the other two
/// variants take over the whole key-handling arm below — nothing else is
/// reachable while an overlay is up (a confirm dialog you can't escape
/// from would be worse than no overlay).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Overlay {
    None,
    /// Pre-composed nudge text, held until the human explicitly confirms
    /// (#92 resolution: "human reviews + confirms before write", no
    /// auto-nudge anywhere).
    ConfirmNudge {
        agent_index: usize,
        text: String,
    },
    /// A write attempt failed (e.g. `InboxWriteError::NoInbox` — the
    /// teammate genuinely has no inbox file, in-process backend). Shown
    /// honestly rather than silently dropped; dismissed on any key.
    WriteError(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BoardUiState {
    selected: usize,
    overlay: Overlay,
}

impl BoardUiState {
    fn new() -> Self {
        Self {
            selected: 0,
            overlay: Overlay::None,
        }
    }

    /// Called after every model refresh: the agent list can shrink/grow
    /// between polls, so the selection index must never point past the
    /// end (never a panic on `agents[selected]`).
    fn clamp_selection(&mut self, agent_count: usize) {
        self.selected = self.selected.min(agent_count.saturating_sub(1));
    }
}

/// What a keypress resolved to — the impure caller (`run_until_quit`)
/// performs the actual herdr call / inbox write; this function stays pure.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Action {
    None,
    Quit,
    Jump(String),
    SendNudge { agent: String, text: String },
}

fn owned_in_progress_task<'a>(
    tasks: &'a [TaskDisplay],
    agent: &AgentRow,
) -> Option<&'a TaskDisplay> {
    tasks.iter().find(|task| {
        task.status == "in_progress"
            && task
                .owner
                .as_deref()
                .is_some_and(|owner| owner == agent.name || owner == agent.agent_id)
    })
}

fn lead_name(agents: &[AgentRow]) -> &str {
    agents
        .iter()
        .find(|agent| agent.is_lead)
        .map(|agent| agent.name.as_str())
        .unwrap_or("lead")
}

/// Pure key-handling state machine: mutates `ui` in place, returns the one
/// side-effecting [`Action`] (if any) for the caller to perform. No I/O,
/// no ratatui types beyond `KeyCode` — directly unit-testable.
fn handle_key(
    ui: &mut BoardUiState,
    agents: &[AgentRow],
    tasks: &[TaskDisplay],
    key: KeyCode,
) -> Action {
    match ui.overlay.clone() {
        Overlay::None => match key {
            KeyCode::Char('q') | KeyCode::Esc => Action::Quit,
            KeyCode::Up | KeyCode::Char('k') => {
                ui.selected = ui.selected.saturating_sub(1);
                Action::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if !agents.is_empty() {
                    ui.selected = (ui.selected + 1).min(agents.len() - 1);
                }
                Action::None
            }
            KeyCode::Char('g') => agents
                .get(ui.selected)
                .and_then(|agent| agent.pane_id.clone())
                .map(Action::Jump)
                .unwrap_or(Action::None),
            KeyCode::Char('n') => {
                if let Some(agent) = agents.get(ui.selected) {
                    let task = owned_in_progress_task(tasks, agent);
                    let text = inbox_write::compose_nudge(task);
                    ui.overlay = Overlay::ConfirmNudge {
                        agent_index: ui.selected,
                        text,
                    };
                }
                Action::None
            }
            _ => Action::None,
        },
        Overlay::ConfirmNudge { agent_index, text } => match key {
            KeyCode::Char('y') | KeyCode::Enter => {
                ui.overlay = Overlay::None;
                match agents.get(agent_index) {
                    Some(agent) => Action::SendNudge {
                        agent: agent.name.clone(),
                        text,
                    },
                    None => Action::None,
                }
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                ui.overlay = Overlay::None;
                Action::None
            }
            _ => Action::None,
        },
        Overlay::WriteError(_) => {
            ui.overlay = Overlay::None;
            Action::None
        }
    }
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
    execute!(io::stdout(), EnterAlternateScreen)?;
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
    let mut ui = BoardUiState::new();
    ui.clamp_selection(model.agents.len());

    loop {
        terminal.draw(|frame| draw(frame, &model, &ui))?;

        if event::poll(POLL_INTERVAL)? {
            if let Event::Key(key) = event::read()? {
                match handle_key(&mut ui, &model.agents, &model.tasks, key.code) {
                    Action::Quit => return Ok(()),
                    Action::Jump(pane_id) => {
                        // Best-effort: navigation, not a write; a failed
                        // jump leaves the human exactly where they were.
                        let _ = jump::jump_to_pane(herdr, &pane_id);
                    }
                    Action::SendNudge { agent, text } => {
                        let from = lead_name(&model.agents).to_owned();
                        let entry = inbox_write::new_entry(
                            &from,
                            &text,
                            SystemTime::now(),
                            inbox_write::generate_msg_id(),
                        );
                        if let Err(err) = inbox_write::append_entry(paths, team, &agent, &entry) {
                            ui.overlay = Overlay::WriteError(describe_write_error(&err));
                        }
                    }
                    Action::None => {}
                }
            }
        } else if last_refresh.elapsed() >= interval {
            model = gather_board(paths, team, herdr, thresholds, SystemTime::now());
            last_refresh = Instant::now();
            ui.clamp_selection(model.agents.len());
        }
    }
}

/// Honest, TUI-facing message for a failed nudge write — never silently
/// swallowed (#99 hard constraint: "missing inbox file = surface honest
/// error in TUI, do not create the file").
fn describe_write_error(err: &InboxWriteError) -> String {
    match err {
        InboxWriteError::NoInbox { agent, .. } => {
            format!("{agent} has no inbox file (in-process teammates aren't reachable this way)")
        }
        other => other.to_string(),
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
/// placeholder, plus the selection cursor and any active overlay. No I/O
/// — takes only a [`BoardModel`] and the transient UI state.
fn draw(frame: &mut Frame, model: &BoardModel, ui: &BoardUiState) {
    let [header_area, agents_area, tasks_area, mailbox_area, metadata_area] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(4),
        Constraint::Min(4),
        Constraint::Min(4),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    frame.render_widget(header_paragraph(&model.header), header_area);
    frame.render_widget(agents_list(&model.agents, ui.selected), agents_area);
    frame.render_widget(tasks_list(&model.tasks), tasks_area);
    frame.render_widget(mailbox_list(&model.mailbox), mailbox_area);
    frame.render_widget(Paragraph::new(METADATA_PLACEHOLDER), metadata_area);

    render_overlay(frame, &ui.overlay);
}

fn render_overlay(frame: &mut Frame, overlay: &Overlay) {
    let (title, body) = match overlay {
        Overlay::None => return,
        Overlay::ConfirmNudge { text, .. } => (
            "Confirm nudge",
            format!("{text}\n\n[y] send   [n/Esc] cancel"),
        ),
        Overlay::WriteError(message) => ("Nudge failed", format!("{message}\n\n[any key] dismiss")),
    };
    let area = centered_rect(60, 30, frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(body).block(Block::default().borders(Borders::ALL).title(title)),
        area,
    );
}

/// Standard ratatui centered-popup recipe.
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(area);
    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(vertical[1])[1]
}

fn header_paragraph(header: &BoardHeader) -> Paragraph<'static> {
    let text = format!(
        "team {} — {} agents — tasks {}/{}",
        header.team, header.agent_count, header.tasks_done, header.tasks_total
    );
    Paragraph::new(text).block(Block::default().borders(Borders::ALL).title("Overview"))
}

fn agents_list(agents: &[AgentRow], selected: usize) -> List<'static> {
    let rows: Vec<ListItem> = if agents.is_empty() {
        vec![ListItem::new("(no agents)")]
    } else {
        agents
            .iter()
            .enumerate()
            .map(|(index, agent)| {
                let cursor = if index == selected { "> " } else { "  " };
                let lead = if agent.is_lead { " (lead)" } else { "" };
                let badge = agent.badge.as_deref().unwrap_or("");
                let jumpable = if agent.pane_id.is_some() {
                    " [g:jump]"
                } else {
                    ""
                };
                ListItem::new(format!(
                    "{cursor}{} {}{lead}  {badge}{jumpable}",
                    agent.glyph, agent.name
                ))
            })
            .collect()
    };
    List::new(rows).block(Block::default().borders(Borders::ALL).title("Agents"))
}

fn tasks_list(tasks: &[TaskDisplay]) -> List<'static> {
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

fn mailbox_list(mailbox: &[MailboxEntry]) -> List<'static> {
    let rows: Vec<ListItem> = if mailbox.is_empty() {
        vec![ListItem::new("(no mailbox activity)")]
    } else {
        mailbox
            .iter()
            .map(|line| {
                let from = line.from.as_deref().unwrap_or("?");
                let text = line.text.as_deref().unwrap_or("");
                ListItem::new(format!(
                    "[{} <- {}] {} ({})",
                    line.agent,
                    from,
                    text,
                    format_age(line.seconds_ago)
                ))
            })
            .collect()
    };
    List::new(rows).block(Block::default().borders(Borders::ALL).title("Mailbox"))
}

/// Off-screen render for tests — same seam as `focus_pane.rs`'s
/// `render_to_buffer`.
#[cfg(test)]
fn render_to_buffer(
    model: &BoardModel,
    ui: &BoardUiState,
    width: u16,
    height: u16,
) -> ratatui::buffer::Buffer {
    use ratatui::backend::TestBackend;

    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test backend never fails to construct");
    terminal
        .draw(|frame| draw(frame, model, ui))
        .expect("draw into TestBackend never fails");
    terminal.backend().buffer().clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal_engine::ObservedFacts;

    fn teammate(name: &str, is_lead: bool, facts: ObservedFacts) -> TeammateFacts {
        teammate_with_pane(name, is_lead, facts, None)
    }

    fn teammate_with_pane(
        name: &str,
        is_lead: bool,
        facts: ObservedFacts,
        pane_id: Option<&str>,
    ) -> TeammateFacts {
        TeammateFacts {
            name: name.to_owned(),
            agent_id: format!("{name}@team-x"),
            is_lead,
            facts,
            pane_id: pane_id.map(str::to_owned),
        }
    }

    fn task(id: &str, status: &str, owner: Option<&str>) -> TaskDisplay {
        TaskDisplay {
            id: id.to_owned(),
            subject: Some(format!("task {id}")),
            status: status.to_owned(),
            owner: owner.map(str::to_owned),
            seconds_since_modified: Some(5),
        }
    }

    fn agent_row(name: &str, agent_id: &str, pane_id: Option<&str>) -> AgentRow {
        AgentRow {
            name: name.to_owned(),
            agent_id: agent_id.to_owned(),
            is_lead: false,
            glyph: '·',
            badge: None,
            pane_id: pane_id.map(str::to_owned),
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
                owner: None,
                seconds_since_modified: Some(30),
            },
            TaskDisplay {
                id: "2".to_owned(),
                subject: None,
                status: "pending".to_owned(),
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
        let buffer = render_to_buffer(&model, &BoardUiState::new(), 80, 24);
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
            owner: Some("team-lead".to_owned()),
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
        let buffer = render_to_buffer(&model, &BoardUiState::new(), 100, 30);
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

    // ── owned_in_progress_task ─────────────────────────────────────────────────

    #[test]
    fn owned_in_progress_task_matches_by_name_or_agent_id() {
        let by_name = agent_row("alpha", "alpha@team-x", None);
        let by_id = agent_row("beta", "beta@team-x", None);
        let tasks = vec![
            task("1", "in_progress", Some("alpha")),
            task("2", "in_progress", Some("beta@team-x")),
            task("3", "completed", Some("alpha")),
        ];
        assert_eq!(
            owned_in_progress_task(&tasks, &by_name).map(|t| t.id.as_str()),
            Some("1")
        );
        assert_eq!(
            owned_in_progress_task(&tasks, &by_id).map(|t| t.id.as_str()),
            Some("2")
        );
    }

    #[test]
    fn owned_in_progress_task_is_none_when_owner_never_matches() {
        let agent = agent_row("alpha", "alpha@team-x", None);
        let tasks = vec![task("1", "in_progress", Some("someone-else"))];
        assert_eq!(owned_in_progress_task(&tasks, &agent), None);
    }

    // ── handle_key: navigation ───────────────────────────────────────────────

    #[test]
    fn up_down_move_selection_and_clamp_at_the_edges() {
        let mut ui = BoardUiState::new();
        let agents = vec![
            agent_row("a", "a@t", None),
            agent_row("b", "b@t", None),
            agent_row("c", "c@t", None),
        ];
        assert_eq!(handle_key(&mut ui, &agents, &[], KeyCode::Up), Action::None);
        assert_eq!(ui.selected, 0, "clamps at the top");

        handle_key(&mut ui, &agents, &[], KeyCode::Down);
        handle_key(&mut ui, &agents, &[], KeyCode::Char('j'));
        assert_eq!(ui.selected, 2);

        handle_key(&mut ui, &agents, &[], KeyCode::Down);
        assert_eq!(ui.selected, 2, "clamps at the bottom");
    }

    #[test]
    fn q_and_esc_quit_when_no_overlay_is_active() {
        let mut ui = BoardUiState::new();
        assert_eq!(
            handle_key(&mut ui, &[], &[], KeyCode::Char('q')),
            Action::Quit
        );
        assert_eq!(handle_key(&mut ui, &[], &[], KeyCode::Esc), Action::Quit);
    }

    // ── handle_key: jump ──────────────────────────────────────────────────────

    #[test]
    fn g_jumps_when_the_selected_agent_has_a_pane_id() {
        let mut ui = BoardUiState::new();
        let agents = vec![agent_row("alpha", "alpha@t", Some("w1A:p1"))];
        assert_eq!(
            handle_key(&mut ui, &agents, &[], KeyCode::Char('g')),
            Action::Jump("w1A:p1".to_owned())
        );
    }

    #[test]
    fn g_is_a_no_op_when_the_selected_agent_has_no_pane_id() {
        let mut ui = BoardUiState::new();
        let agents = vec![agent_row("alpha", "alpha@t", None)];
        assert_eq!(
            handle_key(&mut ui, &agents, &[], KeyCode::Char('g')),
            Action::None
        );
    }

    // ── handle_key: nudge confirm/cancel ─────────────────────────────────────

    #[test]
    fn n_opens_a_confirm_overlay_pre_composed_from_the_owned_task_never_auto_sends() {
        let mut ui = BoardUiState::new();
        let agents = vec![agent_row("alpha", "alpha@t", None)];
        let tasks = vec![task("1", "in_progress", Some("alpha"))];

        let action = handle_key(&mut ui, &agents, &tasks, KeyCode::Char('n'));

        assert_eq!(action, Action::None, "no write happens on 'n' alone");
        match &ui.overlay {
            Overlay::ConfirmNudge { agent_index, text } => {
                assert_eq!(*agent_index, 0);
                assert!(text.contains("task 1"));
            }
            other => panic!("expected ConfirmNudge overlay, got {other:?}"),
        }
    }

    #[test]
    fn y_confirms_the_overlay_and_emits_send_nudge_for_the_right_agent() {
        let mut ui = BoardUiState::new();
        let agents = vec![agent_row("alpha", "alpha@t", None)];
        handle_key(&mut ui, &agents, &[], KeyCode::Char('n'));

        let action = handle_key(&mut ui, &agents, &[], KeyCode::Char('y'));

        assert_eq!(ui.overlay, Overlay::None, "overlay closes on confirm");
        match action {
            Action::SendNudge { agent, .. } => assert_eq!(agent, "alpha"),
            other => panic!("expected SendNudge, got {other:?}"),
        }
    }

    #[test]
    fn esc_and_n_cancel_the_overlay_without_sending() {
        for cancel_key in [KeyCode::Esc, KeyCode::Char('n')] {
            let mut ui = BoardUiState::new();
            let agents = vec![agent_row("alpha", "alpha@t", None)];
            handle_key(&mut ui, &agents, &[], KeyCode::Char('n'));

            let action = handle_key(&mut ui, &agents, &[], cancel_key);

            assert_eq!(action, Action::None);
            assert_eq!(ui.overlay, Overlay::None);
        }
    }

    #[test]
    fn any_key_dismisses_a_write_error_overlay_without_a_new_action() {
        let mut ui = BoardUiState {
            selected: 0,
            overlay: Overlay::WriteError("boom".to_owned()),
        };
        let action = handle_key(&mut ui, &[], &[], KeyCode::Char('x'));
        assert_eq!(action, Action::None);
        assert_eq!(ui.overlay, Overlay::None);
    }

    // ── clamp_selection ───────────────────────────────────────────────────────

    #[test]
    fn clamp_selection_pulls_the_index_back_when_the_agent_list_shrinks() {
        let mut ui = BoardUiState {
            selected: 4,
            overlay: Overlay::None,
        };
        ui.clamp_selection(2);
        assert_eq!(ui.selected, 1);
        ui.clamp_selection(0);
        assert_eq!(ui.selected, 0, "never underflows on an empty list");
    }
}
