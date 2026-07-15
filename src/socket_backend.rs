//! Process-edge adapters for the experimental public-socket transport.
//!
//! Socket events only wake durable collectors. Completion and board truth
//! remain in `run.toml` and the inbox.

use crate::board::{BoardCollector, BoardError, BoardSnapshot};
use crate::god_cli::{GodCliError, GodCollector, GodSnapshot};
use crate::herdr::{
    AgentInfo, HerdrApi, HerdrError, PaneInfo, WaitOutcome, WorkspaceRef, WorktreeRef,
};
use crate::metadata::MetadataUpdate;
use crate::socket::{SocketClient, SubscriptionPoll, SubscriptionStream};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

struct CollectorSocketState {
    subscription: Option<SubscriptionStream>,
    snapshot_pending: bool,
}

impl Default for CollectorSocketState {
    fn default() -> Self {
        Self {
            subscription: None,
            snapshot_pending: true,
        }
    }
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

fn lock(state: &Mutex<CollectorSocketState>) -> MutexGuard<'_, CollectorSocketState> {
    state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub struct SocketGodCollector<C, G> {
    socket: SocketClient<C>,
    fallback: G,
    state: Mutex<CollectorSocketState>,
}

impl<C, G> SocketGodCollector<C, G> {
    pub fn new(socket: SocketClient<C>, fallback: G) -> Self {
        Self {
            socket,
            fallback,
            state: Mutex::new(CollectorSocketState::default()),
        }
    }
}

impl<C: HerdrApi, G: GodCollector> GodCollector for SocketGodCollector<C, G> {
    fn collect(&self) -> Result<GodSnapshot, GodCliError> {
        let mut state = lock(&self.state);
        if state.snapshot_pending && self.socket.snapshot().is_ok() {
            state.snapshot_pending = false;
        }
        drop(state);
        self.fallback.collect()
    }

    fn wait_for_change(&self, timeout: Duration) {
        let started = Instant::now();
        let mut state = lock(&self.state);
        if state.subscription.is_none() {
            let wanted = subscriptions(self.fallback.subscription_panes());
            if wanted.is_empty() {
                drop(state);
                self.fallback.wait_for_change(remaining(started, timeout));
                return;
            }
            let budget = remaining(started, timeout);
            if budget.is_zero() {
                return;
            }
            match self.socket.subscribe(&wanted, budget) {
                Ok(subscription) => state.subscription = Some(subscription),
                Err(_) => {
                    state.snapshot_pending = true;
                    drop(state);
                    self.fallback.wait_for_change(remaining(started, timeout));
                    return;
                }
            }
        }
        let budget = remaining(started, timeout);
        if budget.is_zero() {
            return;
        }
        let poll = state
            .subscription
            .as_mut()
            .expect("subscription initialized")
            .poll(budget);
        if matches!(poll, Ok(SubscriptionPoll::Closed) | Err(_)) {
            state.subscription = None;
            state.snapshot_pending = true;
            drop(state);
            self.fallback.wait_for_change(remaining(started, timeout));
        }
    }

    fn subscription_panes(&self) -> Vec<String> {
        self.fallback.subscription_panes()
    }
}

pub struct SocketBoardCollector<C, B> {
    socket: SocketClient<C>,
    fallback: B,
    state: Mutex<CollectorSocketState>,
}

impl<C, B> SocketBoardCollector<C, B> {
    pub fn new(socket: SocketClient<C>, fallback: B) -> Self {
        Self {
            socket,
            fallback,
            state: Mutex::new(CollectorSocketState::default()),
        }
    }
}

impl<C: HerdrApi, B: BoardCollector> BoardCollector for SocketBoardCollector<C, B> {
    fn collect(&self) -> Result<BoardSnapshot, BoardError> {
        let mut state = lock(&self.state);
        if state.snapshot_pending && self.socket.snapshot().is_ok() {
            state.snapshot_pending = false;
        }
        drop(state);
        self.fallback.collect()
    }

    fn wait_for_change(&self, timeout: Duration) -> bool {
        let started = Instant::now();
        let mut state = lock(&self.state);
        if state.subscription.is_none() {
            let wanted = subscriptions(self.fallback.subscription_panes());
            if wanted.is_empty() {
                return false;
            }
            let budget = remaining(started, timeout);
            if budget.is_zero() {
                return false;
            }
            match self.socket.subscribe(&wanted, budget) {
                Ok(subscription) => state.subscription = Some(subscription),
                Err(_) => {
                    state.snapshot_pending = true;
                    return true;
                }
            }
        }
        let budget = remaining(started, timeout);
        if budget.is_zero() {
            return false;
        }
        match state
            .subscription
            .as_mut()
            .expect("subscription initialized")
            .poll(budget)
        {
            Ok(SubscriptionPoll::Event(event)) => {
                drop(event);
                true
            }
            Ok(SubscriptionPoll::Timeout) => false,
            Ok(SubscriptionPoll::Closed) | Err(_) => {
                state.subscription = None;
                state.snapshot_pending = true;
                true
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
    fn pane_run(&self, p: &str, i: &str) -> Result<(), HerdrError> {
        self.fallback().pane_run(p, i)
    }
    fn pane_read(&self, p: &str) -> Result<String, HerdrError> {
        self.fallback().pane_read(p)
    }
    fn pane_rename(&self, p: &str, t: &str) -> Result<(), HerdrError> {
        self.fallback().pane_rename(p, t)
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
