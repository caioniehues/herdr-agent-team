//! Push-based worker status report and outbox hook from `docs/spec.md` sections 5 and 11.

use crate::herdr::HerdrClient;
use crate::msg;
use crate::reconcile::{reconcile, IncomingEvent, ReconciliationAction};
use crate::run;
use serde_json::{json, Value};
use std::fmt::Display;
use std::path::{Path, PathBuf};
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

pub fn on_agent_status(
    event_json: &str,
    state_dir: &Path,
    herdr: &HerdrClient,
) -> Result<(), HookError> {
    let raw_event: Value = serde_json::from_str(event_json)?;
    let event = parse_event(serde_json::from_value(raw_event.clone())?)?;

    for mut run in run::list_active_runs(state_dir)? {
        let metadata = run::load_hook_metadata(&run.dir)?;
        let reconciliation = reconcile(&event, run.state.clone(), metadata);
        if reconciliation.actions.is_empty() {
            continue;
        }
        run.state = reconciliation.state;
        run::save_run_with_hook(&run, &reconciliation.metadata)?;
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

fn inject_pointer(
    run: &run::RunBoard,
    worker_name: &str,
    status: &str,
    herdr: &HerdrClient,
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
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) => {
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
            append_delivery_event(
                run_dir,
                "delivery_failed",
                target,
                &path,
                Some(&error.to_string()),
            )?;
            break;
        }

        if let Err(error) = std::fs::remove_file(&path) {
            append_delivery_event(
                run_dir,
                "delivery_failed",
                target,
                &path,
                Some(&error.to_string()),
            )?;
            break;
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
    use std::os::unix::fs::PermissionsExt;
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

    struct FakeHerdr {
        client: HerdrClient,
        log: PathBuf,
    }

    impl FakeHerdr {
        fn new(temp: &TempDir) -> Self {
            let binary = temp.path().join("fake-herdr");
            let log = temp.path().join("herdr-argv.log");
            fs::write(
                &binary,
                format!(
                    "#!/bin/sh\nprintf '%s\\n' \"$@\" >> '{}'\nif [ \"$1\" = 'agent' ] && [ \"$2\" = 'wait' ]; then\n  printf '{{\"event\":\"pane.agent_status_changed\",\"data\":{{\"pane_id\":\"%s\",\"agent_status\":\"working\"}}}}\\n' \"$3\"\nfi\n",
                    log.display()
                ),
            )
            .expect("write fake Herdr CLI");
            let mut permissions = fs::metadata(&binary).expect("stat fake CLI").permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&binary, permissions).expect("make fake CLI executable");
            Self {
                client: HerdrClient { binary },
                log,
            }
        }

        fn argv(&self) -> Vec<String> {
            fs::read_to_string(&self.log)
                .expect("read fake CLI log")
                .lines()
                .map(str::to_owned)
                .collect()
        }
    }

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
                        workspace_id: Some("worker-workspace".to_owned()),
                        pane_id: Some(worker_pane.to_owned()),
                        agent_id: Some("agent-1".to_owned()),
                        agent_session: None,
                        worktree_path: None,
                        adopted: false,
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
            if fake.log.exists() {
                assert!(!fs::read_to_string(&fake.log)
                    .expect("read fake Herdr log")
                    .lines()
                    .any(|argument| argument == "queued message"));
            }
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
        assert!(!fake.log.exists(), "non-terminal statuses must not inject");
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
        assert!(!fake.log.exists());
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
        assert_eq!(
            argv,
            [
                "pane",
                "run",
                "god-pane",
                &format!(
                    "[team alpha] builder is blocked — report: {}",
                    report_path.display()
                ),
            ]
        );
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
                workspace_id: Some("reviewer-workspace".to_owned()),
                pane_id: Some("reviewer-pane".to_owned()),
                agent_id: Some("agent-2".to_owned()),
                agent_session: None,
                worktree_path: None,
                adopted: false,
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
