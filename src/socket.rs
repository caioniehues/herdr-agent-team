//! Experimental protocol-16 public NDJSON socket backend (ADR-0011).
//!
//! Mutations deliberately delegate to the CLI. The socket is reserved for
//! snapshots and multiplexed event subscriptions.

use crate::board::{BoardCollector, BoardError, BoardSnapshot};
use crate::god_cli::{GodCliError, GodCollector, GodSnapshot};
use crate::herdr::{
    AgentInfo, HerdrApi, HerdrClient, HerdrError, PaneInfo, WaitOutcome, WorkspaceRef, WorktreeRef,
};
use crate::metadata::MetadataUpdate;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub const SUPPORTED_PROTOCOL: u32 = 16;
pub const DEFAULT_MAX_FRAME_BYTES: usize = 1024 * 1024;
const BACKEND_ENV: &str = "HERDR_TEAM_BACKEND";
const TRACE_ENV: &str = "HERDR_TEAM_SOCKET_TRACE";
const SCHEMA_BASELINE: &str = include_str!("../docs/herdr-api-schema.snapshot.json");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handshake {
    pub version: String,
    pub protocol: u32,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionSnapshot {
    pub version: String,
    pub protocol: u32,
    pub focused_workspace_id: Option<String>,
    pub focused_tab_id: Option<String>,
    pub focused_pane_id: Option<String>,
    pub workspaces: Vec<Value>,
    pub tabs: Vec<Value>,
    pub panes: Vec<Value>,
    pub layouts: Vec<Value>,
    pub agents: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubscriptionEvent {
    pub event: String,
    pub data: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    id: String,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<ApiError>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApiError {
    code: String,
    message: String,
}

#[derive(Debug, Serialize)]
struct Request<'a> {
    id: &'a str,
    method: &'a str,
    params: Value,
}

#[derive(Debug, Clone)]
pub struct SocketClient<C = HerdrClient> {
    socket_path: PathBuf,
    fallback: C,
    max_frame_bytes: usize,
    next_id: std::sync::Arc<AtomicU64>,
    handshake: Handshake,
}

impl SocketClient<HerdrClient> {
    pub fn try_from_env() -> Option<Self> {
        if std::env::var(BACKEND_ENV).as_deref() != Ok("socket") {
            return None;
        }
        let path = std::env::var_os("HERDR_SOCKET_PATH").map(PathBuf::from)?;
        Self::connect(path, HerdrClient::from_env()).ok()
    }
    /// Opt in with `HERDR_TEAM_BACKEND=socket`; any handshake failure returns
    /// the ordinary CLI client, preserving the pre-ADR behavior.
    pub fn from_env_or_cli() -> Backend {
        let cli = HerdrClient::from_env();
        if std::env::var(BACKEND_ENV).as_deref() != Ok("socket") {
            return Backend::Cli(cli);
        }
        let Some(path) = std::env::var_os("HERDR_SOCKET_PATH").map(PathBuf::from) else {
            return Backend::Cli(cli);
        };
        match Self::connect(path, cli.clone()) {
            Ok(socket) => Backend::Socket(socket),
            Err(_) => Backend::Cli(cli),
        }
    }
}

impl<C: HerdrApi> SocketClient<C> {
    pub fn connect(socket_path: PathBuf, fallback: C) -> Result<Self, HerdrError> {
        validate_schema_baseline()?;
        let mut client = Self {
            socket_path,
            fallback,
            max_frame_bytes: DEFAULT_MAX_FRAME_BYTES,
            next_id: Default::default(),
            handshake: Handshake {
                version: String::new(),
                protocol: 0,
            },
        };
        client.handshake = client.perform_handshake()?;
        Ok(client)
    }

    pub fn handshake(&self) -> &Handshake {
        &self.handshake
    }

    pub fn snapshot(&self) -> Result<SessionSnapshot, HerdrError> {
        let result = self.rpc("session.snapshot", json!({}))?;
        #[derive(Deserialize)]
        #[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
        enum ResultShape {
            SessionSnapshot { snapshot: SessionSnapshot },
        }
        match serde_json::from_value::<ResultShape>(result)
            .map_err(|e| invalid("session.snapshot", e))?
        {
            ResultShape::SessionSnapshot { snapshot } => Ok(snapshot),
        }
    }

    /// Re-snapshots before every subscription attempt. EOF triggers a bounded
    /// reconnect; callers receive each fresh snapshot before subsequent events.
    pub fn subscribe_with_reconnect<F>(
        &self,
        subscriptions: &[Value],
        reconnects: usize,
        mut on_item: F,
    ) -> Result<(), HerdrError>
    where
        F: FnMut(StreamItem),
    {
        for attempt in 0..=reconnects {
            on_item(StreamItem::Snapshot(self.snapshot()?));
            let id = self.request_id();
            let mut stream = connect(&self.socket_path)?;
            write_request(
                &mut stream,
                &Request {
                    id: &id,
                    method: "events.subscribe",
                    params: json!({"subscriptions": subscriptions}),
                },
            )?;
            let mut reader = BufReader::new(stream);
            let ack = read_envelope(&mut reader, self.max_frame_bytes, "events.subscribe")?;
            check_id(&id, &ack.id, "events.subscribe")?;
            let value = result_or_error(ack, "events.subscribe")?;
            if value.get("type").and_then(Value::as_str) != Some("subscription_started") {
                return Err(invalid_msg(
                    "events.subscribe",
                    "expected result.type subscription_started",
                ));
            }
            while let Some(value) =
                read_value_optional(&mut reader, self.max_frame_bytes, "events.subscribe")?
            {
                on_item(StreamItem::Event(
                    serde_json::from_value(value)
                        .map_err(|e| invalid("events.subscribe event", e))?,
                ));
            }
            if attempt == reconnects {
                return Ok(());
            }
        }
        Ok(())
    }

    /// Wait for one event from a single subscription carrying every pane's
    /// status changes. Team membership is filtered against durable run state
    /// by `GodCollector`, so this stays one multiplexed server subscription.
    pub fn wait_for_team_change(
        &self,
        timeout: Duration,
    ) -> Result<Option<SubscriptionEvent>, HerdrError> {
        let id = self.request_id();
        let mut stream = connect(&self.socket_path)?;
        stream
            .set_read_timeout(Some(timeout))
            .map_err(|e| invalid("events.subscribe", e))?;
        write_request(
            &mut stream,
            &Request {
                id: &id,
                method: "events.subscribe",
                params: json!({"subscriptions":[{"type":"pane.agent_status_changed"}]}),
            },
        )?;
        let mut reader = BufReader::new(stream);
        let ack = read_envelope(&mut reader, self.max_frame_bytes, "events.subscribe")?;
        check_id(&id, &ack.id, "events.subscribe")?;
        let value = result_or_error(ack, "events.subscribe")?;
        if value.get("type").and_then(Value::as_str) != Some("subscription_started") {
            return Err(invalid_msg(
                "events.subscribe",
                "expected result.type subscription_started",
            ));
        }
        read_value_optional(&mut reader, self.max_frame_bytes, "events.subscribe")?
            .map(|value| {
                serde_json::from_value(value).map_err(|e| invalid("events.subscribe event", e))
            })
            .transpose()
    }

    fn perform_handshake(&self) -> Result<Handshake, HerdrError> {
        let result = self.rpc("ping", json!({}))?;
        #[derive(Deserialize)]
        #[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
        enum Pong {
            Pong {
                version: String,
                protocol: u32,
                #[serde(default)]
                #[serde(rename = "capabilities")]
                _capabilities: Option<Value>,
            },
        }
        let Pong::Pong {
            version,
            protocol,
            _capabilities: _,
        } = serde_json::from_value(result).map_err(|e| invalid("ping", e))?;
        if protocol != SUPPORTED_PROTOCOL {
            return Err(invalid_msg("ping", &format!("unsupported Herdr protocol: client supports {SUPPORTED_PROTOCOL}, server reported {protocol} (version {version})")));
        }
        Ok(Handshake { version, protocol })
    }

    fn rpc(&self, method: &str, params: Value) -> Result<Value, HerdrError> {
        let started = Instant::now();
        let id = self.request_id();
        let mut stream = connect(&self.socket_path)?;
        write_request(
            &mut stream,
            &Request {
                id: &id,
                method,
                params,
            },
        )?;
        let envelope = read_envelope(&mut BufReader::new(stream), self.max_frame_bytes, method)?;
        check_id(&id, &envelope.id, method)?;
        let outcome = result_or_error(envelope, method);
        trace(
            method,
            &id,
            outcome.as_ref().ok(),
            outcome.as_ref().err(),
            started.elapsed(),
        );
        outcome
    }

    fn request_id(&self) -> String {
        format!(
            "herdr-agent-team:{}",
            self.next_id.fetch_add(1, Ordering::Relaxed)
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum StreamItem {
    Snapshot(SessionSnapshot),
    Event(SubscriptionEvent),
}

pub enum Backend {
    Cli(HerdrClient),
    Socket(SocketClient),
}

/// Socket-aware collector adapters preserve durable run/inbox truth. A socket
/// snapshot is used as the fast freshness probe; clean fallback retains the
/// exact existing collector verdict when the socket disappears.
pub struct SocketGodCollector<C, G> {
    pub socket: SocketClient<C>,
    pub fallback: G,
}
impl<C: HerdrApi, G: GodCollector> GodCollector for SocketGodCollector<C, G> {
    fn collect(&self) -> Result<GodSnapshot, GodCliError> {
        let _ = self.socket.snapshot();
        self.fallback.collect()
    }
    fn wait_for_change(&self, timeout: Duration) {
        if self.socket.wait_for_team_change(timeout).is_err() {
            self.fallback.wait_for_change(timeout);
        }
    }
}
pub struct SocketBoardCollector<C, B> {
    pub socket: SocketClient<C>,
    pub fallback: B,
}
impl<C: HerdrApi, B: BoardCollector> BoardCollector for SocketBoardCollector<C, B> {
    fn collect(&self) -> Result<BoardSnapshot, BoardError> {
        let _ = self.socket.snapshot();
        self.fallback.collect()
    }
}

macro_rules! delegate {
    ($self:ident.$method:ident($($arg:expr),*)) => { match $self { Backend::Cli(c) => c.$method($($arg),*), Backend::Socket(c) => c.$method($($arg),*) } };
}

impl HerdrApi for Backend {
    fn workspace_create(&self, c: &Path, l: &str) -> Result<WorkspaceRef, HerdrError> {
        delegate!(self.workspace_create(c, l))
    }
    fn workspace_close(&self, w: &str) -> Result<(), HerdrError> {
        delegate!(self.workspace_close(w))
    }
    fn worktree_create(&self, r: &Path, b: &str) -> Result<WorktreeRef, HerdrError> {
        delegate!(self.worktree_create(r, b))
    }
    fn worktree_remove(&self, p: &Path) -> Result<(), HerdrError> {
        delegate!(self.worktree_remove(p))
    }
    fn pane_run(&self, p: &str, i: &str) -> Result<(), HerdrError> {
        delegate!(self.pane_run(p, i))
    }
    fn pane_get(&self, p: &str) -> Result<PaneInfo, HerdrError> {
        delegate!(self.pane_get(p))
    }
    fn agent_list(&self) -> Result<Vec<AgentInfo>, HerdrError> {
        delegate!(self.agent_list())
    }
    fn agent_wait(&self, p: &str, s: &str, t: Duration) -> Result<WaitOutcome, HerdrError> {
        delegate!(self.agent_wait(p, s, t))
    }
}

impl<C: HerdrApi> HerdrApi for SocketClient<C> {
    fn workspace_create(&self, c: &Path, l: &str) -> Result<WorkspaceRef, HerdrError> {
        self.fallback.workspace_create(c, l)
    }
    fn workspace_close(&self, w: &str) -> Result<(), HerdrError> {
        self.fallback.workspace_close(w)
    }
    fn worktree_create(&self, r: &Path, b: &str) -> Result<WorktreeRef, HerdrError> {
        self.fallback.worktree_create(r, b)
    }
    fn worktree_remove(&self, p: &Path) -> Result<(), HerdrError> {
        self.fallback.worktree_remove(p)
    }
    fn pane_split(&self, w: &str, c: &Path) -> Result<PaneInfo, HerdrError> {
        self.fallback.pane_split(w, c)
    }
    fn pane_run(&self, p: &str, i: &str) -> Result<(), HerdrError> {
        self.fallback.pane_run(p, i)
    }
    fn pane_read(&self, p: &str) -> Result<String, HerdrError> {
        self.fallback.pane_read(p)
    }
    fn pane_rename(&self, p: &str, t: &str) -> Result<(), HerdrError> {
        self.fallback.pane_rename(p, t)
    }
    fn agent_wait(&self, p: &str, s: &str, t: Duration) -> Result<WaitOutcome, HerdrError> {
        self.fallback.agent_wait(p, s, t)
    }
    fn agent_list(&self) -> Result<Vec<AgentInfo>, HerdrError> {
        self.fallback.agent_list()
    }
    fn pane_get(&self, p: &str) -> Result<PaneInfo, HerdrError> {
        self.fallback.pane_get(p)
    }
    fn api_schema(&self) -> Result<String, HerdrError> {
        self.fallback.api_schema()
    }
    fn pane_report_metadata(&self, p: &str, u: &MetadataUpdate) -> Result<(), HerdrError> {
        self.fallback.pane_report_metadata(p, u)
    }
    fn notification_show(&self, t: &str, b: &str, s: &str) -> Result<(), HerdrError> {
        self.fallback.notification_show(t, b, s)
    }
}

#[cfg(unix)]
fn connect(path: &Path) -> Result<UnixStream, HerdrError> {
    UnixStream::connect(path).map_err(|e| invalid_msg("socket connect", &e.to_string()))
}
#[cfg(not(unix))]
fn connect(_: &Path) -> Result<std::fs::File, HerdrError> {
    Err(invalid_msg(
        "socket connect",
        "public socket backend is unavailable on this platform",
    ))
}
fn write_request<W: Write>(w: &mut W, r: &Request<'_>) -> Result<(), HerdrError> {
    serde_json::to_writer(&mut *w, r).map_err(|e| invalid("request", e))?;
    w.write_all(b"\n").map_err(|e| invalid("request", e))?;
    w.flush().map_err(|e| invalid("request", e))
}
fn read_envelope<R: BufRead>(r: &mut R, max: usize, m: &str) -> Result<Envelope, HerdrError> {
    let v = read_value_optional(r, max, m)?.ok_or_else(|| invalid_msg(m, "empty response"))?;
    serde_json::from_value(v).map_err(|e| invalid(m, e))
}
fn read_value_optional<R: BufRead>(
    r: &mut R,
    max: usize,
    m: &str,
) -> Result<Option<Value>, HerdrError> {
    let mut bytes = Vec::new();
    let mut limited = std::io::Read::take(std::io::Read::by_ref(r), (max + 1) as u64);
    let n = limited
        .read_until(b'\n', &mut bytes)
        .map_err(|e| invalid(m, e))?;
    if n == 0 {
        return Ok(None);
    }
    if bytes.len() > max {
        return Err(invalid_msg(m, &format!("frame exceeds {max} byte limit")));
    }
    if !bytes.ends_with(b"\n") {
        return Err(invalid_msg(m, "malformed frame: missing newline"));
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|e| invalid(m, e))
}
fn check_id(expected: &str, actual: &str, m: &str) -> Result<(), HerdrError> {
    if expected == actual {
        Ok(())
    } else {
        Err(invalid_msg(
            m,
            &format!("response id mismatch: expected {expected}, got {actual}"),
        ))
    }
}
fn result_or_error(e: Envelope, m: &str) -> Result<Value, HerdrError> {
    match (e.result, e.error) {
        (Some(v), None) => Ok(v),
        (None, Some(e)) => Err(invalid_msg(m, &format!("{}: {}", e.code, e.message))),
        _ => Err(invalid_msg(
            m,
            "response must contain exactly one of result or error",
        )),
    }
}
fn invalid(m: &str, e: impl std::fmt::Display) -> HerdrError {
    invalid_msg(m, &e.to_string())
}
fn invalid_msg(m: &str, s: &str) -> HerdrError {
    HerdrError::InvalidResponse {
        argv: format!("public socket {m}"),
        message: s.into(),
    }
}
fn validate_schema_baseline() -> Result<(), HerdrError> {
    for required in [
        "\"const\": \"ping\"",
        "\"const\": \"session.snapshot\"",
        "\"const\": \"events.subscribe\"",
        "\"const\": \"subscription_started\"",
    ] {
        if !SCHEMA_BASELINE.contains(required) {
            return Err(invalid_msg(
                "schema baseline",
                &format!("protocol-16 snapshot lacks {required}"),
            ));
        }
    }
    Ok(())
}
fn trace(
    method: &str,
    id: &str,
    result: Option<&Value>,
    error: Option<&HerdrError>,
    elapsed: Duration,
) {
    let Some(path) = std::env::var_os(TRACE_ENV) else {
        return;
    };
    let result_type = result.and_then(|v| v.get("type")).and_then(Value::as_str);
    let row = json!({"id":id,"method":method,"result_type":result_type,"error":error.map(ToString::to_string),"latency_ms":elapsed.as_millis()});
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{row}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::god_cli::{GodSnapshot, InboxRow};
    use crate::herdr::test_support::FakeHerdr;
    use crate::types::{RunLifecycle, WorkerLifecycle};
    use std::fs;
    use std::os::unix::net::UnixListener;
    use std::thread;

    fn snapshot(id: &str) -> String {
        format!("{{\"id\":\"{id}\",\"result\":{{\"type\":\"session_snapshot\",\"snapshot\":{{\"version\":\"0.9.0\",\"protocol\":16,\"focused_workspace_id\":null,\"focused_tab_id\":null,\"focused_pane_id\":null,\"workspaces\":[],\"tabs\":[],\"panes\":[],\"layouts\":[],\"agents\":[]}}}}}}\n")
    }

    fn fake(responses: Vec<String>) -> (PathBuf, thread::JoinHandle<()>) {
        let path = std::env::temp_dir().join(format!(
            "hat-socket-{}-{}.sock",
            std::process::id(),
            rand_id()
        ));
        let _ = fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let h = thread::spawn(move || {
            for response in responses {
                let (mut s, _) = listener.accept().unwrap();
                let mut line = String::new();
                BufReader::new(s.try_clone().unwrap())
                    .read_line(&mut line)
                    .unwrap();
                s.write_all(response.as_bytes()).unwrap();
            }
        });
        (path, h)
    }
    fn rand_id() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }
    fn pong(id: &str, p: u32) -> String {
        format!("{{\"id\":\"{id}\",\"result\":{{\"type\":\"pong\",\"version\":\"0.9.0\",\"protocol\":{p}}}}}\n")
    }
    #[test]
    fn handshake_accepts_protocol_16() {
        let (path, h) = fake(vec![pong("herdr-agent-team:0", 16)]);
        let c = SocketClient::connect(path.clone(), FakeHerdr::default()).unwrap();
        assert_eq!(
            c.handshake(),
            &Handshake {
                version: "0.9.0".into(),
                protocol: 16
            }
        );
        h.join().unwrap();
        let _ = fs::remove_file(path);
    }
    #[test]
    fn wrong_protocol_is_clear() {
        let (path, h) = fake(vec![pong("herdr-agent-team:0", 15)]);
        let e = SocketClient::connect(path.clone(), FakeHerdr::default())
            .err()
            .expect("wrong protocol must fail")
            .to_string();
        assert!(e.contains("client supports 16, server reported 15"));
        h.join().unwrap();
        let _ = fs::remove_file(path);
    }
    #[test]
    fn response_id_mismatch_is_rejected() {
        let (path, h) = fake(vec![pong("wrong", 16)]);
        let e = SocketClient::connect(path.clone(), FakeHerdr::default())
            .err()
            .expect("wrong id must fail")
            .to_string();
        assert!(e.contains("response id mismatch"));
        h.join().unwrap();
        let _ = fs::remove_file(path);
    }
    #[test]
    fn malformed_frame_is_rejected() {
        let (path, h) = fake(vec!["not-json\n".into()]);
        assert!(SocketClient::connect(path.clone(), FakeHerdr::default()).is_err());
        h.join().unwrap();
        let _ = fs::remove_file(path);
    }
    #[test]
    fn bounded_frame_is_enforced() {
        let (path, h) = fake(vec![format!(
            "{}\n",
            "x".repeat(DEFAULT_MAX_FRAME_BYTES + 1)
        )]);
        let e = SocketClient::connect(path.clone(), FakeHerdr::default())
            .err()
            .expect("oversize frame must fail")
            .to_string();
        assert!(e.contains("frame exceeds"));
        h.join().unwrap();
        let _ = fs::remove_file(path);
    }
    #[test]
    fn mutations_delegate_to_cli_seam() {
        let (path, h) = fake(vec![pong("herdr-agent-team:0", 16)]);
        let fallback = FakeHerdr::default();
        let c = SocketClient::connect(path.clone(), fallback).unwrap();
        assert_eq!(
            c.workspace_create(Path::new("/tmp"), "x")
                .unwrap()
                .workspace_id,
            "workspace-1"
        );
        h.join().unwrap();
        let _ = fs::remove_file(path);
    }

    #[test]
    fn snapshot_subscribe_reconnect_resnapshots() {
        let (path, h) = fake(vec![
            pong("herdr-agent-team:0", 16),
            snapshot("herdr-agent-team:1"),
            "{\"id\":\"herdr-agent-team:2\",\"result\":{\"type\":\"subscription_started\"}}\n".into(),
            snapshot("herdr-agent-team:3"),
            concat!("{\"id\":\"herdr-agent-team:4\",\"result\":{\"type\":\"subscription_started\"}}\n", "{\"event\":\"pane.agent_status_changed\",\"data\":{\"pane_id\":\"p1\",\"workspace_id\":\"w1\",\"agent_status\":\"blocked\"}}\n").into(),
        ]);
        let client = SocketClient::connect(path.clone(), FakeHerdr::default()).unwrap();
        let mut items = Vec::new();
        client
            .subscribe_with_reconnect(&[json!({"type":"pane.agent_status_changed"})], 1, |item| {
                items.push(item)
            })
            .unwrap();
        assert_eq!(
            items
                .iter()
                .filter(|i| matches!(i, StreamItem::Snapshot(_)))
                .count(),
            2
        );
        assert_eq!(
            items
                .iter()
                .filter(|i| matches!(i, StreamItem::Event(_)))
                .count(),
            1
        );
        h.join().unwrap();
        let _ = fs::remove_file(path);
    }

    #[test]
    fn connect_failure_can_cleanly_retain_cli_backend() {
        let missing = std::env::temp_dir().join(format!("missing-{}.sock", rand_id()));
        assert!(SocketClient::connect(missing, FakeHerdr::default()).is_err());
        let fallback = FakeHerdr::default();
        assert_eq!(
            fallback
                .workspace_create(Path::new("/tmp"), "x")
                .unwrap()
                .workspace_id,
            "workspace-1"
        );
    }

    #[derive(Clone)]
    struct FixtureGod(GodSnapshot);
    impl GodCollector for FixtureGod {
        fn collect(&self) -> Result<GodSnapshot, GodCliError> {
            Ok(self.0.clone())
        }
    }

    #[test]
    fn socket_god_collector_matches_cli_fixture_verdict() {
        let expected = GodSnapshot {
            run_dir: PathBuf::from("/run"),
            lifecycle: RunLifecycle::Active,
            rows: vec![InboxRow {
                worker: "m".into(),
                report_present: false,
                report_mtime_ms: None,
                attention: true,
                read: false,
                stopped_not_done: false,
            }],
            worker_lifecycles: vec![("m".into(), WorkerLifecycle::Running)],
            statuses: vec![("m".into(), "blocked".into())],
        };
        let (path, h) = fake(vec![
            pong("herdr-agent-team:0", 16),
            snapshot("herdr-agent-team:1"),
        ]);
        let socket = SocketClient::connect(path.clone(), FakeHerdr::default()).unwrap();
        let cli = FixtureGod(expected.clone());
        let over_socket = SocketGodCollector {
            socket,
            fallback: cli.clone(),
        };
        assert_eq!(cli.collect().unwrap(), over_socket.collect().unwrap());
        h.join().unwrap();
        let _ = fs::remove_file(path);
    }
}
