//! Experimental protocol-16 public NDJSON socket backend (ADR-0011).
//!
//! Mutations deliberately delegate to the CLI. The socket is reserved for
//! snapshots and multiplexed event subscriptions.

use crate::herdr::{HerdrApi, HerdrClient, HerdrError};
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
pub const DEFAULT_IO_TIMEOUT: Duration = Duration::from_millis(100);
pub const MAX_RECONNECTS: usize = 3;
pub(crate) const RECONNECT_BACKOFF: Duration = Duration::from_millis(10);
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
    pub workspaces: Vec<SnapshotWorkspace>,
    pub tabs: Vec<SnapshotTab>,
    pub panes: Vec<SnapshotPane>,
    pub layouts: Vec<SnapshotLayout>,
    pub agents: Vec<SnapshotAgent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SnapshotWorkspace {
    pub workspace_id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SnapshotTab {
    pub tab_id: String,
    pub workspace_id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SnapshotPane {
    pub pane_id: String,
    pub terminal_id: String,
    pub workspace_id: String,
    pub tab_id: String,
    pub agent_status: AgentStatus,
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SnapshotLayout {
    pub workspace_id: String,
    pub tab_id: String,
    pub focused_pane_id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SnapshotAgent {
    pub terminal_id: String,
    pub workspace_id: String,
    pub tab_id: String,
    pub pane_id: String,
    pub agent_status: AgentStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Idle,
    Working,
    Blocked,
    Done,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "event", content = "data")]
pub enum SubscriptionEvent {
    #[serde(rename = "pane.agent_status_changed")]
    PaneAgentStatusChanged(PaneAgentStatusChanged),
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PaneAgentStatusChanged {
    pub pane_id: String,
    pub workspace_id: String,
    pub agent_status: AgentStatus,
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
    io_timeout: Duration,
    trace_path: Option<PathBuf>,
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
}

impl<C: HerdrApi> SocketClient<C> {
    pub fn connect(socket_path: PathBuf, fallback: C) -> Result<Self, HerdrError> {
        validate_runtime_schema(&fallback.api_schema()?)?;
        Self::connect_validated(socket_path, fallback)
    }

    fn connect_validated(socket_path: PathBuf, fallback: C) -> Result<Self, HerdrError> {
        let mut client = Self {
            socket_path,
            fallback,
            max_frame_bytes: DEFAULT_MAX_FRAME_BYTES,
            io_timeout: DEFAULT_IO_TIMEOUT,
            trace_path: std::env::var_os(TRACE_ENV).map(PathBuf::from),
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
        let reconnects = reconnects.min(MAX_RECONNECTS);
        let deadline = Instant::now() + self.io_timeout.saturating_mul((reconnects as u32 + 1) * 3);
        for attempt in 0..=reconnects {
            if Instant::now() >= deadline {
                return Err(invalid_msg(
                    "events.subscribe",
                    "reconnect deadline exceeded",
                ));
            }
            on_item(StreamItem::Snapshot(self.snapshot()?));
            let mut reader = self.open_subscription(subscriptions, self.io_timeout)?;
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
            std::thread::sleep(
                RECONNECT_BACKOFF.min(deadline.saturating_duration_since(Instant::now())),
            );
        }
        Ok(())
    }

    /// Wait for one event from a single subscription carrying every pane's
    /// status changes. Team membership is filtered against durable run state
    /// by `GodCollector`, so this stays one multiplexed server subscription.
    pub fn wait_for_team_change(
        &self,
        pane_ids: &[String],
        timeout: Duration,
    ) -> Result<Option<SubscriptionEvent>, HerdrError> {
        let subscriptions = pane_ids
            .iter()
            .map(|pane_id| json!({"type":"pane.agent_status_changed","pane_id":pane_id}))
            .collect::<Vec<_>>();
        self.read_one_event(&subscriptions, timeout)
    }

    pub fn wait_for_change(
        &self,
        subscriptions: &[Value],
        timeout: Duration,
    ) -> Result<bool, HerdrError> {
        self.read_one_event(subscriptions, timeout)
            .map(|event| event.is_some())
    }

    pub(crate) fn subscribe(
        &self,
        subscriptions: &[Value],
        timeout: Duration,
    ) -> Result<SubscriptionStream, HerdrError> {
        self.open_subscription(subscriptions, timeout)
            .map(|reader| SubscriptionStream {
                reader,
                max_frame_bytes: self.max_frame_bytes,
            })
    }

    pub(crate) fn fallback(&self) -> &C {
        &self.fallback
    }

    fn read_one_event(
        &self,
        subscriptions: &[Value],
        timeout: Duration,
    ) -> Result<Option<SubscriptionEvent>, HerdrError> {
        let mut reader = self.open_subscription(subscriptions, timeout)?;
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
        let mut stream = self.open_stream(self.io_timeout)?;
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
        self.trace(
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

    fn open_stream(&self, timeout: Duration) -> Result<UnixStream, HerdrError> {
        let stream = connect(&self.socket_path)?;
        stream
            .set_read_timeout(Some(timeout))
            .map_err(|e| invalid("socket timeout", e))?;
        stream
            .set_write_timeout(Some(timeout))
            .map_err(|e| invalid("socket timeout", e))?;
        Ok(stream)
    }

    fn open_subscription(
        &self,
        subscriptions: &[Value],
        timeout: Duration,
    ) -> Result<BufReader<UnixStream>, HerdrError> {
        let started = Instant::now();
        let id = self.request_id();
        let outcome = (|| {
            let mut stream = self.open_stream(timeout)?;
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
            #[derive(Deserialize)]
            #[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
            enum Ack {
                SubscriptionStarted {},
            }
            serde_json::from_value::<Ack>(value.clone())
                .map_err(|e| invalid("events.subscribe ack", e))?;
            Ok((reader, value))
        })();
        match outcome {
            Ok((reader, value)) => {
                self.trace(
                    "events.subscribe",
                    &id,
                    Some(&value),
                    None,
                    started.elapsed(),
                );
                Ok(reader)
            }
            Err(error) => {
                self.trace(
                    "events.subscribe",
                    &id,
                    None,
                    Some(&error),
                    started.elapsed(),
                );
                Err(error)
            }
        }
    }

    fn trace(
        &self,
        method: &str,
        id: &str,
        result: Option<&Value>,
        error: Option<&HerdrError>,
        elapsed: Duration,
    ) {
        let Some(path) = self.trace_path.as_deref() else {
            return;
        };
        let result_type = result
            .and_then(|value| value.get("type"))
            .and_then(Value::as_str);
        let error_code = error.map(|_| "transport_or_protocol_error");
        let row = json!({"id":id,"method":method,"result_type":result_type,"error_code":error_code,"latency_ms":elapsed.as_millis()});
        write_trace(path, &row);
    }
}

pub(crate) struct SubscriptionStream {
    reader: BufReader<UnixStream>,
    max_frame_bytes: usize,
}

pub(crate) enum SubscriptionPoll {
    Event(SubscriptionEvent),
    Timeout,
    Closed,
}

impl SubscriptionStream {
    pub(crate) fn poll(&mut self, timeout: Duration) -> Result<SubscriptionPoll, HerdrError> {
        self.reader
            .get_ref()
            .set_read_timeout(Some(timeout))
            .map_err(|source| transport("set subscription timeout", source))?;
        match read_value_optional(&mut self.reader, self.max_frame_bytes, "events.subscribe") {
            Ok(Some(value)) => serde_json::from_value(value)
                .map(SubscriptionPoll::Event)
                .map_err(|e| invalid("events.subscribe event", e)),
            Ok(None) => Ok(SubscriptionPoll::Closed),
            Err(HerdrError::Transport { source, .. })
                if matches!(
                    source.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                Ok(SubscriptionPoll::Timeout)
            }
            Err(error) => Err(error),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum StreamItem {
    Snapshot(SessionSnapshot),
    Event(SubscriptionEvent),
}

#[cfg(unix)]
fn connect(path: &Path) -> Result<UnixStream, HerdrError> {
    UnixStream::connect(path).map_err(|source| transport("connect", source))
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
    w.write_all(b"\n")
        .map_err(|source| transport("write request", source))?;
    w.flush()
        .map_err(|source| transport("flush request", source))
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
        .map_err(|source| transport(&format!("read {m}"), source))?;
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
fn transport(operation: &str, source: std::io::Error) -> HerdrError {
    HerdrError::Transport {
        operation: operation.to_owned(),
        source,
    }
}
fn validate_runtime_schema(runtime: &str) -> Result<(), HerdrError> {
    let baseline: Value =
        serde_json::from_str(SCHEMA_BASELINE).map_err(|e| invalid("schema baseline", e))?;
    let runtime: Value = serde_json::from_str(runtime).map_err(|e| invalid("runtime schema", e))?;
    let protocol = runtime.get("protocol").and_then(Value::as_u64);
    if protocol != Some(u64::from(SUPPORTED_PROTOCOL)) {
        return Err(invalid_msg(
            "runtime schema",
            &format!("expected protocol {SUPPORTED_PROTOCOL}, got {protocol:?}"),
        ));
    }
    if runtime != baseline {
        return Err(invalid_msg(
            "runtime schema",
            "installed Herdr schema differs from checked-in protocol-16 snapshot",
        ));
    }
    Ok(())
}
fn write_trace(path: &Path, row: &Value) {
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{row}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{BoardCollector, BoardError, BoardSnapshot, BoardWorker};
    use crate::god_cli::{GodCliError, GodCollector, GodSnapshot, InboxRow};
    use crate::herdr::test_support::FakeHerdr;
    use crate::socket_backend::{SocketBoardCollector, SocketGodCollector};
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
    fn recording_fake(
        responses: Vec<String>,
    ) -> (
        PathBuf,
        thread::JoinHandle<()>,
        std::sync::Arc<std::sync::Mutex<Vec<Value>>>,
    ) {
        let path = std::env::temp_dir().join(format!(
            "hat-recording-{}-{}.sock",
            std::process::id(),
            rand_id()
        ));
        let _ = fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = requests.clone();
        let h = thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut line = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut line)
                    .unwrap();
                captured
                    .lock()
                    .unwrap()
                    .push(serde_json::from_str(&line).unwrap());
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        (path, h, requests)
    }
    fn persistent_board_fake() -> (
        PathBuf,
        thread::JoinHandle<()>,
        std::sync::Arc<std::sync::Mutex<Vec<Value>>>,
    ) {
        let path = std::env::temp_dir().join(format!(
            "hat-persistent-{}-{}.sock",
            std::process::id(),
            rand_id()
        ));
        let _ = fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = requests.clone();
        let h = thread::spawn(move || {
            for response in [
                pong("herdr-agent-team:0", 16),
                snapshot("herdr-agent-team:1"),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut line = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut line)
                    .unwrap();
                captured
                    .lock()
                    .unwrap()
                    .push(serde_json::from_str(&line).unwrap());
                stream.write_all(response.as_bytes()).unwrap();
            }
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            captured
                .lock()
                .unwrap()
                .push(serde_json::from_str(&line).unwrap());
            stream.write_all(b"{\"id\":\"herdr-agent-team:2\",\"result\":{\"type\":\"subscription_started\"}}\n").unwrap();
            thread::sleep(Duration::from_millis(30));
            let _ = stream.write_all(b"{\"event\":\"pane.agent_status_changed\",\"data\":{\"pane_id\":\"p1\",\"workspace_id\":\"w1\",\"agent_status\":\"blocked\"}}\n");
        });
        (path, h, requests)
    }
    fn silent_fake(hold: Duration, partial: bool) -> (PathBuf, thread::JoinHandle<()>) {
        let path = std::env::temp_dir().join(format!(
            "hat-silent-{}-{}.sock",
            std::process::id(),
            rand_id()
        ));
        let _ = fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let h = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut request)
                .unwrap();
            if partial {
                stream.write_all(b"{\"id\":\"partial").unwrap();
            }
            thread::sleep(hold);
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
        let c = SocketClient::connect_validated(path.clone(), FakeHerdr::default()).unwrap();
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
        let e = SocketClient::connect_validated(path.clone(), FakeHerdr::default())
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
        let e = SocketClient::connect_validated(path.clone(), FakeHerdr::default())
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
        assert!(SocketClient::connect_validated(path.clone(), FakeHerdr::default()).is_err());
        h.join().unwrap();
        let _ = fs::remove_file(path);
    }
    #[test]
    fn bounded_frame_is_enforced() {
        let (path, h) = fake(vec![format!(
            "{}\n",
            "x".repeat(DEFAULT_MAX_FRAME_BYTES + 1)
        )]);
        let e = SocketClient::connect_validated(path.clone(), FakeHerdr::default())
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
        let c = SocketClient::connect_validated(path.clone(), fallback).unwrap();
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
        let client = SocketClient::connect_validated(path.clone(), FakeHerdr::default()).unwrap();
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
        assert!(SocketClient::connect_validated(missing, FakeHerdr::default()).is_err());
        let fallback = FakeHerdr::default();
        assert_eq!(
            fallback
                .workspace_create(Path::new("/tmp"), "x")
                .unwrap()
                .workspace_id,
            "workspace-1"
        );
    }

    #[test]
    fn silent_partial_peer_is_bounded() {
        let (path, h) = silent_fake(Duration::from_millis(300), true);
        let started = Instant::now();
        assert!(SocketClient::connect_validated(path.clone(), FakeHerdr::default()).is_err());
        assert!(started.elapsed() < Duration::from_millis(150));
        h.join().unwrap();
        let _ = fs::remove_file(path);
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
        let socket = SocketClient::connect_validated(path.clone(), FakeHerdr::default()).unwrap();
        let cli = FixtureGod(expected.clone());
        let over_socket = SocketGodCollector::new(socket, cli.clone());
        assert_eq!(cli.collect().unwrap(), over_socket.collect().unwrap());
        h.join().unwrap();
        let _ = fs::remove_file(path);
    }

    #[test]
    fn reconnect_attempts_are_internally_capped() {
        let mut responses = vec![pong("herdr-agent-team:0", 16)];
        for attempt in 0..=MAX_RECONNECTS {
            responses.push(snapshot(&format!("herdr-agent-team:{}", attempt * 2 + 1)));
            responses.push(format!("{{\"id\":\"herdr-agent-team:{}\",\"result\":{{\"type\":\"subscription_started\"}}}}\n", attempt * 2 + 2));
        }
        let (path, h) = fake(responses);
        let client = SocketClient::connect_validated(path.clone(), FakeHerdr::default()).unwrap();
        let mut snapshots = 0;
        client
            .subscribe_with_reconnect(
                &[json!({"type":"pane.agent_status_changed"})],
                usize::MAX,
                |item| {
                    if matches!(item, StreamItem::Snapshot(_)) {
                        snapshots += 1;
                    }
                },
            )
            .unwrap();
        assert_eq!(snapshots, MAX_RECONNECTS + 1);
        h.join().unwrap();
        let _ = fs::remove_file(path);
    }

    #[test]
    fn typed_snapshot_and_event_payloads_reject_missing_fields() {
        let malformed_snapshot = "{\"id\":\"herdr-agent-team:1\",\"result\":{\"type\":\"session_snapshot\",\"snapshot\":{\"version\":\"0.9.0\",\"protocol\":16,\"focused_workspace_id\":null,\"focused_tab_id\":null,\"focused_pane_id\":null,\"workspaces\":[],\"tabs\":[],\"panes\":[{\"pane_id\":\"p1\"}],\"layouts\":[],\"agents\":[]}}}\n";
        let (path, h) = fake(vec![
            pong("herdr-agent-team:0", 16),
            malformed_snapshot.into(),
        ]);
        let client = SocketClient::connect_validated(path.clone(), FakeHerdr::default()).unwrap();
        assert!(client
            .snapshot()
            .unwrap_err()
            .to_string()
            .contains("missing field"));
        h.join().unwrap();
        let _ = fs::remove_file(path);

        let event = "{\"id\":\"herdr-agent-team:1\",\"result\":{\"type\":\"subscription_started\"}}\n{\"event\":\"pane.agent_status_changed\",\"data\":{\"pane_id\":\"p1\"}}\n";
        let (path, h) = fake(vec![pong("herdr-agent-team:0", 16), event.into()]);
        let client = SocketClient::connect_validated(path.clone(), FakeHerdr::default()).unwrap();
        assert!(client
            .wait_for_team_change(&["p1".into()], Duration::from_millis(50))
            .unwrap_err()
            .to_string()
            .contains("missing field"));
        h.join().unwrap();
        let _ = fs::remove_file(path);
    }

    #[test]
    fn trace_never_serializes_untrusted_error_text() {
        let path = std::env::temp_dir().join(format!("hat-trace-{}.jsonl", rand_id()));
        let error = invalid_msg("rpc", "SECRET prompt contents");
        let row = json!({"id":"r1","method":"pane.run","result_type":null,"error_code":if Some(&error).is_some(){Some("transport_or_protocol_error")}else{None},"latency_ms":1});
        write_trace(&path, &row);
        let contents = fs::read_to_string(&path).unwrap();
        assert!(!contents.contains("SECRET"));
        assert!(!contents.contains("prompt contents"));
        assert!(contents.contains("transport_or_protocol_error"));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn runtime_schema_must_exactly_match_checked_in_protocol_16() {
        assert!(validate_runtime_schema(SCHEMA_BASELINE).is_ok());
        let drifted = SCHEMA_BASELINE.replacen("\"protocol\": 16", "\"protocol\": 17", 1);
        assert!(validate_runtime_schema(&drifted)
            .unwrap_err()
            .to_string()
            .contains("expected protocol 16"));
        let drifted = SCHEMA_BASELINE.replacen("\"schema_version\": 1", "\"schema_version\": 2", 1);
        assert!(validate_runtime_schema(&drifted)
            .unwrap_err()
            .to_string()
            .contains("differs"));
    }

    #[derive(Clone)]
    struct FixtureBoard(BoardSnapshot);
    impl BoardCollector for FixtureBoard {
        fn collect(&self) -> Result<BoardSnapshot, BoardError> {
            Ok(self.0.clone())
        }
        fn subscription_panes(&self) -> Vec<String> {
            vec!["p1".into()]
        }
    }

    #[test]
    fn board_bootstraps_snapshot_then_uses_typed_subscription_without_replacing_durable_truth() {
        let event = concat!("{\"id\":\"herdr-agent-team:2\",\"result\":{\"type\":\"subscription_started\"}}\n", "{\"event\":\"pane.agent_status_changed\",\"data\":{\"pane_id\":\"p1\",\"workspace_id\":\"w1\",\"agent_status\":\"blocked\"}}\n");
        let (path, h, requests) = recording_fake(vec![
            pong("herdr-agent-team:0", 16),
            snapshot("herdr-agent-team:1"),
            event.into(),
        ]);
        let socket = SocketClient::connect_validated(path.clone(), FakeHerdr::default()).unwrap();
        let durable = BoardSnapshot {
            team: "wave8".into(),
            run_dir: "/run".into(),
            lifecycle: "active".into(),
            workers: vec![BoardWorker {
                name: "m".into(),
                lifecycle: WorkerLifecycle::Running,
                pane_id: Some("p1".into()),
                task: None,
                report: None,
            }],
            mailbox_events: 0,
        };
        let collector = SocketBoardCollector::new(socket, FixtureBoard(durable.clone()));
        assert_eq!(collector.collect().unwrap(), durable);
        assert!(collector.wait_for_change(Duration::from_millis(50)));
        h.join().unwrap();
        let methods = requests
            .lock()
            .unwrap()
            .iter()
            .map(|request| request["method"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            methods,
            vec!["ping", "session.snapshot", "events.subscribe"]
        );
        assert_eq!(
            requests.lock().unwrap()[2]["params"]["subscriptions"][0]["pane_id"],
            "p1"
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn board_preserves_one_subscription_across_refresh_polls() {
        let (path, h, requests) = persistent_board_fake();
        let socket = SocketClient::connect_validated(path.clone(), FakeHerdr::default()).unwrap();
        let durable = BoardSnapshot {
            team: "wave8".into(),
            run_dir: "/run".into(),
            lifecycle: "active".into(),
            workers: vec![BoardWorker {
                name: "m".into(),
                lifecycle: WorkerLifecycle::Running,
                pane_id: Some("p1".into()),
                task: None,
                report: None,
            }],
            mailbox_events: 0,
        };
        let collector = SocketBoardCollector::new(socket, FixtureBoard(durable));
        collector.collect().unwrap();
        assert!(!collector.wait_for_change(Duration::from_millis(5)));
        assert!(collector.wait_for_change(Duration::from_millis(80)));
        h.join().unwrap();
        let methods = requests
            .lock()
            .unwrap()
            .iter()
            .map(|request| request["method"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            methods,
            vec!["ping", "session.snapshot", "events.subscribe"]
        );
        let _ = fs::remove_file(path);
    }

    #[derive(Clone)]
    struct CountingGod {
        waits: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }
    impl GodCollector for CountingGod {
        fn collect(&self) -> Result<GodSnapshot, GodCliError> {
            Ok(FixtureGod(GodSnapshot {
                run_dir: "/run".into(),
                lifecycle: RunLifecycle::Active,
                rows: vec![],
                worker_lifecycles: vec![],
                statuses: vec![],
            })
            .0)
        }
        fn wait_for_change(&self, timeout: Duration) {
            self.waits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            thread::sleep(timeout)
        }
        fn subscription_panes(&self) -> Vec<String> {
            vec!["p1".into()]
        }
    }

    #[test]
    fn immediate_subscription_error_uses_one_bounded_cli_fallback_sleep() {
        let error="{\"id\":\"herdr-agent-team:1\",\"error\":{\"code\":\"unsupported\",\"message\":\"no subscription\"}}\n";
        let (path, h) = fake(vec![pong("herdr-agent-team:0", 16), error.into()]);
        let socket = SocketClient::connect_validated(path.clone(), FakeHerdr::default()).unwrap();
        let waits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let collector = SocketGodCollector::new(
            socket,
            CountingGod {
                waits: waits.clone(),
            },
        );
        let started = Instant::now();
        collector.wait_for_change(Duration::from_millis(20));
        assert_eq!(waits.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(started.elapsed() >= Duration::from_millis(20));
        assert!(started.elapsed() < Duration::from_millis(50));
        h.join().unwrap();
        let _ = fs::remove_file(path);
    }

    #[test]
    fn transport_module_has_no_collector_adapter_dependencies() {
        let source = include_str!("socket.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        assert!(!source.contains(&["crate", "::board"].concat()));
        assert!(!source.contains(&["crate", "::god_cli"].concat()));
        assert!(!source.contains(&["impl Board", "Collector"].concat()));
        assert!(!source.contains(&["impl God", "Collector"].concat()));
    }

    #[test]
    fn failed_resnapshot_never_allows_replacement_subscription() {
        let malformed_snapshot="{\"id\":\"herdr-agent-team:3\",\"result\":{\"type\":\"session_snapshot\",\"snapshot\":{}}}\n";
        let replacement="{\"id\":\"herdr-agent-team:4\",\"result\":{\"type\":\"subscription_started\"}}\n{\"event\":\"pane.agent_status_changed\",\"data\":{\"pane_id\":\"p1\",\"workspace_id\":\"w1\",\"agent_status\":\"blocked\"}}\n";
        let (path, h, requests) = recording_fake(vec![
            pong("herdr-agent-team:0", 16),
            snapshot("herdr-agent-team:1"),
            "{\"id\":\"herdr-agent-team:2\",\"result\":{\"type\":\"subscription_started\"}}\n"
                .into(),
            malformed_snapshot.into(),
            replacement.into(),
        ]);
        let socket = SocketClient::connect_validated(path.clone(), FakeHerdr::default()).unwrap();
        let durable = BoardSnapshot {
            team: "wave8".into(),
            run_dir: "/run".into(),
            lifecycle: "active".into(),
            workers: vec![],
            mailbox_events: 0,
        };
        let collector = SocketBoardCollector::new(socket, FixtureBoard(durable));
        collector.collect().unwrap();
        assert!(collector.wait_for_change(Duration::from_millis(10)));
        thread::sleep(RECONNECT_BACKOFF + Duration::from_millis(2));
        collector.collect().unwrap();
        let _ = collector.wait_for_change(Duration::from_millis(10));
        drop(collector);
        drop(h);
        let methods = requests
            .lock()
            .unwrap()
            .iter()
            .map(|r| r["method"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            methods,
            vec![
                "ping".to_owned(),
                "session.snapshot".to_owned(),
                "events.subscribe".to_owned(),
                "session.snapshot".to_owned()
            ]
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn terminal_typed_event_error_never_reconnects() {
        let malformed_event="{\"id\":\"herdr-agent-team:2\",\"result\":{\"type\":\"subscription_started\"}}\n{\"event\":\"pane.agent_status_changed\",\"data\":{\"pane_id\":\"p1\"}}\n";
        let (path, h, requests) = recording_fake(vec![
            pong("herdr-agent-team:0", 16),
            snapshot("herdr-agent-team:1"),
            malformed_event.into(),
            snapshot("herdr-agent-team:3"),
            "{\"id\":\"herdr-agent-team:4\",\"result\":{\"type\":\"subscription_started\"}}\n"
                .into(),
        ]);
        let socket = SocketClient::connect_validated(path.clone(), FakeHerdr::default()).unwrap();
        let durable = BoardSnapshot {
            team: "wave8".into(),
            run_dir: "/run".into(),
            lifecycle: "active".into(),
            workers: vec![],
            mailbox_events: 0,
        };
        let collector = SocketBoardCollector::new(socket, FixtureBoard(durable));
        collector.collect().unwrap();
        assert!(collector.wait_for_change(Duration::from_millis(10)));
        collector.collect().unwrap();
        let _ = collector.wait_for_change(Duration::from_millis(10));
        drop(collector);
        drop(h);
        let methods = requests
            .lock()
            .unwrap()
            .iter()
            .map(|r| r["method"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            methods,
            vec![
                "ping".to_owned(),
                "session.snapshot".to_owned(),
                "events.subscribe".to_owned()
            ]
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn production_reconnects_are_capped_and_backed_off() {
        let mut responses = vec![pong("herdr-agent-team:0", 16)];
        for attempt in 0..6 {
            responses.push(snapshot(&format!("herdr-agent-team:{}", attempt * 2 + 1)));
            responses.push(format!("{{\"id\":\"herdr-agent-team:{}\",\"result\":{{\"type\":\"subscription_started\"}}}}\n",attempt*2+2));
        }
        let (path, _h, requests) = recording_fake(responses);
        let socket = SocketClient::connect_validated(path.clone(), FakeHerdr::default()).unwrap();
        let durable = BoardSnapshot {
            team: "wave8".into(),
            run_dir: "/run".into(),
            lifecycle: "active".into(),
            workers: vec![],
            mailbox_events: 0,
        };
        let collector = SocketBoardCollector::new(socket, FixtureBoard(durable));
        let started = Instant::now();
        for _ in 0..6 {
            collector.collect().unwrap();
            let _ = collector.wait_for_change(Duration::from_millis(20));
        }
        let subscribe_count = requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r["method"] == "events.subscribe")
            .count();
        assert!(subscribe_count <= MAX_RECONNECTS + 1);
        assert!(started.elapsed() >= RECONNECT_BACKOFF * MAX_RECONNECTS as u32);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn board_and_god_adapters_share_one_lifecycle_controller() {
        let source = include_str!("socket_backend.rs");
        assert_eq!(
            source.matches("struct CollectorSocketController").count(),
            1
        );
        assert_eq!(source.matches("snapshot_pending").count(), 0);
        assert!(!source.contains("struct CollectorSocketState"));
    }

    #[test]
    fn subscription_ack_and_error_are_traced_without_server_text() {
        let trace_path =
            std::env::temp_dir().join(format!("hat-subscription-trace-{}.jsonl", rand_id()));
        let success =
            "{\"id\":\"herdr-agent-team:1\",\"result\":{\"type\":\"subscription_started\"}}\n";
        let (path, h) = fake(vec![pong("herdr-agent-team:0", 16), success.into()]);
        let mut client =
            SocketClient::connect_validated(path.clone(), FakeHerdr::default()).unwrap();
        client.trace_path = Some(trace_path.clone());
        drop(
            client
                .subscribe(
                    &[json!({"type":"pane.agent_status_changed","pane_id":"p1"})],
                    Duration::from_millis(20),
                )
                .unwrap(),
        );
        h.join().unwrap();
        let _ = fs::remove_file(path);

        let failure="{\"id\":\"herdr-agent-team:1\",\"error\":{\"code\":\"denied\",\"message\":\"SECRET prompt contents\"}}\n";
        let (path, h) = fake(vec![pong("herdr-agent-team:0", 16), failure.into()]);
        let mut client =
            SocketClient::connect_validated(path.clone(), FakeHerdr::default()).unwrap();
        client.trace_path = Some(trace_path.clone());
        assert!(client
            .subscribe(
                &[json!({"type":"pane.agent_status_changed","pane_id":"p1"})],
                Duration::from_millis(20)
            )
            .is_err());
        h.join().unwrap();
        let _ = fs::remove_file(path);
        let rows = fs::read_to_string(&trace_path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .filter(|row| row["method"] == "events.subscribe")
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["result_type"], "subscription_started");
        assert!(rows[0]["error_code"].is_null());
        assert_eq!(rows[1]["error_code"], "transport_or_protocol_error");
        assert!(rows[1]["result_type"].is_null());
        let contents = fs::read_to_string(&trace_path).unwrap();
        assert!(!contents.contains("SECRET"));
        assert!(!contents.contains("prompt contents"));
        let _ = fs::remove_file(trace_path);
    }
}
