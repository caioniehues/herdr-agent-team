//! Focus pane TUI (D3, issue #86 commit 7): adds the attention-queue region
//! on top of commit 6's static task/next-action/decisions skeleton — a
//! selectable list (j/k to move, Enter to jump to the selected item's pane,
//! d to mark it done in the audit log), plus a live refresh loop so the pane
//! reflects file/status changes made while it's open.
//!
//! herdr has no filesystem-watch primitive exposed to plugins, so "live
//! file-watch refresh" here is poll-based, not inotify-based: the event
//! loop polls for a keypress with a short timeout, and on every timeout
//! (no key pressed) checks whether at least 1s has passed since the last
//! state reload — that 1s floor is the "debounce": rapid underlying changes
//! (a worker writing several inbox messages in a burst, editing the focus
//! file a few times) collapse into at most one reload per second rather
//! than one reload per 300ms poll tick. A reload failure never tears down
//! the pane (ADR-0012 degrade policy) — the previous good state is kept.
//!
//! Render model stays pure and separate from terminal I/O (unchanged split
//! from commit 6): `draw` only takes `&mut Frame` and `&AppState`, and
//! `apply_key`/`clamp_selection` are plain functions with no ratatui/herdr
//! types in their signatures, so all three are exercised in tests without a
//! real terminal or a `HerdrApi` call.

