//! `jump` subcommand (D3, issue #86 commit 5): resolve the highest-priority
//! attention-queue item that has a pane to go to, across every known team,
//! and bring that pane's tab into view.
//!
//! Live-verified via `herdr pane --help` (2026-07-16): there is no
//! `pane focus <pane_id>` verb, only directional `pane focus --direction`
//! and `pane zoom <pane_id>`. `pane get`, `workspace focus`, and
//! `tab focus` DO accept an id directly, so the jump path is: `pane get`
//! (resolve `workspace_id`/`tab_id` for the target pane) → `workspace
//! focus` → `tab focus`. This reliably brings the correct tab into view;
//! herdr does not expose finer-than-tab input-focus control among several
//! panes sharing that tab — a documented gap (ADR-0012 degrade policy),
//! not a blocker, since it's still strictly better than a human hunting
//! for the pane by hand.
//!
//! Decision-kind attention items (unresolved focus-file entries) are never
//! a jump target: a decision has no pane, it lives in the focus file and
//! is the TUI's decision queue's job (commit 7), not something a human
//! "jumps to." `jump` skips straight to the highest-priority item that
//! *does* carry a `pane_id`.

use crate::attention::{self, AttentionItem, AttentionKind};
use crate::audit;
use crate::focusfile::{self, FocusFile, FocusFileError};
use crate::herdr::{HerdrApi, HerdrClient, HerdrError};
use crate::paths::{self, PathError};
use crate::pump;
use crate::teamfiles::{self, Teammate};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

const AUDIT_LOG_FILE: &str = "attention-audit.jsonl";

