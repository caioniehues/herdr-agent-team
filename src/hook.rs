//! Push-based worker status report and outbox hook from `docs/spec.md` sections 5 and 11.

use crate::herdr::{HerdrApi, HerdrClient};
use crate::metadata::{map_facts, MetadataCapabilities, MetadataFacts};
use crate::msg;
use crate::reconcile::{reconcile_at, IncomingEvent, ReconciliationAction};
use crate::run;
use serde_json::{json, Value};
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum HookError {
    #[error("required environment variable {0} is not set or is not valid Unicode")]
    MissingEnvironment(&'static str),

    #[error("invalid hook event JSON: {0}")]
    InvalidEvent(#[from] serde_json::Error),

    #[error("unsupported or malformed hook event `{actual}`")]
    UnexpectedEvent { field: &'static str, actual: String },

    #[error("failed to resolve an absolute report path: {0}")]
    CurrentDirectory(#[from] std::io::Error),

    #[error("failed to {action} `{path}`: {source}")]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(transparent)]
    Run(#[from] run::RunError),

    #[error(transparent)]
    Herdr(#[from] crate::herdr::HerdrError),
}

#[derive(Debug, serde::Deserialize)]
struct EventEnvelope {
    event: String,
    data: Value,
}

pub fn hook_command() -> Result<(), HookError> {
    let event_json = std::env::var("HERDR_PLUGIN_EVENT_JSON")
        .map_err(|_| HookError::MissingEnvironment("HERDR_PLUGIN_EVENT_JSON"))?;
    let state_dir = std::env::var("HERDR_PLUGIN_STATE_DIR")
        .map(PathBuf::from)
        .map_err(|_| HookError::MissingEnvironment("HERDR_PLUGIN_STATE_DIR"))?;
    on_agent_status(&event_json, &state_dir, &HerdrClient::from_env())
}

pub fn on_agent_status<H: HerdrApi>(
    event_json: &str,
    state_dir: &Path,
    herdr: &H,
) -> Result<(), HookError> {
    let raw_event: Value = serde_json::from_str(event_json)?;
    let event = parse_event(serde_json::from_value(raw_event.clone())?)?;

    for listed_run in run::list_active_runs(state_dir)? {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(run::RunError::from)?
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        let blocked_threshold_ms = std::env::var("HERDR_AGENT_TEAM_BLOCKED_THRESHOLD_MS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(300_000);
        let (run, reconciliation) =
            run::update_run_with_hook(&listed_run.dir, |run, metadata| -> Result<_, HookError> {
                let mut reconciliation = reconcile_at(
                    &event,
                    run.state.clone(),
                    metadata.clone(),
                    now_ms,
                    blocked_threshold_ms,
                );
                if reconciliation.metadata.metadata_capabilities.is_none()
                    && reconciliation.actions.iter().any(|action| {
                        matches!(action, ReconciliationAction::PublishMetadata { .. })
                    })
                {
                    reconciliation.metadata.metadata_capabilities =
                        Some(MetadataCapabilities::from_schema(&herdr.api_schema()?));
                }
                run.state = reconciliation.state.clone();
                *metadata = reconciliation.metadata.clone();
                Ok(reconciliation)
            })?;
        if reconciliation.actions.is_empty() {
            continue;
        }
        run::append_event(&run.dir, &raw_event)?;

        for action in reconciliation.actions {
            match action {
                ReconciliationAction::DrainOutbox { worker_name } => {
                    drain_outbox(&run.dir, &worker_name, |text| {
                        msg::deliver_queued_message(&run, &worker_name, text, herdr)
                    })?;
                }
                ReconciliationAction::InjectPointer {
                    worker_name,
                    status,
                } => {
                    inject_pointer(&run, &worker_name, &status, herdr)?;
                }
                ReconciliationAction::PublishMetadata {
                    worker_name,
                    status,
                    sequence,
                    attention,
                } => {
                    let Some(capabilities) = reconciliation.metadata.metadata_capabilities.as_ref()
                    else {
                        continue;
                    };
                    let worker = run
                        .state
                        .spec
                        .workers
                        .iter()
                        .find(|worker| worker.name == worker_name)
                        .expect("reconciled worker is in spec");
                    if let Some(update) = map_facts(
                        MetadataFacts {
                            team: &run.state.spec.name,
                            role: &worker.role,
                            task: run
                                .state
                                .workers
                                .get(&worker_name)
                                .and_then(|worker| worker.task.as_deref())
                                .or(worker.task.as_deref()),
                            status: &status,
                            attention,
                        },
                        capabilities,
                        sequence,
                    ) {
                        if let Some(pane_id) = run
                            .state
                            .workers
                            .get(&worker_name)
                            .and_then(|worker| worker.pane_id.as_deref())
                        {
                            herdr.pane_report_metadata(pane_id, &update)?;
                        }
                    }
                }
                ReconciliationAction::Notify { title, body, sound } => {
                    herdr.notification_show(&title, &body, &sound)?
                }
                ReconciliationAction::RecordEvent
                | ReconciliationAction::TrackAgentStatus { .. }
                | ReconciliationAction::MigratePane { .. }
                | ReconciliationAction::MigrateGodPane
                | ReconciliationAction::MarkWorkerOrphaned { .. }
                | ReconciliationAction::EndWorker { .. }
                | ReconciliationAction::BindAgentIdentity { .. }
                | ReconciliationAction::EndRun => {}
            }
        }
    }
    Ok(())
}

fn parse_event(envelope: EventEnvelope) -> Result<IncomingEvent, HookError> {
    let required = |field: &'static str| required_string(&envelope.data, field, &envelope.event);
    match envelope.event.as_str() {
        "pane_agent_status_changed" => Ok(IncomingEvent::AgentStatusChanged {
            pane_id: required("pane_id")?,
            status: required("agent_status")?,
        }),
        "pane_moved" => Ok(IncomingEvent::PaneMoved {
            previous_pane_id: required("previous_pane_id")?,
            pane_id: envelope
                .data
                .get("pane")
                .and_then(|pane| pane.get("pane_id"))
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| HookError::UnexpectedEvent {
                    field: "data.pane.pane_id",
                    actual: envelope.event.clone(),
                })?,
            workspace_id: envelope
                .data
                .get("pane")
                .and_then(|pane| pane.get("workspace_id"))
                .and_then(Value::as_str)
                .map(str::to_owned),
        }),
        "pane_exited" => Ok(IncomingEvent::PaneExited {
            pane_id: required("pane_id")?,
        }),
        "pane_closed" => Ok(IncomingEvent::PaneClosed {
            pane_id: required("pane_id")?,
        }),
        "workspace_closed" => Ok(IncomingEvent::WorkspaceClosed {
            workspace_id: required("workspace_id")?,
        }),
        "worktree_removed" => Ok(IncomingEvent::WorktreeRemoved {
            workspace_id: required("workspace_id")?,
            worktree_path: envelope
                .data
                .get("worktree")
                .and_then(|worktree| worktree.get("path"))
                .and_then(Value::as_str)
                .map(PathBuf::from),
        }),
        "pane_agent_detected" => Ok(IncomingEvent::PaneAgentDetected {
            pane_id: required("pane_id")?,
            agent: envelope
                .data
                .get("agent")
                .and_then(Value::as_str)
                .map(str::to_owned),
        }),
        _ => Err(HookError::UnexpectedEvent {
            field: "event",
            actual: envelope.event,
        }),
    }
}

fn required_string(data: &Value, field: &'static str, event: &str) -> Result<String, HookError> {
    data.get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| HookError::UnexpectedEvent {
            field,
            actual: event.to_owned(),
        })
}

fn inject_pointer<H: HerdrApi>(
    run: &run::RunBoard,
    worker_name: &str,
    status: &str,
    herdr: &H,
) -> Result<(), HookError> {
    let report_path = absolute_path(&run.dir.join("inbox").join(format!("{worker_name}.md")))?;
    let pointer = format!(
        "[team {}] {} is {} — report: {}",
        run.state.spec.name,
        worker_name,
        status,
        report_path.display()
    );
    herdr.pane_run(&run.state.god_pane_id, &pointer)?;
    Ok(())
}

fn drain_outbox<E, F>(run_dir: &Path, target: &str, mut deliver: F) -> Result<(), HookError>
where
    E: Display,
    F: FnMut(&str) -> Result<(), E>,
{
    for path in queued_message_paths(run_dir, target)? {
        // Atomically claim the message before consuming it. Concurrent hook
        // invocations both list the same queued entry (defect #59); the rename
        // winner owns delivery, and a loser's ENOENT means already-claimed —
        // not a delivery failure — so it must skip silently.
        let claimed = path.with_extension("claim");
        if let Err(error) = std::fs::rename(&path, &claimed) {
            if error.kind() == std::io::ErrorKind::NotFound {
                continue;
            }
            append_delivery_event(
                run_dir,
                "delivery_failed",
                target,
                &path,
                Some(&error.to_string()),
            )?;
            break;
        }

        let text = match std::fs::read_to_string(&claimed) {
            Ok(text) => text,
            Err(error) => {
                // Best-effort requeue so the message is retried on a later drain.
                let _ = std::fs::rename(&claimed, &path);
                append_delivery_event(
                    run_dir,
                    "delivery_failed",
                    target,
                    &path,
                    Some(&error.to_string()),
                )?;
                break;
            }
        };

        if let Err(error) = deliver(&text) {
            // Best-effort requeue so the message is retried on a later drain.
            let _ = std::fs::rename(&claimed, &path);
            append_delivery_event(
                run_dir,
                "delivery_failed",
                target,
                &path,
                Some(&error.to_string()),
            )?;
            break;
        }

        if let Err(error) = std::fs::remove_file(&claimed) {
            // After a successful claim the file is exclusively ours: ENOENT
            // here is already-consumed, never a delivery failure.
            if error.kind() != std::io::ErrorKind::NotFound {
                append_delivery_event(
                    run_dir,
                    "delivery_failed",
                    target,
                    &path,
                    Some(&error.to_string()),
                )?;
                break;
            }
        }
        append_delivery_event(run_dir, "delivered", target, &path, None)?;
    }
    Ok(())
}

fn queued_message_paths(run_dir: &Path, target: &str) -> Result<Vec<PathBuf>, HookError> {
    let outbox_dir = run_dir.join("outbox").join(target);
    let entries = match std::fs::read_dir(&outbox_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(HookError::Io {
                action: "read message outbox",
                path: outbox_dir,
                source,
            })
        }
    };

    let mut messages = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| HookError::Io {
            action: "read message outbox entry",
            path: outbox_dir.clone(),
            source,
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|source| HookError::Io {
            action: "inspect queued message",
            path: path.clone(),
            source,
        })?;
        if !file_type.is_file() {
            continue;
        }
        let Some(sequence) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.strip_suffix(".msg"))
            .and_then(|sequence| sequence.parse::<u64>().ok())
        else {
            continue;
        };
        messages.push((sequence, path));
    }
    messages.sort_unstable();
    Ok(messages.into_iter().map(|(_, path)| path).collect())
}

fn append_delivery_event(
    run_dir: &Path,
    kind: &str,
    target: &str,
    path: &Path,
    error: Option<&str>,
) -> Result<(), HookError> {
    let mut event = json!({
        "event": kind,
        "target": target,
        "path": path.display().to_string(),
    });
    if let Some(error) = error {
        event["error"] = Value::String(error.to_owned());
    }
    run::append_event(run_dir, &event)?;
    Ok(())
}

fn absolute_path(path: &Path) -> Result<PathBuf, std::io::Error> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::{create_run, RunBoard};
    use crate::types::{
        GodSpec, RunLifecycle, RunState, TeamSpec, Topology, WorkerLifecycle, WorkerRunState,
        WorkerSpec,
    };
    use std::collections::BTreeMap;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
    const CAPTURED_EVENT: &str = r#"{"event":"pane_agent_status_changed","data":{"type":"pane_agent_status_changed","pane_id":"wG:p2","workspace_id":"wG","agent_status":"idle","agent":"claude"}}"#;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("test clock should be after Unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "herdr-hook-tests-{}-{nanos}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("create hook test directory");
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

    struct HookFixture {
        client: crate::herdr::test_support::FakeHerdr,
    }

    impl HookFixture {
        fn new(_: &TempDir) -> Self {
            Self {
                client: crate::herdr::test_support::FakeHerdr::default(),
            }
        }

        fn argv(&self) -> Vec<String> {
            self.client
                .calls()
                .into_iter()
                .flat_map(|call| {
                    if let Some(rest) = call.strip_prefix("pane_run:") {
                        let (pane, input) = rest.split_once(':').expect("pane run call shape");
                        return vec!["pane", "run", pane, input]
                            .into_iter()
                            .map(str::to_owned)
                            .collect();
                    }
                    if let Some(rest) = call.strip_prefix("agent_wait:") {
                        let (pane, status) = rest.split_once(':').expect("agent wait call shape");
                        return vec!["agent", "wait", pane, "--status", status]
                            .into_iter()
                            .map(str::to_owned)
                            .collect();
                    }
                    if let Some(rest) = call.strip_prefix("notification:") {
                        let mut parts = rest.splitn(3, ':');
                        let title = parts.next().expect("notification title");
                        let body = parts.next().expect("notification body");
                        let sound = parts.next().expect("notification sound");
                        return vec![
                            "notification",
                            "show",
                            title,
                            "--body",
                            body,
                            "--sound",
                            sound,
                        ]
                        .into_iter()
                        .map(str::to_owned)
                        .collect();
                    }
                    if call == "api_schema" {
                        return vec!["api".to_owned(), "schema".to_owned(), "--json".to_owned()];
                    }
                    Vec::new()
                })
                .collect()
        }
    }

    type FakeHerdr = HookFixture;

    fn fixture_run(state_dir: &Path, worker_pane: &str) -> RunBoard {
        let worker_name = "builder".to_owned();
        create_run(
            state_dir,
            RunState {
                spec: TeamSpec {
                    name: "alpha".to_owned(),
                    topology: Topology::Star,
                    cwd: PathBuf::from("/tmp/project"),
                    setup: Vec::new(),
                    god: GodSpec {
                        target: "self".to_owned(),
                    },
                    workers: vec![WorkerSpec {
                        name: worker_name.clone(),
                        agent: "codex".to_owned(),
                        role: "builder".to_owned(),
                        task: None,
                        worktree: false,
                        branch: None,
                        brief: PathBuf::from("brief.md"),
                    }],
                },
                god_pane_id: "god-pane".to_owned(),
                herdr_session: Default::default(),
                workers: BTreeMap::from([(
                    worker_name,
                    WorkerRunState {
                        task: None,
                        workspace_id: Some("worker-workspace".to_owned()),
                        pane_id: Some(worker_pane.to_owned()),
                        agent_id: Some("agent-1".to_owned()),
                        agent_session: None,
                        worktree_path: None,
                        adopted: false,
                        launch_checkpoint: Default::default(),
                        lifecycle: WorkerLifecycle::Running,
                    },
                )]),
                lifecycle: RunLifecycle::Active,
            },
        )
        .expect("create hook fixture run")
    }

    fn event(pane_id: &str, status: &str) -> Value {
        json!({
            "event": "pane_agent_status_changed",
            "data": {
                "type": "pane_agent_status_changed",
                "pane_id": pane_id,
                "workspace_id": "worker-workspace",
                "agent_status": status,
                "agent": "codex",
                "custom_status": null,
                "display_agent": null,
                "title": null,
                "state_labels": {"phase": "verification"}
            }
        })
    }

    fn queue_message(run: &RunBoard, sequence: u64, text: &str) -> PathBuf {
        let outbox = run.dir.join("outbox/builder");
        fs::create_dir_all(&outbox).expect("create fixture outbox");
        let path = outbox.join(format!("{sequence:020}.msg"));
        fs::write(&path, text).expect("write queued fixture message");
        path
    }

    fn read_events(run: &RunBoard) -> Vec<Value> {
        fs::read_to_string(run.dir.join("inbox/events.jsonl"))
            .expect("read durable event log")
            .lines()
            .map(|line| serde_json::from_str(line).expect("parse durable event"))
            .collect()
    }

    #[test]
    fn metadata_payload_includes_a_workers_task_when_titles_are_supported() {
        #[derive(Default)]
        struct MetadataHerdr {
            update: std::cell::RefCell<Option<crate::metadata::MetadataUpdate>>,
        }

        impl HerdrApi for MetadataHerdr {
            fn api_schema(&self) -> Result<String, crate::herdr::HerdrError> {
                Ok(r#"{"schemas":{"request":{"$defs":{"PaneReportMetadataParams":{"properties":{"pane_id":{},"source":{},"title":{}}}}}}}"#.to_owned())
            }

            fn pane_report_metadata(
                &self,
                _: &str,
                update: &crate::metadata::MetadataUpdate,
            ) -> Result<(), crate::herdr::HerdrError> {
                *self.update.borrow_mut() = Some(update.clone());
                Ok(())
            }
        }

        let temp = TempDir::new();
        let mut run = fixture_run(temp.path(), "worker-pane");
        run.state.workers.get_mut("builder").unwrap().task = Some("ship hook seam".to_owned());
        run::save_run(&run).expect("persist worker task");
        let herdr = MetadataHerdr::default();

        on_agent_status(
            &event("worker-pane", "working").to_string(),
            temp.path(),
            &herdr,
        )
        .expect("publish worker metadata");

        assert_eq!(
            herdr.update.borrow().as_ref().unwrap().title.as_deref(),
            Some("ship hook seam")
        );
    }

    #[test]
    fn duplicate_drains_never_record_delivery_failed_after_delivered() {
        let temp = TempDir::new();
        let run = fixture_run(temp.path(), "worker-pane");
        queue_message(&run, 1, "queued once");
        let mut deliveries = Vec::new();

        // Simulate the twice-reproduced live race (defect #59): a second hook
        // invocation drains the same outbox entry while the first is mid-flight.
        drain_outbox(&run.dir, "builder", |text| {
            drain_outbox(&run.dir, "builder", |text| {
                deliveries.push(text.to_owned());
                Ok::<(), std::convert::Infallible>(())
            })
            .expect("concurrent duplicate drain");
            deliveries.push(text.to_owned());
            Ok::<(), std::convert::Infallible>(())
        })
        .expect("original drain");

        assert_eq!(
            deliveries,
            ["queued once"],
            "the message must be delivered exactly once"
        );
        let events = read_events(&run);
        assert!(
            !events
                .iter()
                .any(|event| event["event"] == "delivery_failed"),
            "ENOENT after a successful claim is already-delivered, not a failure: {events:?}"
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| event["event"] == "delivered")
                .count(),
            1,
            "exactly one delivered event: {events:?}"
        );
    }

    #[test]
    fn drain_delivers_exact_content_in_sequence_order_then_removes_and_audits() {
        let temp = TempDir::new();
        let run = fixture_run(temp.path(), "worker-pane");
        let later = queue_message(&run, 10, "second\nline");
        let earlier = queue_message(&run, 2, "first");
        let mut delivered = Vec::new();

        drain_outbox(&run.dir, "builder", |text| {
            delivered.push(text.to_owned());
            Ok::<(), std::convert::Infallible>(())
        })
        .expect("drain queued messages");

        assert_eq!(delivered, ["first", "second\nline"]);
        assert!(!earlier.exists());
        assert!(!later.exists());
        let events = read_events(&run);
        assert_eq!(events.len(), 2);
        assert!(events
            .iter()
            .all(|event| event["event"] == "delivered" && event["target"] == "builder"));
        assert!(events[0]["path"]
            .as_str()
            .unwrap()
            .ends_with("00000000000000000002.msg"));
        assert!(events[1]["path"]
            .as_str()
            .unwrap()
            .ends_with("00000000000000000010.msg"));
    }

    #[test]
    fn failed_delivery_keeps_queue_logs_failure_and_stops_before_later_messages() {
        let temp = TempDir::new();
        let run = fixture_run(temp.path(), "worker-pane");
        let first = queue_message(&run, 1, "first");
        let second = queue_message(&run, 2, "second");
        let mut attempts = Vec::new();

        drain_outbox(&run.dir, "builder", |text| {
            attempts.push(text.to_owned());
            Err::<(), _>("delivery refused")
        })
        .expect("record failed drain without aborting the hook");

        assert_eq!(attempts, ["first"]);
        assert!(first.exists());
        assert!(second.exists());
        let events = read_events(&run);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["event"], "delivery_failed");
        assert_eq!(events[0]["target"], "builder");
        assert_eq!(events[0]["error"], "delivery refused");
        assert!(events[0]["path"]
            .as_str()
            .unwrap()
            .ends_with("00000000000000000001.msg"));
    }

    #[test]
    fn done_drains_through_verified_delivery_before_injecting_report_pointer() {
        let temp = TempDir::new();
        let run = fixture_run(temp.path(), "worker-pane");
        let first = queue_message(&run, 1, "first");
        let second = queue_message(&run, 2, "second");
        let fake = FakeHerdr::new(&temp);

        on_agent_status(
            &event("worker-pane", "working").to_string(),
            temp.path(),
            &fake.client,
        )
        .expect("record working state");
        on_agent_status(
            &event("worker-pane", "done").to_string(),
            temp.path(),
            &fake.client,
        )
        .expect("drain done worker outbox");

        assert!(!first.exists());
        assert!(!second.exists());
        let argv = fake.argv();
        let call_position = |expected: &[&str]| {
            argv.windows(expected.len())
                .position(|window| {
                    window
                        .iter()
                        .map(String::as_str)
                        .eq(expected.iter().copied())
                })
                .expect("expected Herdr call")
        };
        let report_path = run.dir.join("inbox/builder.md");
        let pointer = format!(
            "[team alpha] builder is done — report: {}",
            report_path.display()
        );
        let first_delivery = call_position(&["pane", "run", "worker-pane", "first"]);
        let second_delivery = call_position(&["pane", "run", "worker-pane", "second"]);
        let pointer_delivery = call_position(&["pane", "run", "god-pane", &pointer]);
        assert!(first_delivery < second_delivery);
        assert!(second_delivery < pointer_delivery);
        assert_eq!(
            argv.windows(2)
                .filter(|window| window[0] == "agent" && window[1] == "wait")
                .count(),
            2
        );
        let events = read_events(&run);
        assert_eq!(
            events
                .iter()
                .map(|event| event["event"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "pane_agent_status_changed",
                "pane_agent_status_changed",
                "delivered",
                "delivered"
            ]
        );
    }

    #[test]
    fn working_blocked_and_unknown_flips_leave_queued_messages_untouched() {
        for status in ["working", "blocked", "unknown"] {
            let temp = TempDir::new();
            let run = fixture_run(temp.path(), "worker-pane");
            let queued = queue_message(&run, 1, "queued message");
            let fake = FakeHerdr::new(&temp);

            on_agent_status(
                &event("worker-pane", status).to_string(),
                temp.path(),
                &fake.client,
            )
            .expect("process non-draining status");

            assert!(queued.exists(), "{status} must not drain the outbox");
            assert!(!fake
                .client
                .calls()
                .iter()
                .any(|call| call.ends_with(":queued message")));
        }
    }

    #[test]
    fn captured_payload_and_optional_fields_are_preserved_for_non_terminal_statuses() {
        let temp = TempDir::new();
        let run = fixture_run(temp.path(), "wG:p2");
        let fake = FakeHerdr::new(&temp);
        let captured_event = serde_json::from_str::<Value>(CAPTURED_EVENT).unwrap();
        let event_with_optional_fields = event("wG:p2", "working");

        on_agent_status(CAPTURED_EVENT, temp.path(), &fake.client).expect("process captured event");
        on_agent_status(
            &event_with_optional_fields.to_string(),
            temp.path(),
            &fake.client,
        )
        .expect("process event with optional fields");

        let events =
            fs::read_to_string(run.dir.join("inbox/events.jsonl")).expect("read durable event log");
        let events = events
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(events, [captured_event, event_with_optional_fields]);
        assert!(
            fake.argv()
                .windows(2)
                .all(|window| window != ["pane", "run"]),
            "non-terminal statuses must not inject pointers"
        );
    }

    #[test]
    fn unrelated_pane_exits_without_writing_or_invoking_herdr() {
        let temp = TempDir::new();
        let run = fixture_run(temp.path(), "worker-pane");
        let fake = FakeHerdr::new(&temp);

        on_agent_status(
            &event("not-a-team-pane", "done").to_string(),
            temp.path(),
            &fake.client,
        )
        .expect("ignore unrelated pane");

        assert!(!run.dir.join("inbox/events.jsonl").exists());
        assert!(fake.client.calls().is_empty());
    }

    #[test]
    fn blocked_and_done_append_events_and_inject_exact_absolute_pointers() {
        let temp = TempDir::new();
        let run = fixture_run(temp.path(), "worker-pane");
        let fake = FakeHerdr::new(&temp);

        for status in ["blocked", "done"] {
            on_agent_status(
                &event("worker-pane", status).to_string(),
                temp.path(),
                &fake.client,
            )
            .expect("process terminal status");
        }

        let events =
            fs::read_to_string(run.dir.join("inbox/events.jsonl")).expect("read durable event log");
        assert_eq!(events.lines().count(), 2);

        let report_path = run.dir.join("inbox/builder.md");
        assert!(report_path.is_absolute());
        let argv = fake.argv();
        assert!(argv.windows(4).any(|window| window
            == [
                "pane",
                "run",
                "god-pane",
                &format!(
                    "[team alpha] builder is blocked — report: {}",
                    report_path.display()
                ),
            ]));
        assert!(argv
            .windows(3)
            .any(|window| window == ["notification", "show", "Team complete"]));
    }

    #[test]
    fn dot_form_event_types_are_rejected() {
        let temp = TempDir::new();
        let fake = FakeHerdr::new(&temp);
        let mut raw_event = event("worker-pane", "done");
        raw_event["event"] = json!("pane.agent_status_changed");

        let error = on_agent_status(&raw_event.to_string(), temp.path(), &fake.client)
            .expect_err("dot-form JSON event must fail");

        assert!(matches!(
            error,
            HookError::UnexpectedEvent { field: "event", .. }
        ));
    }

    #[test]
    fn seen_completion_fixture_injects_once_and_repeated_idle_does_not() {
        let temp = TempDir::new();
        let run = fixture_run(temp.path(), "worker-pane");
        let fake = FakeHerdr::new(&temp);

        for status in ["working", "idle", "idle"] {
            on_agent_status(
                &event("worker-pane", status).to_string(),
                temp.path(),
                &fake.client,
            )
            .expect("process completion fixture");
        }

        let pointer = format!(
            "[team alpha] builder is idle — report: {}",
            run.dir.join("inbox/builder.md").display()
        );
        assert_eq!(
            fake.argv()
                .iter()
                .filter(|argument| *argument == &pointer)
                .count(),
            1
        );
        assert_eq!(
            run::load_hook_metadata(&run.dir)
                .expect("load hook metadata")
                .worker_status
                .get("builder"),
            Some(&"idle".to_owned())
        );
    }

    #[test]
    fn unseen_done_fixture_injects_once() {
        let temp = TempDir::new();
        let run = fixture_run(temp.path(), "worker-pane");
        let fake = FakeHerdr::new(&temp);

        on_agent_status(
            &event("worker-pane", "working").to_string(),
            temp.path(),
            &fake.client,
        )
        .expect("record working state");
        for _ in 0..2 {
            on_agent_status(
                &event("worker-pane", "done").to_string(),
                temp.path(),
                &fake.client,
            )
            .expect("process unseen completion fixture");
        }

        let pointer = format!(
            "[team alpha] builder is done — report: {}",
            run.dir.join("inbox/builder.md").display()
        );
        assert_eq!(
            fake.argv()
                .iter()
                .filter(|argument| *argument == &pointer)
                .count(),
            1
        );
    }

    #[test]
    fn pane_moved_fixture_persists_identifier_migration() {
        let temp = TempDir::new();
        let run = fixture_run(temp.path(), "old-pane");
        let fake = FakeHerdr::new(&temp);
        let fixture = json!({"event":"pane_moved","data":{"type":"pane_moved","previous_pane_id":"old-pane","previous_workspace_id":"worker-workspace","previous_tab_id":"old-tab","pane":{"pane_id":"new-pane","workspace_id":"new-workspace"}}});

        on_agent_status(&fixture.to_string(), temp.path(), &fake.client).expect("migrate pane");

        let persisted = run::load_run(&run.dir).expect("load migrated run");
        assert_eq!(
            persisted.state.workers["builder"].pane_id.as_deref(),
            Some("new-pane")
        );
        assert_eq!(
            persisted.state.workers["builder"].workspace_id.as_deref(),
            Some("new-workspace")
        );
    }

    #[test]
    fn pane_exited_fixture_marks_worker_orphaned() {
        let temp = TempDir::new();
        let run = fixture_run(temp.path(), "worker-pane");
        let fake = FakeHerdr::new(&temp);
        let fixture = json!({"event":"pane_exited","data":{"type":"pane_exited","pane_id":"worker-pane","workspace_id":"worker-workspace"}});

        on_agent_status(&fixture.to_string(), temp.path(), &fake.client).expect("reconcile exit");

        assert_eq!(
            run::load_run(&run.dir)
                .expect("load exited run")
                .state
                .workers["builder"]
                .lifecycle,
            WorkerLifecycle::Orphaned
        );
    }

    #[test]
    fn pane_closed_fixture_marks_worker_orphaned() {
        let temp = TempDir::new();
        let run = fixture_run(temp.path(), "worker-pane");
        let fake = FakeHerdr::new(&temp);
        let fixture = json!({"event":"pane_closed","data":{"type":"pane_closed","pane_id":"worker-pane","workspace_id":"worker-workspace"}});

        on_agent_status(&fixture.to_string(), temp.path(), &fake.client).expect("reconcile close");

        assert_eq!(
            run::load_run(&run.dir)
                .expect("load closed run")
                .state
                .workers["builder"]
                .lifecycle,
            WorkerLifecycle::Orphaned
        );
    }

    #[test]
    fn workspace_closed_fixture_ends_run() {
        let temp = TempDir::new();
        let run = fixture_run(temp.path(), "worker-pane");
        let fake = FakeHerdr::new(&temp);
        let fixture = json!({"event":"workspace_closed","data":{"type":"workspace_closed","workspace_id":"worker-workspace"}});

        on_agent_status(&fixture.to_string(), temp.path(), &fake.client)
            .expect("reconcile workspace close");

        assert_eq!(
            run::load_run(&run.dir)
                .expect("load ended run")
                .state
                .lifecycle,
            RunLifecycle::Ended
        );
    }

    #[test]
    fn one_worker_workspace_close_ends_only_that_workers_allocation() {
        let temp = TempDir::new();
        let mut run = fixture_run(temp.path(), "worker-pane");
        run.state.workers.insert(
            "reviewer".to_owned(),
            WorkerRunState {
                task: None,
                workspace_id: Some("reviewer-workspace".to_owned()),
                pane_id: Some("reviewer-pane".to_owned()),
                agent_id: Some("agent-2".to_owned()),
                agent_session: None,
                worktree_path: None,
                adopted: false,
                launch_checkpoint: Default::default(),
                lifecycle: WorkerLifecycle::Running,
            },
        );
        run::save_run(&run).expect("persist two-worker fixture");
        let fake = FakeHerdr::new(&temp);
        let fixture = json!({"event":"workspace_closed","data":{"type":"workspace_closed","workspace_id":"worker-workspace"}});

        on_agent_status(&fixture.to_string(), temp.path(), &fake.client)
            .expect("reconcile one worker workspace close");

        let persisted = run::load_run(&run.dir).expect("load reconciled run");
        assert_eq!(persisted.state.lifecycle, RunLifecycle::Active);
        assert_eq!(
            persisted.state.workers["builder"].lifecycle,
            WorkerLifecycle::Ended
        );
        assert_eq!(
            persisted.state.workers["reviewer"].lifecycle,
            WorkerLifecycle::Running
        );
    }

    #[test]
    fn god_pane_moved_fixture_persists_god_pane_id() {
        let temp = TempDir::new();
        let run = fixture_run(temp.path(), "worker-pane");
        let fake = FakeHerdr::new(&temp);
        let fixture = json!({"event":"pane_moved","data":{"type":"pane_moved","previous_pane_id":"god-pane","previous_workspace_id":"god-workspace","previous_tab_id":"god-tab","pane":{"pane_id":"new-god-pane","workspace_id":"new-god-workspace"}}});

        on_agent_status(&fixture.to_string(), temp.path(), &fake.client).expect("migrate god pane");

        let persisted = run::load_run(&run.dir).expect("load migrated run");
        assert_eq!(persisted.state.god_pane_id, "new-god-pane");
        assert_eq!(
            run::load_hook_metadata(&run.dir)
                .expect("load hook metadata")
                .god_workspace_id
                .as_deref(),
            Some("new-god-workspace")
        );
    }

    #[test]
    fn worktree_removed_fixture_ends_run() {
        let temp = TempDir::new();
        let run = fixture_run(temp.path(), "worker-pane");
        let fake = FakeHerdr::new(&temp);
        let fixture = json!({"event":"worktree_removed","data":{"type":"worktree_removed","workspace_id":"worker-workspace","worktree":{"path":"/tmp/worktree"},"forced":false}});

        on_agent_status(&fixture.to_string(), temp.path(), &fake.client)
            .expect("reconcile worktree removal");

        assert_eq!(
            run::load_run(&run.dir)
                .expect("load ended run")
                .state
                .lifecycle,
            RunLifecycle::Ended
        );
    }

    #[test]
    fn pane_agent_detected_fixture_persists_optional_identity() {
        let temp = TempDir::new();
        let run = fixture_run(temp.path(), "worker-pane");
        let fake = FakeHerdr::new(&temp);
        let fixture = json!({"event":"pane_agent_detected","data":{"type":"pane_agent_detected","pane_id":"worker-pane","workspace_id":"worker-workspace","agent":"codex"}});

        on_agent_status(&fixture.to_string(), temp.path(), &fake.client)
            .expect("bind agent identity");

        assert_eq!(
            run::load_hook_metadata(&run.dir)
                .expect("load hook metadata")
                .worker_agent_identity
                .get("builder"),
            Some(&"codex".to_owned())
        );
    }
}