use crate::attention::{AttentionItem, AttentionKind};
use crate::audit;
use crate::focusfile::{self, FocusFile, FocusFileError};
use crate::herdr::{HerdrClient, HerdrError};
use crate::jump;
use crate::paths::PathError;
use crate::pump;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event, KeyCode};
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::crossterm::{execute, ExecutableCommand};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FocusPaneError {
    #[error(transparent)]
    FocusFile(#[from] FocusFileError),
    #[error("cannot resolve the Claude Code team files directory: set HOME")]
    UnresolvedTeamsRoot,
    #[error(transparent)]
    Herdr(#[from] HerdrError),
    #[error(transparent)]
    Path(#[from] PathError),
    #[error(transparent)]
    Audit(#[from] audit::AuditLogError),
    #[error("terminal I/O error: {0}")]
    Io(#[from] io::Error),
}

struct Runtime {
    herdr: HerdrClient,
    teams_root: PathBuf,
    audit_path: PathBuf,
}

struct AppState {
    focus: FocusFile,
    queue: Vec<AttentionItem>,
    selection: usize,
    /// One-line status message (issue #86 review finding 3): a failed jump
    /// used to be silently swallowed (`let _ = jump::jump_to_pane(...)`),
    /// leaving the human staring at an apparently-unresponsive UI. Set on
    /// jump failure, cleared on jump success; carried across reloads so it
    /// doesn't vanish the instant the 1s debounce tick fires.
    status: Option<String>,
}

pub fn focus_pane_command(_args: &[String]) -> Result<(), FocusPaneError> {
    let runtime = Runtime {
        herdr: HerdrClient::from_env(),
        teams_root: pump::default_teams_root().map_err(|_| FocusPaneError::UnresolvedTeamsRoot)?,
        audit_path: jump::default_audit_log_path()?,
    };
    let state = load_state(&runtime)?;

    enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;

    let result = run_until_quit(&mut terminal, &runtime, state);

    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;

    result
}

fn load_state(runtime: &Runtime) -> Result<AppState, FocusPaneError> {
    let focus = focusfile::read_focus_file(&jump::default_focus_file_path())?;
    let agents = runtime.herdr.agent_list()?;
    let team_leads = jump::discover_team_leads(&runtime.teams_root, &agents);
    let consumed = audit::read_consumed(&runtime.audit_path)?;
    let queue = audit::filter_unconsumed(
        jump::merge_team_queues(&agents, &focus, &team_leads),
        &consumed,
        jump::now_ms(),
    );
    Ok(AppState {
        focus,
        queue,
        selection: 0,
        status: None,
    })
}

/// Best-effort reload: any I/O failure keeps the previous state rather than
/// tearing down the pane (ADR-0012 degrade policy).
fn try_reload_state(runtime: &Runtime, previous: AppState) -> AppState {
    match load_state(runtime) {
        Ok(mut fresh) => {
            fresh.selection = previous.selection.min(fresh.queue.len().saturating_sub(1));
            fresh.status = previous.status;
            fresh
        }
        Err(_) => previous,
    }
}

const DEBOUNCE: Duration = Duration::from_secs(1);
const POLL_INTERVAL: Duration = Duration::from_millis(300);

fn run_until_quit(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    runtime: &Runtime,
    mut state: AppState,
) -> Result<(), FocusPaneError> {
    let mut last_refresh = Instant::now();
    loop {
        terminal.draw(|frame| draw(frame, &state))?;

        if event::poll(POLL_INTERVAL)? {
            if let Event::Key(key) = event::read()? {
                match apply_key(key.code) {
                    QueueAction::Move(delta) => {
                        state.selection =
                            clamp_selection(state.selection, state.queue.len(), delta);
                    }
                    QueueAction::Jump => {
                        match state
                            .queue
                            .get(state.selection)
                            .and_then(|item| item.pane_id.as_deref())
                        {
                            Some(pane_id) => {
                                state.status = match jump::jump_to_pane(&runtime.herdr, pane_id) {
                                    Ok(()) => None,
                                    Err(error) => Some(format!("jump failed: {error}")),
                                };
                            }
                            None => {
                                state.status =
                                    Some("selected item has no pane to jump to".to_owned());
                            }
                        }
                    }
                    QueueAction::MarkDone => {
                        if let Some(item) = state.queue.get(state.selection) {
                            let _ = audit::append_consumed(
                                &runtime.audit_path,
                                &item.id,
                                jump::now_ms(),
                            );
                            state = try_reload_state(runtime, state);
                        }
                    }
                    QueueAction::Quit => return Ok(()),
                    QueueAction::None => {}
                }
            }
        } else if last_refresh.elapsed() >= DEBOUNCE {
            state = try_reload_state(runtime, state);
            last_refresh = Instant::now();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QueueAction {
    Move(isize),
    Jump,
    MarkDone,
    Quit,
    None,
}

/// Pure keymap — no ratatui rendering, no I/O — tested directly.
fn apply_key(key: KeyCode) -> QueueAction {
    match key {
        KeyCode::Char('j') | KeyCode::Down => QueueAction::Move(1),
        KeyCode::Char('k') | KeyCode::Up => QueueAction::Move(-1),
        KeyCode::Enter => QueueAction::Jump,
        KeyCode::Char('d') => QueueAction::MarkDone,
        KeyCode::Char('q') | KeyCode::Esc => QueueAction::Quit,
        _ => QueueAction::None,
    }
}

/// Move `selection` by `delta`, clamped to `[0, len - 1]` (or `0` when
/// `len == 0`). Pure — tested directly.
fn clamp_selection(selection: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    (selection as isize + delta).clamp(0, len as isize - 1) as usize
}

/// Pure render: attention queue on top, then task / next-action /
/// decisions, then a one-line status area. No I/O.
fn draw(frame: &mut Frame, state: &AppState) {
    let [queue_area, task_area, next_action_area, decisions_area, status_area] =
        Layout::vertical([
            Constraint::Min(5),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .areas(frame.area());

    let mut list_state = ListState::default();
    if !state.queue.is_empty() {
        list_state.select(Some(state.selection));
    }
    frame.render_stateful_widget(queue_list(&state.queue), queue_area, &mut list_state);

    frame.render_widget(section("Task", state.focus.task.as_deref()), task_area);
    frame.render_widget(
        section("Next Action", state.focus.next_action.as_deref()),
        next_action_area,
    );
    frame.render_widget(decisions_section(&state.focus), decisions_area);
    frame.render_widget(status_line(state.status.as_deref()), status_area);
}

fn status_line(status: Option<&str>) -> Paragraph<'static> {
    Paragraph::new(status.unwrap_or("").to_owned())
}

fn kind_label(kind: AttentionKind) -> &'static str {
    match kind {
        AttentionKind::Blocked => "BLOCKED",
        AttentionKind::Decision => "DECISION",
        AttentionKind::InboxMessage => "INBOX",
    }
}

fn queue_list(items: &[AttentionItem]) -> List<'static> {
    let rows: Vec<ListItem> = if items.is_empty() {
        vec![ListItem::new("(nothing needs attention)")]
    } else {
        items
            .iter()
            .map(|item| ListItem::new(format!("[{}] {}", kind_label(item.kind), item.summary)))
            .collect()
    };
    List::new(rows)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Attention Queue (j/k move, Enter jump, d done)"),
        )
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
}

fn section(title: &str, body: Option<&str>) -> Paragraph<'static> {
    let text = body.map_or_else(|| "(none)".to_owned(), str::to_owned);
    Paragraph::new(text).wrap(Wrap { trim: true }).block(
        Block::default()
            .borders(Borders::ALL)
            .title(title.to_owned()),
    )
}

fn decisions_section(focus: &FocusFile) -> Paragraph<'static> {
    let lines = if focus.decisions.is_empty() {
        vec![Line::from("(none)")]
    } else {
        focus
            .decisions
            .iter()
            .map(|decision| {
                let checkbox = if decision.resolved { "[x]" } else { "[ ]" };
                let style = if decision.resolved {
                    Style::default().add_modifier(Modifier::CROSSED_OUT)
                } else {
                    Style::default()
                };
                Line::from(Span::styled(format!("{checkbox} {}", decision.text), style))
            })
            .collect()
    };
    Paragraph::new(lines)
        .wrap(Wrap { trim: true })
        .block(Block::default().borders(Borders::ALL).title("Decisions"))
}

/// Render `state` into an off-screen buffer of the given size — the seam
/// tests use to assert on rendered content without a real terminal.
#[cfg(test)]
fn render_to_buffer(state: &AppState, width: u16, height: u16) -> ratatui::buffer::Buffer {
    use ratatui::backend::TestBackend;

    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test backend never fails to construct");
    terminal
        .draw(|frame| draw(frame, state))
        .expect("draw into TestBackend never fails");
    terminal.backend().buffer().clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::focusfile::DecisionEntry;

    fn buffer_text(buffer: &ratatui::buffer::Buffer) -> String {
        buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    }

    fn item(id: &str, kind: AttentionKind, summary: &str) -> AttentionItem {
        AttentionItem {
            id: id.to_owned(),
            kind,
            summary: summary.to_owned(),
            pane_id: None,
        }
    }

    #[test]
    fn empty_state_shows_placeholders() {
        let state = AppState {
            focus: FocusFile::default(),
            queue: vec![],
            selection: 0,
            status: None,
        };
        let text = buffer_text(&render_to_buffer(&state, 60, 16));
        assert!(text.contains("nothing needs attention"));
        assert_eq!(text.matches("(none)").count(), 3);
    }

    #[test]
    fn queue_items_and_focus_fields_render() {
        let state = AppState {
            focus: FocusFile {
                task: Some("Ship #86".to_owned()),
                next_action: Some("Wire the queue region".to_owned()),
                decisions: vec![DecisionEntry {
                    id: "a".to_owned(),
                    text: "Pending call".to_owned(),
                    resolved: false,
                }],
            },
            queue: vec![
                item("blocked:w1A:p1", AttentionKind::Blocked, "w1A:p1"),
                item(
                    "inbox:abc",
                    AttentionKind::InboxMessage,
                    "report from alpha",
                ),
            ],
            selection: 0,
            status: None,
        };
        let text = buffer_text(&render_to_buffer(&state, 70, 20));
        assert!(text.contains("[BLOCKED] w1A:p1"));
        assert!(text.contains("[INBOX] report from alpha"));
        assert!(text.contains("Ship #86"));
        assert!(text.contains("Wire the queue region"));
        assert!(text.contains("[ ] Pending call"));
    }

    #[test]
    fn status_message_renders_on_its_own_line() {
        let state = AppState {
            focus: FocusFile::default(),
            queue: vec![],
            selection: 0,
            status: Some("jump failed: pane not found".to_owned()),
        };
        let text = buffer_text(&render_to_buffer(&state, 60, 16));
        assert!(text.contains("jump failed: pane not found"));
    }

    #[test]
    fn j_and_k_map_to_move_down_and_up() {
        assert_eq!(apply_key(KeyCode::Char('j')), QueueAction::Move(1));
        assert_eq!(apply_key(KeyCode::Down), QueueAction::Move(1));
        assert_eq!(apply_key(KeyCode::Char('k')), QueueAction::Move(-1));
        assert_eq!(apply_key(KeyCode::Up), QueueAction::Move(-1));
    }

    #[test]
    fn enter_d_q_esc_map_to_expected_actions() {
        assert_eq!(apply_key(KeyCode::Enter), QueueAction::Jump);
        assert_eq!(apply_key(KeyCode::Char('d')), QueueAction::MarkDone);
        assert_eq!(apply_key(KeyCode::Char('q')), QueueAction::Quit);
        assert_eq!(apply_key(KeyCode::Esc), QueueAction::Quit);
    }

    #[test]
    fn unmapped_key_is_a_no_op() {
        assert_eq!(apply_key(KeyCode::Char('z')), QueueAction::None);
    }

    #[test]
    fn clamp_selection_stays_within_bounds() {
        assert_eq!(clamp_selection(0, 3, -1), 0);
        assert_eq!(clamp_selection(2, 3, 1), 2);
        assert_eq!(clamp_selection(1, 3, 1), 2);
        assert_eq!(clamp_selection(1, 3, -1), 0);
    }

    #[test]
    fn clamp_selection_on_empty_queue_is_always_zero() {
        assert_eq!(clamp_selection(0, 0, 1), 0);
        assert_eq!(clamp_selection(5, 0, -1), 0);
    }
}