#[derive(Debug, Error)]
pub enum JumpError {
    #[error(transparent)]
    Path(#[from] PathError),
    #[error("cannot resolve the Claude Code team files directory: set HOME")]
    UnresolvedTeamsRoot,
    #[error(transparent)]
    Herdr(#[from] HerdrError),
    #[error(transparent)]
    FocusFile(#[from] FocusFileError),
    #[error(transparent)]
    Audit(#[from] audit::AuditLogError),
    #[error("no attention item has a pane to jump to")]
    NoJumpablePane,
}

pub fn jump_command(_args: &[String]) -> Result<(), JumpError> {
    let teams_root = pump::default_teams_root().map_err(|_| JumpError::UnresolvedTeamsRoot)?;
    let herdr = HerdrClient::from_env();
    let agents = herdr.agent_list()?;
    let focus = focusfile::read_focus_file(&default_focus_file_path())?;
    let team_leads = discover_team_leads(&teams_root, &agents);

    let audit_path = default_audit_log_path()?;
    let consumed = audit::read_consumed(&audit_path)?;
    let queue = audit::filter_unconsumed(
        merge_team_queues(&agents, &focus, &team_leads),
        &consumed,
        now_ms(),
    );

    let target = queue
        .into_iter()
        .find(|item| item.pane_id.is_some())
        .ok_or(JumpError::NoJumpablePane)?;
    let pane_id = target.pane_id.as_deref().expect("filtered for Some above");

    jump_to_pane(&herdr, pane_id)?;

    let _ = audit::append_consumed(&audit_path, &target.id, now_ms());
    Ok(())
}

/// `pane get` (resolve `workspace_id`/`tab_id`) → `workspace focus` →
/// `tab focus` — the 3-call sequence that stands in for the pane-focus-by-id
/// verb herdr doesn't have (see module docs). `pub(crate)`, generic over
/// [`HerdrApi`] so both the focus-pane TUI's Enter-to-jump action (#86
/// commit 7) and the mission-control board's jump affordance (#99) reuse
/// this instead of duplicating the call sequence — the board's terminal
/// loop is generic over `HerdrApi` for `FakeHerdr` testability, `jump`'s
/// own caller passes a concrete `HerdrClient`, both monomorphize fine.
pub(crate) fn jump_to_pane<H: HerdrApi>(herdr: &H, pane_id: &str) -> Result<(), HerdrError> {
    let pane = herdr.pane_get(pane_id)?;
    herdr.workspace_focus(&pane.workspace_id)?;
    if let Some(tab_id) = &pane.tab_id {
        herdr.tab_focus(tab_id)?;
    }
    Ok(())
}

/// `~/.local/share/herdmates/focus.md` — fixed, never overridable (see
/// `docs/focus-file.md`'s "not XDG-resolved, not overridable via env var"
/// ownership rule; unlike `HERDR_PLUGIN_STATE_DIR`, this is a stable
/// contract commitment, not a test seam). `pub(crate)` so the focus-pane
/// TUI (#86 commit 6) resolves the same fixed path rather than
/// duplicating this decision.
pub(crate) fn default_focus_file_path() -> PathBuf {
    let home = std::env::var_os("HOME").map_or_else(PathBuf::new, PathBuf::from);
    home.join(".local/share/herdmates/focus.md")
}

/// `pub(crate)` so the focus-pane TUI (#86 commit 7) logs consumed items to
/// the same audit file rather than picking its own path.
pub(crate) fn default_audit_log_path() -> Result<PathBuf, PathError> {
    Ok(paths::state_dir()?.join(AUDIT_LOG_FILE))
}

/// `pub(crate)` so the focus-pane TUI (#86 commit 7) stamps its own
/// audit-log writes the same way.
pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

/// Resolve every known team's lead Teammate + its herdr pane id (`None`
/// when unresolvable, matching `pump::pump_once`'s degrade policy — a team
/// with no resolvable lead just contributes no inbox items, never an error).
/// `pub(crate)` so the focus-pane TUI (#86 commit 7) reuses team discovery.
pub(crate) fn discover_team_leads(
    teams_root: &std::path::Path,
    agents: &[crate::herdr::AgentInfo],
) -> Vec<(Teammate, Option<String>)> {
    pump::discover_team_dirs(teams_root)
        .into_iter()
        .filter_map(|team_dir| {
            let config = teamfiles::read_team_config(&team_dir.join("config.json")).ok()?;
            let pane_id = pump::resolve_lead_pane(&config, agents);
            let inboxes = pump::read_inboxes(&team_dir.join("inboxes"));
            let teammates = teamfiles::build_teammates(&config, &inboxes);
            let lead = teammates.into_iter().find(|teammate| teammate.is_lead)?;
            Some((lead, pane_id))
        })
        .collect()
}

/// Merge attention sources across every known team into one ranked queue.
/// Blocked-worker and decision items are global (herdr agent status and the
/// focus file aren't per-team), so they're computed once; inbox items are
/// per-team (each team's lead has its own inbox), so
/// `attention::build_attention_queue` is called once per team-lead and only
/// its `InboxMessage` items are folded back in. Pure — no I/O — so it's
/// unit-testable with injected data. `pub(crate)` so the focus-pane TUI
/// (#86 commit 7) builds its live queue with the same merge logic.
pub(crate) fn merge_team_queues(
    agents: &[crate::herdr::AgentInfo],
    focus: &FocusFile,
    team_leads: &[(Teammate, Option<String>)],
) -> Vec<AttentionItem> {
    let mut items = attention::build_attention_queue(agents, focus, None, None);
    for (lead, pane_id) in team_leads {
        let per_team = attention::build_attention_queue(
            &[],
            &FocusFile::default(),
            Some(lead),
            pane_id.as_deref(),
        );
        items.extend(
            per_team
                .into_iter()
                .filter(|item| item.kind == AttentionKind::InboxMessage),
        );
    }
    items.sort_by_key(|item| item.kind);
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::focusfile::DecisionEntry;
    use crate::herdr::{AgentInfo, AgentSession};
    use crate::teamfiles::InboxMessage;

    fn agent(pane_id: &str, status: &str) -> AgentInfo {
        AgentInfo {
            pane_id: pane_id.to_owned(),
            workspace_id: "w1".to_owned(),
            agent: Some("claude".to_owned()),
            agent_id: None,
            agent_session: Some(AgentSession {
                source: "claude-code".to_owned(),
                agent: "claude".to_owned(),
                kind: "session".to_owned(),
                value: "session-1".to_owned(),
            }),
            status: Some(status.to_owned()),
        }
    }

    fn lead(name: &str, messages: Vec<InboxMessage>) -> Teammate {
        Teammate {
            name: name.to_owned(),
            agent_id: format!("{name}@t"),
            is_lead: true,
            tmux_pane_id: None,
            backend_type: None,
            is_active: true,
            model: None,
            task: None,
            inbox: messages,
        }
    }

    fn message(from: &str, content: &str) -> InboxMessage {
        InboxMessage {
            from_agent_id: Some(from.to_owned()),
            to_agent_id: None,
            content: Some(content.to_owned()),
        }
    }

    #[test]
    fn merge_with_no_teams_still_surfaces_global_items() {
        let agents = [agent("w1A:p1", "blocked")];
        let queue = merge_team_queues(&agents, &FocusFile::default(), &[]);
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].kind, AttentionKind::Blocked);
    }

    #[test]
    fn merge_folds_in_only_inbox_items_per_team_not_duplicated_globals() {
        let agents = [agent("w1A:p1", "blocked")];
        let focus = FocusFile {
            task: None,
            next_action: None,
            decisions: vec![DecisionEntry {
                id: "abc".to_owned(),
                text: "Pending".to_owned(),
                resolved: false,
            }],
        };
        let team_leads = vec![
            (
                lead("team-a-lead", vec![message("alpha@a", "report A")]),
                Some("w1A:p2".to_owned()),
            ),
            (
                lead("team-b-lead", vec![message("beta@b", "report B")]),
                Some("w1A:p3".to_owned()),
            ),
        ];

        let queue = merge_team_queues(&agents, &focus, &team_leads);

        // 1 blocked + 1 decision (each computed once, not once per team) + 2 inbox.
        assert_eq!(queue.len(), 4);
        assert_eq!(queue[0].kind, AttentionKind::Blocked);
        assert_eq!(queue[1].kind, AttentionKind::Decision);
        assert_eq!(queue[2].kind, AttentionKind::InboxMessage);
        assert_eq!(queue[3].kind, AttentionKind::InboxMessage);
        let inbox_panes = queue[2..]
            .iter()
            .filter_map(|item| item.pane_id.clone())
            .collect::<Vec<_>>();
        assert_eq!(inbox_panes, vec!["w1A:p2".to_owned(), "w1A:p3".to_owned()]);
    }

    #[test]
    fn merge_with_unresolved_lead_pane_still_includes_inbox_item_without_a_pane() {
        let team_leads = vec![(lead("solo-lead", vec![message("a@t", "hi")]), None)];
        let queue = merge_team_queues(&[], &FocusFile::default(), &team_leads);
        assert_eq!(queue.len(), 1);
        assert!(queue[0].pane_id.is_none());
    }
}
