//! Pure hook event reconciliation for the run-board.

use crate::metadata::MetadataCapabilities;
use crate::types::{RunLifecycle, RunState, WorkerLifecycle};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IncomingEvent {
    AgentStatusChanged {
        pane_id: String,
        status: String,
    },
    PaneMoved {
        previous_pane_id: String,
        pane_id: String,
        workspace_id: Option<String>,
    },
    PaneExited {
        pane_id: String,
    },
    PaneClosed {
        pane_id: String,
    },
    WorkspaceClosed {
        workspace_id: String,
    },
    WorktreeRemoved {
        workspace_id: String,
        worktree_path: Option<PathBuf>,
    },
    PaneAgentDetected {
        pane_id: String,
        agent: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconciliationAction {
    RecordEvent,
    TrackAgentStatus {
        worker_name: String,
        status: String,
    },
    InjectPointer {
        worker_name: String,
        status: String,
    },
    DrainOutbox {
        worker_name: String,
    },
    MigratePane {
        worker_name: String,
    },
    MigrateGodPane,
    MarkWorkerOrphaned {
        worker_name: String,
    },
    EndWorker {
        worker_name: String,
    },
    BindAgentIdentity {
        worker_name: String,
    },
    PublishMetadata {
        worker_name: String,
        status: String,
        sequence: u64,
        attention: bool,
    },
    Notify {
        title: String,
        body: String,
        sound: String,
    },
    EndRun,
}

/// Additive hook-owned metadata persisted as `[hook]` in `run.toml`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookMetadata {
    /// Last report modification time acknowledged by `report` (spec section 13).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub report_read_mtime_ms: BTreeMap<String, u64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub worker_status: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub worker_agent_identity: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub god_workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata_capabilities: Option<MetadataCapabilities>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata_sequence: BTreeMap<String, u64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub blocked_since_ms: BTreeMap<String, u64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attention_pending: BTreeMap<String, bool>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub aggregate_notifications: BTreeMap<String, bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reconciliation {
    pub state: RunState,
    pub metadata: HookMetadata,
    pub actions: Vec<ReconciliationAction>,
}

/// Reconcile one event against one active run without I/O or process spawning.
pub fn reconcile(event: &IncomingEvent, state: RunState, metadata: HookMetadata) -> Reconciliation {
    reconcile_at(event, state, metadata, 0, 0)
}

/// Pure reconciliation with an injected clock for the blocked-duration policy.
pub fn reconcile_at(
    event: &IncomingEvent,
    mut state: RunState,
    mut metadata: HookMetadata,
    now_ms: u64,
    blocked_threshold_ms: u64,
) -> Reconciliation {
    let mut actions = Vec::new();
    let target = match event {
        IncomingEvent::AgentStatusChanged { pane_id, .. }
        | IncomingEvent::PaneExited { pane_id }
        | IncomingEvent::PaneClosed { pane_id }
        | IncomingEvent::PaneAgentDetected { pane_id, .. } => target_for_pane(&state, pane_id),
        IncomingEvent::PaneMoved {
            previous_pane_id, ..
        } => target_for_pane(&state, previous_pane_id),
        IncomingEvent::WorkspaceClosed { workspace_id } => {
            worker_for_workspace(&state, workspace_id)
                .map(Target::Worker)
                .or_else(|| {
                    (metadata.god_workspace_id.as_deref() == Some(workspace_id))
                        .then_some(Target::God)
                })
        }
        IncomingEvent::WorktreeRemoved {
            workspace_id,
            worktree_path,
        } => worker_for_workspace(&state, workspace_id)
            .or_else(|| {
                worktree_path
                    .as_ref()
                    .and_then(|path| worker_for_worktree(&state, path))
            })
            .map(Target::Worker)
            .or_else(|| {
                (metadata.god_workspace_id.as_deref() == Some(workspace_id)).then_some(Target::God)
            }),
    };

    let Some(target) = target else {
        return Reconciliation {
            state,
            metadata,
            actions,
        };
    };
    actions.push(ReconciliationAction::RecordEvent);

    match event {
        IncomingEvent::AgentStatusChanged { status, .. } => {
            let Target::Worker(worker_name) = target else {
                sweep_blocked_workers(
                    &state,
                    &mut metadata,
                    &mut actions,
                    now_ms,
                    blocked_threshold_ms,
                );
                return Reconciliation {
                    state,
                    metadata,
                    actions,
                };
            };
            let previous = metadata
                .worker_status
                .insert(worker_name.clone(), status.clone());
            actions.push(ReconciliationAction::TrackAgentStatus {
                worker_name: worker_name.clone(),
                status: status.clone(),
            });
            let sequence = metadata
                .metadata_sequence
                .entry(worker_name.clone())
                .or_insert(0);
            *sequence += 1;
            let attention = metadata.attention_pending.remove(&worker_name).is_some();
            actions.push(ReconciliationAction::PublishMetadata {
                worker_name: worker_name.clone(),
                status: status.clone(),
                sequence: *sequence,
                attention,
            });
            if status == "idle" || status == "done" {
                actions.push(ReconciliationAction::DrainOutbox {
                    worker_name: worker_name.clone(),
                });
            }
            if status == "blocked"
                || (status == "idle" && previous.as_deref() == Some("working"))
                || (status == "done" && previous.as_deref() == Some("working"))
            {
                actions.push(ReconciliationAction::InjectPointer {
                    worker_name: worker_name.clone(),
                    status: status.clone(),
                });
            }
            if status == "blocked" {
                metadata
                    .blocked_since_ms
                    .entry(worker_name.clone())
                    .or_insert(now_ms);
            } else {
                metadata.blocked_since_ms.remove(&worker_name);
            }
            if state.workers.keys().all(|name| {
                matches!(
                    metadata.worker_status.get(name).map(String::as_str),
                    Some("idle" | "done")
                )
            }) {
                notify_once(
                    &mut metadata,
                    &mut actions,
                    "team-complete".to_owned(),
                    "Team complete",
                    format!(
                        "All workers in {} reached a terminal status.",
                        state.spec.name
                    ),
                    "done",
                );
            }
        }
        IncomingEvent::PaneMoved {
            pane_id,
            workspace_id,
            ..
        } => match target {
            Target::Worker(worker_name) => {
                let worker = state
                    .workers
                    .get_mut(&worker_name)
                    .expect("matched worker exists");
                worker.pane_id = Some(pane_id.clone());
                if let Some(workspace_id) = workspace_id {
                    worker.workspace_id = Some(workspace_id.clone());
                }
                actions.push(ReconciliationAction::MigratePane { worker_name });
            }
            Target::God => {
                state.god_pane_id = pane_id.clone();
                metadata.god_workspace_id = workspace_id.clone();
                actions.push(ReconciliationAction::MigrateGodPane);
            }
        },
        IncomingEvent::PaneExited { .. } | IncomingEvent::PaneClosed { .. } => match target {
            Target::Worker(worker_name) => {
                state
                    .workers
                    .get_mut(&worker_name)
                    .expect("matched worker exists")
                    .lifecycle = WorkerLifecycle::Orphaned;
                actions.push(ReconciliationAction::MarkWorkerOrphaned {
                    worker_name: worker_name.clone(),
                });
                notify_once(
                    &mut metadata,
                    &mut actions,
                    format!("exit:{worker_name}"),
                    "Worker exited",
                    format!("{} exited before the team released it.", worker_name),
                    "request",
                );
                end_run_if_no_live_workers(&mut state, &mut actions);
            }
            Target::God => end_run(&mut state, &mut actions),
        },
        IncomingEvent::WorkspaceClosed { .. } | IncomingEvent::WorktreeRemoved { .. } => {
            match target {
                Target::Worker(worker_name) => {
                    state
                        .workers
                        .get_mut(&worker_name)
                        .expect("matched worker exists")
                        .lifecycle = WorkerLifecycle::Ended;
                    actions.push(ReconciliationAction::EndWorker { worker_name });
                    end_run_if_no_live_workers(&mut state, &mut actions);
                }
                Target::God => end_run(&mut state, &mut actions),
            }
        }
        IncomingEvent::PaneAgentDetected { agent, .. } => {
            let Target::Worker(worker_name) = target else {
                sweep_blocked_workers(
                    &state,
                    &mut metadata,
                    &mut actions,
                    now_ms,
                    blocked_threshold_ms,
                );
                return Reconciliation {
                    state,
                    metadata,
                    actions,
                };
            };
            if let Some(agent) = agent {
                metadata
                    .worker_agent_identity
                    .insert(worker_name.clone(), agent.clone());
                actions.push(ReconciliationAction::BindAgentIdentity { worker_name });
            }
        }
    }

    sweep_blocked_workers(
        &state,
        &mut metadata,
        &mut actions,
        now_ms,
        blocked_threshold_ms,
    );

    Reconciliation {
        state,
        metadata,
        actions,
    }
}

fn sweep_blocked_workers(
    state: &RunState,
    metadata: &mut HookMetadata,
    actions: &mut Vec<ReconciliationAction>,
    now_ms: u64,
    blocked_threshold_ms: u64,
) {
    for (worker_name, blocked_since) in metadata.blocked_since_ms.clone() {
        if now_ms.saturating_sub(blocked_since) >= blocked_threshold_ms {
            notify_once(
                metadata,
                actions,
                format!("blocked:{worker_name}"),
                "Worker blocked",
                format!(
                    "{worker_name} has remained blocked beyond the configured threshold in {}.",
                    state.spec.name
                ),
                "request",
            );
        }
    }
}

fn notify_once(
    metadata: &mut HookMetadata,
    actions: &mut Vec<ReconciliationAction>,
    key: String,
    title: &str,
    body: String,
    sound: &str,
) {
    if metadata.aggregate_notifications.insert(key, true).is_none() {
        actions.push(ReconciliationAction::Notify {
            title: title.to_owned(),
            body,
            sound: sound.to_owned(),
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    Worker(String),
    God,
}

fn target_for_pane(state: &RunState, pane_id: &str) -> Option<Target> {
    if state.god_pane_id == pane_id {
        Some(Target::God)
    } else {
        worker_for_pane(state, pane_id).map(Target::Worker)
    }
}

fn end_run_if_no_live_workers(state: &mut RunState, actions: &mut Vec<ReconciliationAction>) {
    if state.workers.values().all(|worker| {
        matches!(
            worker.lifecycle,
            WorkerLifecycle::Failed
                | WorkerLifecycle::Ended
                | WorkerLifecycle::Released
                | WorkerLifecycle::Orphaned
        )
    }) {
        end_run(state, actions);
    }
}

fn end_run(state: &mut RunState, actions: &mut Vec<ReconciliationAction>) {
    state.lifecycle = RunLifecycle::Ended;
    actions.push(ReconciliationAction::EndRun);
}

fn worker_for_pane(state: &RunState, pane_id: &str) -> Option<String> {
    state.workers.iter().find_map(|(name, worker)| {
        (worker.pane_id.as_deref() == Some(pane_id)).then(|| name.clone())
    })
}

fn worker_for_workspace(state: &RunState, workspace_id: &str) -> Option<String> {
    state.workers.iter().find_map(|(name, worker)| {
        (worker.workspace_id.as_deref() == Some(workspace_id)).then(|| name.clone())
    })
}

fn worker_for_worktree(state: &RunState, worktree_path: &std::path::Path) -> Option<String> {
    state.workers.iter().find_map(|(name, worker)| {
        (worker.worktree_path.as_deref() == Some(worktree_path)).then(|| name.clone())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{GodSpec, TeamSpec, Topology, WorkerRunState, WorkerSpec};
    use std::collections::BTreeMap;

    fn state() -> RunState {
        RunState {
            spec: TeamSpec {
                name: "alpha".to_owned(),
                topology: Topology::Star,
                cwd: PathBuf::from("/tmp"),
                setup: vec![],
                god: GodSpec {
                    target: "self".to_owned(),
                },
                workers: vec![WorkerSpec {
                    name: "builder".to_owned(),
                    agent: "codex".to_owned(),
                    role: "build".to_owned(),
                    task: None,
                    worktree: true,
                    branch: None,
                    brief: PathBuf::from("brief.md"),
                }],
            },
            god_pane_id: "god".to_owned(),
            herdr_session: Default::default(),
            lifecycle: RunLifecycle::Active,
            workers: BTreeMap::from([(
                "builder".to_owned(),
                WorkerRunState {
                    task: None,
                    workspace_id: Some("workspace-1".to_owned()),
                    pane_id: Some("pane-1".to_owned()),
                    agent_id: None,
                    agent_session: None,
                    worktree_path: Some(PathBuf::from("/tmp/worktree")),
                    adopted: false,
                    launch_checkpoint: Default::default(),
                    lifecycle: WorkerLifecycle::Running,
                },
            )]),
        }
    }

    #[test]
    fn completion_pointers_are_at_most_once_per_completion_transition() {
        let working = reconcile(
            &IncomingEvent::AgentStatusChanged {
                pane_id: "pane-1".into(),
                status: "working".into(),
            },
            state(),
            HookMetadata::default(),
        );
        let idle = reconcile(
            &IncomingEvent::AgentStatusChanged {
                pane_id: "pane-1".into(),
                status: "idle".into(),
            },
            working.state,
            working.metadata,
        );
        let repeated_idle = reconcile(
            &IncomingEvent::AgentStatusChanged {
                pane_id: "pane-1".into(),
                status: "idle".into(),
            },
            idle.state.clone(),
            idle.metadata.clone(),
        );
        let unseen_working = reconcile(
            &IncomingEvent::AgentStatusChanged {
                pane_id: "pane-1".into(),
                status: "working".into(),
            },
            state(),
            HookMetadata::default(),
        );
        let unseen_done = reconcile(
            &IncomingEvent::AgentStatusChanged {
                pane_id: "pane-1".into(),
                status: "done".into(),
            },
            unseen_working.state,
            unseen_working.metadata,
        );

        assert!(idle.actions.iter().any(|action| matches!(action, ReconciliationAction::InjectPointer { status, .. } if status == "idle")));
        assert!(!repeated_idle
            .actions
            .iter()
            .any(|action| matches!(action, ReconciliationAction::InjectPointer { .. })));
        assert!(unseen_done.actions.iter().any(|action| matches!(action, ReconciliationAction::InjectPointer { status, .. } if status == "done")));
    }

    #[test]
    fn lifecycle_events_update_the_run_board() {
        let moved = reconcile(
            &IncomingEvent::PaneMoved {
                previous_pane_id: "pane-1".into(),
                pane_id: "pane-2".into(),
                workspace_id: Some("workspace-2".into()),
            },
            state(),
            HookMetadata::default(),
        );
        assert_eq!(
            moved.state.workers["builder"].pane_id.as_deref(),
            Some("pane-2")
        );
        assert_eq!(
            moved.state.workers["builder"].workspace_id.as_deref(),
            Some("workspace-2")
        );
        for event in [
            IncomingEvent::PaneExited {
                pane_id: "pane-1".into(),
            },
            IncomingEvent::PaneClosed {
                pane_id: "pane-1".into(),
            },
        ] {
            assert_eq!(
                reconcile(&event, state(), HookMetadata::default())
                    .state
                    .workers["builder"]
                    .lifecycle,
                WorkerLifecycle::Orphaned
            );
        }
        assert_eq!(
            reconcile(
                &IncomingEvent::WorkspaceClosed {
                    workspace_id: "workspace-1".into()
                },
                state(),
                HookMetadata::default()
            )
            .state
            .lifecycle,
            RunLifecycle::Ended
        );
        assert_eq!(
            reconcile(
                &IncomingEvent::WorktreeRemoved {
                    workspace_id: "other".into(),
                    worktree_path: Some(PathBuf::from("/tmp/worktree"))
                },
                state(),
                HookMetadata::default()
            )
            .state
            .lifecycle,
            RunLifecycle::Ended
        );
    }

    #[test]
    fn metadata_sequences_and_aggregate_notifications_are_at_most_once() {
        let working = reconcile(
            &IncomingEvent::AgentStatusChanged {
                pane_id: "pane-1".into(),
                status: "working".into(),
            },
            state(),
            HookMetadata::default(),
        );
        let blocked = reconcile(
            &IncomingEvent::AgentStatusChanged {
                pane_id: "pane-1".into(),
                status: "blocked".into(),
            },
            working.state,
            working.metadata,
        );
        let repeated_blocked = reconcile(
            &IncomingEvent::AgentStatusChanged {
                pane_id: "pane-1".into(),
                status: "blocked".into(),
            },
            blocked.state,
            blocked.metadata,
        );
        assert_eq!(repeated_blocked.metadata.metadata_sequence["builder"], 3);
        assert_eq!(
            repeated_blocked
                .metadata
                .aggregate_notifications
                .get("blocked:builder"),
            Some(&true),
        );
        assert_eq!(
            repeated_blocked
                .actions
                .iter()
                .filter(|action| matches!(action, ReconciliationAction::Notify { .. }))
                .count(),
            0,
        );

        let exited = reconcile(
            &IncomingEvent::PaneExited {
                pane_id: "pane-1".into(),
            },
            repeated_blocked.state,
            repeated_blocked.metadata,
        );
        let repeated_exit = reconcile(
            &IncomingEvent::PaneExited {
                pane_id: "pane-1".into(),
            },
            exited.state,
            exited.metadata,
        );
        assert!(repeated_exit
            .metadata
            .aggregate_notifications
            .contains_key("exit:builder"));
        assert_eq!(
            repeated_exit
                .actions
                .iter()
                .filter(|action| matches!(action, ReconciliationAction::Notify { .. }))
                .count(),
            0,
        );

        let completed = reconcile(
            &IncomingEvent::AgentStatusChanged {
                pane_id: "pane-1".into(),
                status: "idle".into(),
            },
            state(),
            HookMetadata::default(),
        );
        assert!(completed
            .metadata
            .aggregate_notifications
            .contains_key("team-complete"));
    }

    #[test]
    fn blocked_notification_waits_for_the_configured_duration() {
        let first = reconcile_at(
            &IncomingEvent::AgentStatusChanged {
                pane_id: "pane-1".into(),
                status: "blocked".into(),
            },
            state(),
            HookMetadata::default(),
            100,
            1_000,
        );
        assert!(!first
            .actions
            .iter()
            .any(|action| matches!(action, ReconciliationAction::Notify { .. })));
        let before_threshold = reconcile_at(
            &IncomingEvent::AgentStatusChanged {
                pane_id: "pane-1".into(),
                status: "blocked".into(),
            },
            first.state,
            first.metadata,
            1_099,
            1_000,
        );
        assert!(!before_threshold
            .actions
            .iter()
            .any(|action| matches!(action, ReconciliationAction::Notify { .. })));
        let elapsed = reconcile_at(
            &IncomingEvent::AgentStatusChanged {
                pane_id: "pane-1".into(),
                status: "blocked".into(),
            },
            before_threshold.state,
            before_threshold.metadata,
            1_100,
            1_000,
        );
        assert_eq!(
            elapsed
                .actions
                .iter()
                .filter(|action| matches!(action, ReconciliationAction::Notify { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn unrelated_worker_event_sweeps_blocked_duration() {
        let mut two_workers = state();
        two_workers.spec.workers.push(WorkerSpec {
            name: "reviewer".to_owned(),
            agent: "claude".to_owned(),
            role: "review".to_owned(),
            task: None,
            worktree: false,
            branch: None,
            brief: PathBuf::from("review.md"),
        });
        two_workers.workers.insert(
            "reviewer".to_owned(),
            WorkerRunState {
                task: None,
                workspace_id: Some("workspace-2".to_owned()),
                pane_id: Some("pane-2".to_owned()),
                agent_id: None,
                agent_session: None,
                worktree_path: None,
                adopted: false,
                launch_checkpoint: Default::default(),
                lifecycle: WorkerLifecycle::Running,
            },
        );
        let blocked = reconcile_at(
            &IncomingEvent::AgentStatusChanged {
                pane_id: "pane-1".into(),
                status: "blocked".into(),
            },
            two_workers,
            HookMetadata::default(),
            100,
            1_000,
        );
        let later_reviewer_event = reconcile_at(
            &IncomingEvent::AgentStatusChanged {
                pane_id: "pane-2".into(),
                status: "working".into(),
            },
            blocked.state,
            blocked.metadata,
            1_100,
            1_000,
        );
        assert!(later_reviewer_event.actions.iter().any(|action| matches!(
            action,
            ReconciliationAction::Notify { title, .. } if title == "Worker blocked"
        )));
    }
}
