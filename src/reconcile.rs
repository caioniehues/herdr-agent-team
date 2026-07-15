//! Pure hook event reconciliation for the run-board.

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
    TrackAgentStatus { worker_name: String, status: String },
    InjectPointer { worker_name: String, status: String },
    DrainOutbox { worker_name: String },
    MigratePane { worker_name: String },
    MigrateGodPane,
    MarkWorkerOrphaned { worker_name: String },
    EndWorker { worker_name: String },
    BindAgentIdentity { worker_name: String },
    EndRun,
}

/// Additive hook-owned metadata persisted as `[hook]` in `run.toml`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookMetadata {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub worker_status: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub worker_agent_identity: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub god_workspace_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reconciliation {
    pub state: RunState,
    pub metadata: HookMetadata,
    pub actions: Vec<ReconciliationAction>,
}

/// Reconcile one event against one active run without I/O or process spawning.
pub fn reconcile(
    event: &IncomingEvent,
    mut state: RunState,
    mut metadata: HookMetadata,
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
                    worker_name,
                    status: status.clone(),
                });
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
                actions.push(ReconciliationAction::MarkWorkerOrphaned { worker_name });
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

    Reconciliation {
        state,
        metadata,
        actions,
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
                    workspace_id: Some("workspace-1".to_owned()),
                    pane_id: Some("pane-1".to_owned()),
                    agent_id: None,
                    agent_session: None,
                    worktree_path: Some(PathBuf::from("/tmp/worktree")),
                    adopted: false,
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
}
