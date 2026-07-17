# Mission-Control Feature Landscape for Local-First Claude-Teams Supervision

Synthesized from 27 verified tool-feature claims and 12 deep-read extracts across 12 tools. Framing throughout: a **read-only, local-first supervisor** that reconstructs state by polling `~/.claude/teams/{team}/inboxes/*.json` and `~/.claude/tasks/{team}/`, rendered first as a herdr TUI sidebar and later as a rich local web view.

The single most important architectural fact for prioritization: **native Claude Code Agent Teams already persist their entire coordination state to disk** — mailbox messages and locked task files with dependencies. A file-reader inherits, for free, data that every hook-based competitor has to instrument. That flips several "differentiators" below into cheap table stakes *for this specific architecture*, and it exposes gaps (mailbox graph, task DAG, status-lag detection) that no shipping tool fills.

---

## TIER 1 — TABLE STAKES
*Multiple tools ship it; users now expect it. Absence reads as "not a supervision tool."*

### 1.1 At-a-glance multi-agent overview
- **What:** One surface listing every active agent/session with live status, so a human running N agents never has to hunt across terminals.
- **Ships it:** cmux ([github.com/manaflow-ai/cmux](https://github.com/manaflow-ai/cmux)), Conductor ([conductor.build](https://www.conductor.build/)), Crystal/Nimbalyst ([github.com/stravu/crystal](https://github.com/stravu/crystal)), Vibe Kanban ([github.com/BloopAI/vibe-kanban](https://github.com/BloopAI/vibe-kanban)), claude-squad ([github.com/smtg-ai/claude-squad](https://github.com/smtg-ai/claude-squad)), Claude-Code-Agent-Monitor ([github.com/hoangsonww/Claude-Code-Agent-Monitor](https://github.com/hoangsonww/Claude-Code-Agent-Monitor)), Stargx dashboard ([github.com/Stargx/claude-code-dashboard](https://github.com/Stargx/claude-code-dashboard)).
- **Why for us:** The whole point. The native "live agent panel" already exists inside a Claude Code lead session, but it's trapped in one terminal — surfacing it in a persistent sidebar is the base deliverable.
- **Fit:** TUI-now (list of rows: agent name, status glyph, current task). Web-later adds density.

### 1.2 Live status / state detection per agent
- **What:** Thinking / working / waiting / idle / stale / error indicators, updated in real time.
- **Ships it:** Stargx dashboard (thinking/waiting/idle/stale badges), Agent-Monitor (Working/Waiting/Completed/Error), Crystal (real-time status), native teams (idle notifications).
- **Why for us:** Derivable from file mtimes + inbox activity + task-file lock state without any hook. Freshness of `tasks/{team}/*.json` and inbox writes is a usable liveness signal.
- **Fit:** TUI-now (colored glyph per row). Trivially first thing to build.

### 1.3 Diff review before merge
- **What:** See what an agent changed and approve/reject before it lands.
- **Ships it:** Conductor (diff viewer, Cmd+Shift+D, per [/docs/concepts/workflow](https://www.conductor.build/docs/concepts/workflow)), Vibe Kanban (line-by-line + inline comments back to agent), claude-squad (preview diffs, `c` checkout-to-pause), container-use (git checkout across branches), Sculptor (shows diff before applying).
- **Why for us:** *Caveat* — native teams share one workspace via file mailboxes, **not** per-agent worktrees, so there's no per-agent branch to diff. This table-stake is architecturally awkward for us; the honest position is to lean on `git diff` / existing tooling (container-use's stance) rather than build a bespoke diff UI. Flag as "borrowed from git, not owned."
- **Fit:** Web-later (diffs are painful in a narrow TUI sidebar). Low priority given the shared-workspace mismatch.

### 1.4 Waiting / needs-attention notification
- **What:** Signal the human when an agent is blocked on input.
- **Ships it:** cmux (notification rings + centralized panel), native teams (automatic idle notifications to lead), Agent-Monitor (waiting-for-input banner), Stargx (waiting indicator).
- **Why for us:** Idle notifications are already emitted to the lead and land in mailbox files — a reader can detect "teammate idle, awaiting" directly. This is high-value and cheap.
- **Fit:** TUI-now (highlight the row + optional bell). See §2.1 for the differentiator version.

### 1.5 Cost / token tracking
- **What:** Per-session and aggregate token counts and dollar cost, with per-model pricing.
- **Ships it:** Agent-Monitor (per-model, per-subagent breakdown), Stargx (per-session + combined, model-specific pricing), claude-view ([recca0120.github.io](https://recca0120.github.io/en/2026/04/07/claude-view-mission-control/)) (per-agent cost/token).
- **Why for us:** Users expect it, but note the honest limit: **Vibe Kanban and claude-squad ship no cost tracking at all** and remain credible supervisors — so it's table stakes for the *dashboard* class, optional for the *TUI* class. Token/cost data lives in `~/.claude/projects/` JSONL, not the team files, so it's a second data source.
- **Fit:** Web-later (charts). A single aggregate number fits the TUI footer.

### 1.6 Direct intervention / steering
- **What:** Drop into a live agent to take control when it's stuck.
- **Ships it:** container-use ("drop into any agent's terminal"), claude-squad (attach-and-type), cmux (resume commands), native (message any teammate via SendMessage).
- **Why for us:** A pure file-reader is **read-only** — it can *route the human to* the right pane (herdr can focus it) but cannot itself inject steering unless it writes to an inbox file. Writing an inbox JSON is feasible and is the natural "reply from the dashboard" feature. Decide deliberately whether the tool stays observe-only.
- **Fit:** TUI-now for "jump to pane" (herdr focus). Web-later for compose-a-message-to-teammate.

### 1.7 Metadata sidebar (branch / dir / ports)
- **What:** git branch, working dir, listening ports, PR status per workspace.
- **Ships it:** cmux (branch, PR status/number, dir, ports, latest notification), Stargx (git branch, permission-mode badges).
- **Fit:** TUI-now. Directly models the herdr sidebar target.

---

## TIER 2 — DIFFERENTIATORS
*Rare, with evidence it matters. Several are unusually cheap for a file-reader and should be the product's spine.*

### 2.1 Waiting-**reason** classification
- **What:** Distinguish *permission-prompt* vs *turn-completion* vs *interruption/hung* — not just "waiting."
- **Ships it:** Agent-Monitor (banner with the three reasons + elapsed time), motivated explicitly by cmux's own criticism that *"Claude Code's notification body is always just 'waiting for your input' with no context."*
- **Why it matters (evidence):** cmux's creator named the contextless notification as a core pain; Agent-Monitor built a whole banner around fixing it. Users demonstrably feel this.
- **Why for us:** Best-in-class opportunity. Task-file states + inbox message types + elapsed time since last write let a reader infer reason **without hooks**. Agent-Monitor admits its own detection is "unreliable… depends on Claude Code's `awaiting_reason` signal" — a file-reader watching the team substrate may actually do this *better*.
- **Fit:** TUI-now (reason as a short tag on the waiting row). This should be a flagship.

### 2.2 Task DAG with automatic dependency tracking
- **What:** Shared task list where completing one task unblocks dependents; visualize the dependency graph.
- **Ships it:** Native teams (shared task list with auto blocks/unblocks) — the data is *already in* `~/.claude/tasks/{team}/`. Agent-Monitor/Vibe Kanban ship kanban *columns* but not native dependency edges.
- **Why for us:** Nobody renders the native task DAG. The reader owns this data outright. This is the closest thing to Flurry's "roadmap."
- **Fit:** TUI-now as a flat ordered task list with blocked/ready flags; **web-later** for the actual DAG graph (edges are hard in a TUI).

### 2.3 Mailbox message-graph visibility
- **What:** Show who messaged whom — the peer-to-peer SendMessage traffic between teammates and broadcasts.
- **Ships it:** **Nobody visualizes this.** Native teams *generate* it (mailbox system, [code.claude.com/docs/en/agent-teams](https://code.claude.com/docs/en/agent-teams)); no supervisor surfaces it.
- **Why for us:** `inboxes/*.json` is literally the reader's primary input. A live tail of inter-agent messages, and later a communication graph, is a unique capability no competitor can match without reverse-engineering the same files.
- **Fit:** TUI-now (chronological message tail, like a chat log). Web-later (node-edge comm graph).

### 2.4 Task-status-**lag** detection (a native failure the reader can catch)
- **What:** Alert when a teammate finished work but never marked its task complete, leaving dependents blocked indefinitely.
- **Ships it:** **Nobody.** It's a *documented native bug*: "teammates sometimes fail to mark tasks complete, blocking dependents indefinitely."
- **Why for us:** A file-reader watching task-file lock/state vs. teammate idle-status can detect exactly this deadlock — turning a known native pain point into a headline feature. High differentiation, directly enabled by the architecture.
- **Fit:** TUI-now (a warning badge on the stuck task).

### 2.5 Ground-truth command/activity log (did vs. claimed)
- **What:** Complete command history and logs of what an agent *actually did*, not its summary.
- **Ships it:** container-use ("complete command history and logs… not just what it claims"), claude-view (tool calls / bash / file ops as cards), Agent-Monitor (activity feed).
- **Why for us:** Aligns with the user's own verification doctrine ("verify with real evidence, not summaries"). Session JSONL under `~/.claude/projects/` carries the real tool calls.
- **Fit:** Web-later (dense card stream). TUI-now can offer a per-agent "recent activity" tail.

### 2.6 Subagent / teammate hierarchy tree with per-node cost
- **What:** Collapsible parent→child agent tree, each node showing its own cost/token draw.
- **Ships it:** Agent-Monitor (collapsible tree, per-subagent cost), claude-view (sub-agent tree with per-agent breakdown).
- **Why for us:** Native teams are flat-ish (no nested teams — see anti-features), so the tree is shallow: lead → teammates. Still valuable for cost attribution.
- **Fit:** Web-later (tree UI). TUI-now can indent teammates under the lead.

### 2.7 Context-window utilization
- **What:** Per-session progress bar showing how full each agent's context is.
- **Ships it:** **Only Stargx dashboard.** Rare, and directly predictive of imminent degradation/compaction.
- **Why for us:** Early-warning that an agent is about to lose the plot — high signal, and computable from session token counts.
- **Fit:** TUI-now (a compact bar per row). Undervalued; cheap to ship.

### 2.8 Notification rings / findability across many panes
- **What:** Visual highlight (blue ring, tab glow) that makes a blocked agent locatable across many splits.
- **Ships it:** cmux (unique), motivated by "with enough tabs open I couldn't even read the titles anymore."
- **Why for us:** herdr is the pane substrate; the sidebar equivalent is highlighting the row and offering jump-to-pane.
- **Fit:** TUI-now (row highlight + herdr focus action).

### 2.9 Active-files awareness
- **What:** Which files each agent is currently touching.
- **Ships it:** Stargx dashboard, Mission Control (workspace sync). Rare.
- **Why for us:** Cheap collision-detection signal (two agents editing the same file in a shared workspace — a real hazard given native teams share one tree).
- **Fit:** TUI-now (one line per agent). Doubles as a same-file-conflict warning.

### 2.10 Full-text search across sessions
- **What:** Query all historical sessions fast.
- **Ships it:** claude-view (Tantivy, <50ms over 1,500 sessions).
- **Fit:** Web-later only. Nice-to-have; not core to live supervision.

### 2.11 Alerts / webhooks rules engine
- **What:** Fire on inactivity, stuck agents, token thresholds, event patterns → Slack/Discord/etc.
- **Ships it:** Agent-Monitor (14 providers).
- **Why for us:** For an away-from-keyboard user, a push on "agent blocked >5 min" or "task-status deadlock" is the payoff of local-first monitoring. Start with one local notifier, not 14 integrations.
- **Fit:** Web-later for config UI; the *engine* can run headless behind the TUI now.

### 2.12 Plan-approval / review-before-execute gate
- **What:** Force an agent to present a plan for human approval before it edits.
- **Ships it:** Conductor Plan Mode ([/docs/concepts/agent-modes](https://www.conductor.build/docs/concepts/agent-modes)), native plan-approval workflow, Sculptor Suggestions (beta), Mission Control "Aegis" gate.
- **Why for us:** **A read-only reader cannot gate execution** — gating requires hooks (native uses `TeammateIdle`/`TaskCreated`/`TaskCompleted` with exit-code-2 enforcement). Honest scope: the reader can *surface* a pending plan awaiting approval and route the human to approve in-pane; it cannot itself block. Note this boundary explicitly.
- **Fit:** TUI-now to *surface* "plan awaiting approval"; actual gating is out of a pure reader's scope.

---

## TIER 3 — UNPROVEN IDEAS
*Proposed but unvalidated. Interesting, but no shipping evidence they work — build behind a flag, if at all.*

### 3.1 ETA / time-to-completion prediction
- **What:** Estimate when a long-horizon task will finish.
- **Status:** From **Flurry's mission-control concept**, and *nobody ships it* — Sculptor's extract explicitly notes "no evidence of… ETA prediction (vs. Nathan Flurry's July-2026 concept)." Flurry himself caveats accuracy.
- **Assessment:** Highest-visibility idea, weakest evidence. Agent runtimes are famously non-stationary; a naive ETA will be wrong and erode trust. If attempted, frame as a wide range, never a countdown, and gate behind explicit opt-in.
- **Fit:** Web-later, experimental only.

### 3.2 Human-friendly progress log / roadmap synthesis
- **What:** Auto-generate a readable narrative of what the team has done and plans to do.
- **Status:** Flurry concept. Partially approximable from the task DAG (§2.2) + mailbox log (§2.3), but "human-friendly artifact" synthesis (summarizing raw activity into prose) is unproven at this granularity.
- **Assessment:** The *raw* version (task list + message tail) is Tier-2 real; the *synthesized narrative* is the unproven part. Ship raw first; treat synthesis as speculative.
- **Fit:** Web-later.

### 3.3 ROI / contribution accounting
- **What:** Lines changed, files touched, commits, **cost-per-commit**.
- **Status:** claude-view ships it, but as its own framing — no evidence users act on cost-per-commit. Sculptor/others omit it.
- **Assessment:** Vanity-metric risk. Defer.
- **Fit:** Web-later.

### 3.4 "Fluency score" / effectiveness metric
- **What:** A 0–100 score for "how effectively you use AI."
- **Status:** claude-view ships it **self-labeled experimental**. No validation.
- **Assessment:** Skip. Composite scores without a validated model mislead.

### 3.5 Proactive codebase issue-scanning → fix-agent-per-issue
- **What:** Scan repo for bugs (missing tests, race conditions, leaks), spawn a fix agent per finding, show diff before apply.
- **Status:** **Only Sculptor** ([imbue.com/blog/sculptor](https://imbue.com/blog/sculptor)) ships this. Valuable but singular — unproven as a *supervision* feature vs. a separate agent product, and orthogonal to reading team files.
- **Assessment:** Out of scope for a supervisor; it's an agent-launcher, not a monitor.

### 3.6 Instruction audits / policy-violation detection
- **What:** Catch when an agent violates governance policy.
- **Status:** Sculptor roadmap (deferred), Mission Control governance (alpha). Nobody ships maturely.
- **Assessment:** Speculative; requires defining policy DSL. Not now.

---

## (a) Criticisms & anti-features noted in sources

| Anti-feature / criticism | Source | Lesson for a local-first reader |
|---|---|---|
| Notification body is always "waiting for your input," no context | cmux creator | Do **not** replicate contextless alerts — classify the reason (§2.1). |
| Too many tabs → titles unreadable | cmux | Sidebar must stay scannable; don't just mirror tabs. |
| **Monitors but cannot block/intercept tool calls mid-flight** | Agent-Monitor | A file-reader is inherently observe-only; gating needs hooks. State this boundary. |
| Waiting-state detection unreliable; can't tell input-block from hung | Agent-Monitor | Use multiple file signals (mtime + inbox + task lock), don't trust one flag. |
| Subagent tool attribution is reconstructive (back-filled parsing), not live | Agent-Monitor | Reconstructing from JSONL has lag; be honest about staleness. |
| Session resumption fails — `/resume`,`/rewind` don't restore in-process teammates | native teams | The reader may be the *only* durable record after a lead crashes — persist observed state. |
| **Task-status lag blocks dependents indefinitely** | native teams | Turn this bug into a feature (§2.4). |
| Orphaned tmux sessions persist after exit; manual cleanup | native teams | A reader could detect and flag orphans (unclaimed gap). |
| Split-pane needs tmux/iTerm2 — **not Ghostty/VS Code/Windows Terminal** | native teams | herdr is the display layer precisely because native split-pane is limited — reinforces the architecture choice. |
| Single team per session; no nested teams; no cross-session sharing | native teams | Scope v1 to one team; multi-team is a gap (below). |
| Auto-accept/YOLO modes flagged experimental | claude-squad, Stargx (YOLO badge) | Surface permission mode prominently as a risk badge (Stargx does this well). |
| tmux startup timeouts, doc load errors | claude-squad | Reliability of the substrate matters more than features. |
| Vibe Kanban **sunsetting/discontinued** | Vibe Kanban README | Its feature set is reference-only; don't cite as a living competitor. |
| Suggestions beta; forking & instruction-audits deferred to roadmap | Sculptor | Even the richest tool punts governance — don't over-scope v1. |
| Alpha schema volatility | Mission Control | Local-first schema churn is real; keep the file-reader tolerant of format drift. |
| Goals unavailable in cloud (local-only) | Conductor | Local-first is a genuine differentiator others treat as a limitation. |

**Cross-cutting anti-pattern:** the field splits into *monitors* (observe, never block: Agent-Monitor, Stargx, claude-view) and *gates* (block execution: native hooks, Conductor Plan Mode, Mission Control Aegis). A pure file-reader is structurally a **monitor**. Choose that identity deliberately, or add a hook companion to gate — don't pretend a reader can enforce.

---

## (b) Gaps nobody ships (whitespace for this architecture)

1. **Native mailbox message graph** — the SendMessage peer-to-peer traffic is persisted but *no tool visualizes it*. Uniquely ours (§2.3).
2. **Native task-dependency DAG rendering** — data exists in `tasks/{team}/`; no supervisor draws the edges (§2.2).
3. **Task-status-lag / deadlock detection** — the documented native "dependents blocked indefinitely" bug has no watcher (§2.4).
4. **Orphaned-tmux / dead-teammate detection** — native leaves orphans; nobody flags them.
5. **Same-file-collision warning in a shared workspace** — every other tool isolates via worktrees/containers, so none needed this; native teams share one tree, making it a *novel* hazard only a shared-workspace supervisor must solve (partial basis in §2.9 active-files).
6. **Durable record surviving lead crash** — since native resume fails, an external reader is the only thing that can persist "what the team was doing" across a crash. No tool positions itself as the black-box recorder.
7. **ETA / long-horizon progress with honesty** — the Flurry gap; unproven but genuinely unfilled (§3.1).
8. **Multi-team / cross-session overview** — native is single-team; no supervisor aggregates multiple teams into one board.
9. **Reason-aware, away-from-keyboard push** that fires on *semantic* team events (deadlock, plan-awaiting-approval), not just generic inactivity — Agent-Monitor's webhooks are rule-based on generic signals, not team-semantic.

---

## Build-order recommendation (one decisive call)

For **herdr-TUI-now**, the cheapest-yet-most-differentiated spine is the set of features the file substrate hands you nearly for free and nobody else ships: **§1.1 overview + §1.2 status + §2.1 waiting-reason + §2.2 task list (flat) + §2.3 mailbox tail + §2.4 status-lag detection + §2.7 context bar + §1.7 metadata sidebar.** All are single-line-per-agent TUI rows driven purely by polling team files and session JSONL — no hooks, no worktrees, no cloud.

Defer to **web-later** everything that needs pixels: DAG graph (§2.2), comm graph (§2.3), subagent tree (§2.6), cost charts (§1.5), activity cards (§2.5), full-text search (§2.10).

Treat **diff review (§1.3)** and **plan gating (§2.12)** as architecturally out-of-lane for a read-only reader — borrow git and hooks respectively rather than build — and keep **all of Tier 3 behind flags**, ETA most cautiously of all.
