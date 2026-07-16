//! Process-edge adapters for the experimental public-socket transport.
//!
//! One controller owns snapshot/subscription/reconnect lifecycle. Board and
//! God wrappers only map its outcomes onto their distinct fallback policies.

use crate::board::{BoardCollector, BoardError, BoardSnapshot};
use crate::god_cli::{GodCliError, GodCollector, GodSnapshot};
use crate::herdr::{
    AgentInfo, HerdrApi, HerdrError, PaneInfo, PaneLayoutSnapshot, WaitOutcome, WorkspaceRef,
    WorktreeRef,
};
use crate::metadata::MetadataUpdate;
use crate::socket::{
    SocketClient, SubscriptionPoll, SubscriptionStream, DEFAULT_IO_TIMEOUT, MAX_RECONNECTS,
    RECONNECT_BACKOFF,
};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

enum LifecyclePhase {
    NeedSnapshot,
    Ready,
    Subscribed(SubscriptionStream),
    Terminal,
    Exhausted,
}

struct LifecycleState {
    phase: LifecyclePhase,
    reconnect_attempts: usize,
    reconnect_deadline: Option<Instant>,
    retry_not_before: Option<Instant>,
}

impl Default for LifecycleState {
    fn default() -> Self {
        Self {
            phase: LifecyclePhase::NeedSnapshot,
            reconnect_attempts: 0,
            reconnect_deadline: None,
            retry_not_before: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ControllerPoll {
    Event,
    Timeout,
    ReconnectPending,
    Terminal,
}

struct CollectorSocketController<C> {
    socket: SocketClient<C>,
    state: Mutex<LifecycleState>,
}

impl<C: HerdrApi> CollectorSocketController<C> {
    fn new(socket: SocketClient<C>) -> Self {
        Self {
            socket,
            state: Mutex::new(LifecycleState::default()),
        }
    }

    fn refresh_snapshot(&self) -> ControllerPoll {
        let mut state = lock(&self.state);
        self.ensure_snapshot(&mut state)
    }

    fn poll(&self, panes: Vec<String>, budget: Duration) -> ControllerPoll {
        if budget.is_zero() {
            return ControllerPoll::Timeout;
        }
        let started = Instant::now();
        let mut state = lock(&self.state);
        match self.ensure_snapshot(&mut state) {
            ControllerPoll::Timeout => {}
            other => return other,
        }
        if matches!(state.phase, LifecyclePhase::Ready) {
            let wanted = subscriptions(panes);
            if wanted.is_empty() {
                return ControllerPoll::Terminal;
            }
            let remaining = remaining(started, budget);
            if remaining.is_zero() {
                return ControllerPoll::Timeout;
            }
            match self.socket.subscribe(&wanted, remaining) {
                Ok(subscription) => state.phase = LifecyclePhase::Subscribed(subscription),
                Err(error) => return self.transition_error(&mut state, error),
            }
        }
        let remaining = remaining(started, budget);
        if remaining.is_zero() {
            return ControllerPoll::Timeout;
        }
        let result = match &mut state.phase {
            LifecyclePhase::Subscribed(subscription) => subscription.poll(remaining),
            LifecyclePhase::Terminal | LifecyclePhase::Exhausted => {
                return ControllerPoll::Terminal
            }
            LifecyclePhase::NeedSnapshot => return ControllerPoll::ReconnectPending,
            LifecyclePhase::Ready => unreachable!("subscription established above"),
        };
        match result {
            Ok(SubscriptionPoll::Event(event)) => {
                drop(event);
                healthy(&mut state);
                ControllerPoll::Event
            }
            Ok(SubscriptionPoll::Timeout) => {
                healthy(&mut state);
                ControllerPoll::Timeout
            }
            Ok(SubscriptionPoll::Closed) => self.transition_loss(&mut state),
            Err(error) => self.transition_error(&mut state, error),
        }
    }

    fn ensure_snapshot(&self, state: &mut LifecycleState) -> ControllerPoll {
        match state.phase {
            LifecyclePhase::Ready | LifecyclePhase::Subscribed(_) => {
                return ControllerPoll::Timeout
            }
            LifecyclePhase::Terminal | LifecyclePhase::Exhausted => {
                return ControllerPoll::Terminal
            }
            LifecyclePhase::NeedSnapshot => {}
        }
        let now = Instant::now();
        if state
            .reconnect_deadline
            .is_some_and(|deadline| now >= deadline)
            || state.reconnect_attempts > MAX_RECONNECTS
        {
            state.phase = LifecyclePhase::Exhausted;
            return ControllerPoll::Terminal;
        }
        if state.retry_not_before.is_some_and(|retry| now < retry) {
            return ControllerPoll::ReconnectPending;
        }
        match self.socket.snapshot() {
            Ok(_) => {
                state.phase = LifecyclePhase::Ready;
                state.retry_not_before = None;
                ControllerPoll::Timeout
            }
            Err(error) => self.transition_error(state, error),
        }
    }

    fn transition_error(&self, state: &mut LifecycleState, error: HerdrError) -> ControllerPoll {
        if matches!(error, HerdrError::Transport { .. }) {
            self.transition_loss(state)
        } else {
            state.phase = LifecyclePhase::Terminal;
            ControllerPoll::Terminal
        }
    }

    fn transition_loss(&self, state: &mut LifecycleState) -> ControllerPoll {
        let now = Instant::now();
        state.reconnect_attempts += 1;
        state.reconnect_deadline.get_or_insert_with(|| {
            now + DEFAULT_IO_TIMEOUT.saturating_mul((MAX_RECONNECTS as u32 + 1) * 3)
        });
        if state.reconnect_attempts > MAX_RECONNECTS {
            state.phase = LifecyclePhase::Exhausted;
            return ControllerPoll::Terminal;
        }
        state.retry_not_before =
            Some(now + RECONNECT_BACKOFF.saturating_mul(state.reconnect_attempts as u32));
        state.phase = LifecyclePhase::NeedSnapshot;
        ControllerPoll::ReconnectPending
    }
}

fn healthy(state: &mut LifecycleState) {
    state.reconnect_attempts = 0;
    state.reconnect_deadline = None;
    state.retry_not_before = None;
}

fn subscriptions(panes: Vec<String>) -> Vec<Value> {
    panes
        .into_iter()
        .map(|pane_id| json!({"type":"pane.agent_status_changed","pane_id":pane_id}))
        .collect()
}
fn remaining(started: Instant, budget: Duration) -> Duration {
    budget.saturating_sub(started.elapsed())
}
fn lock(state: &Mutex<LifecycleState>) -> MutexGuard<'_, LifecycleState> {
    state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub struct SocketGodCollector<C, G> {
    controller: CollectorSocketController<C>,
    fallback: G,
}
impl<C: HerdrApi, G> SocketGodCollector<C, G> {
    pub fn new(socket: SocketClient<C>, fallback: G) -> Self {
        Self {
            controller: CollectorSocketController::new(socket),
            fallback,
        }
    }
}
impl<C: HerdrApi, G: GodCollector> GodCollector for SocketGodCollector<C, G> {
    fn collect(&self) -> Result<GodSnapshot, GodCliError> {
        let _ = self.controller.refresh_snapshot();
        self.fallback.collect()
    }
    fn wait_for_change(&self, timeout: Duration) {
        let started = Instant::now();
        match self
            .controller
            .poll(self.fallback.subscription_panes(), timeout)
        {
            ControllerPoll::Event | ControllerPoll::Timeout => {}
            ControllerPoll::ReconnectPending | ControllerPoll::Terminal => {
                self.fallback.wait_for_change(remaining(started, timeout))
            }
        }
    }
    fn subscription_panes(&self) -> Vec<String> {
        self.fallback.subscription_panes()
    }
}

pub struct SocketBoardCollector<C, B> {
    controller: CollectorSocketController<C>,
    fallback: B,
}
impl<C: HerdrApi, B> SocketBoardCollector<C, B> {
    pub fn new(socket: SocketClient<C>, fallback: B) -> Self {
        Self {
            controller: CollectorSocketController::new(socket),
            fallback,
        }
    }
}
impl<C: HerdrApi, B: BoardCollector> BoardCollector for SocketBoardCollector<C, B> {
    fn collect(&self) -> Result<BoardSnapshot, BoardError> {
        let _ = self.controller.refresh_snapshot();
        self.fallback.collect()
    }
    fn wait_for_change(&self, timeout: Duration) -> bool {
        let started = Instant::now();
        match self
            .controller
            .poll(self.fallback.subscription_panes(), timeout)
        {
            ControllerPoll::Event => true,
            ControllerPoll::Timeout => false,
            ControllerPoll::ReconnectPending | ControllerPoll::Terminal => {
                self.fallback.wait_for_change(remaining(started, timeout))
            }
        }
    }
    fn subscription_panes(&self) -> Vec<String> {
        self.fallback.subscription_panes()
    }
}

impl<C: HerdrApi> HerdrApi for SocketClient<C> {
    fn workspace_create(&self, c: &Path, l: &str) -> Result<WorkspaceRef, HerdrError> {
        self.fallback().workspace_create(c, l)
    }
    fn workspace_close(&self, w: &str) -> Result<(), HerdrError> {
        self.fallback().workspace_close(w)
    }
    fn worktree_create(&self, r: &Path, b: &str) -> Result<WorktreeRef, HerdrError> {
        self.fallback().worktree_create(r, b)
    }
    fn worktree_remove(&self, p: &Path) -> Result<(), HerdrError> {
        self.fallback().worktree_remove(p)
    }
    fn pane_split(&self, w: &str, c: &Path) -> Result<PaneInfo, HerdrError> {
        self.fallback().pane_split(w, c)
    }
    fn pane_split_pane(
        &self,
        target_pane_id: &str,
        direction: &str,
        ratio: Option<f64>,
    ) -> Result<PaneInfo, HerdrError> {
        self.fallback()
            .pane_split_pane(target_pane_id, direction, ratio)
    }
    fn pane_run(&self, p: &str, i: &str) -> Result<(), HerdrError> {
        self.fallback().pane_run(p, i)
    }
    fn pane_read(&self, p: &str) -> Result<String, HerdrError> {
        self.fallback().pane_read(p)
    }
    fn pane_rename(&self, p: &str, t: &str) -> Result<(), HerdrError> {
        self.fallback().pane_rename(p, t)
    }
    fn pane_close(&self, p: &str) -> Result<(), HerdrError> {
        self.fallback().pane_close(p)
    }
    fn pane_resize(&self, p: &str, direction: &str, amount: Option<f64>) -> Result<(), HerdrError> {
        self.fallback().pane_resize(p, direction, amount)
    }
    fn agent_wait(&self, p: &str, s: &str, t: Duration) -> Result<WaitOutcome, HerdrError> {
        self.fallback().agent_wait(p, s, t)
    }
    fn agent_list(&self) -> Result<Vec<AgentInfo>, HerdrError> {
        self.fallback().agent_list()
    }
    fn pane_get(&self, p: &str) -> Result<PaneInfo, HerdrError> {
        self.fallback().pane_get(p)
    }
    fn pane_list(&self, w: Option<&str>) -> Result<Vec<PaneInfo>, HerdrError> {
        self.fallback().pane_list(w)
    }
    fn pane_layout(&self, p: &str) -> Result<PaneLayoutSnapshot, HerdrError> {
        self.fallback().pane_layout(p)
    }
    fn api_schema(&self) -> Result<String, HerdrError> {
        self.fallback().api_schema()
    }
    fn pane_report_metadata(&self, p: &str, u: &MetadataUpdate) -> Result<(), HerdrError> {
        self.fallback().pane_report_metadata(p, u)
    }
    fn notification_show(&self, t: &str, b: &str, s: &str) -> Result<(), HerdrError> {
        self.fallback().notification_show(t, b, s)
    }
}
