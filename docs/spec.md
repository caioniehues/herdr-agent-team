# herdmates — north star specification

Claude Code teammates, native in herdr. This is the destination spec
locked by the wayfinder map (#87, closed 2026-07-17); decisions and
evidence citations live in [ADR-0013](adr/0013-north-star-mission-control.md).
The legacy v1.x orchestration spec (frozen at v1.1.0) is preserved at
[legacy/spec-v1.md](legacy/spec-v1.md).

## 1. Identity

Herdmates makes herdr the **visible home of native Claude Code agent
teams**. Anthropic's agent teams own spawn, mailboxes, membership, and
lifecycle; herdmates hosts them (shim) and observes them (mission
control). Herdr plugin, period — no standalone reader, no web app. We
never re-implement native-team features; we read the documented team
files and drive the herdr CLI.

## 2. Pillars

### 2.1 Teammux shim (proven)

A fake `tmux` on PATH inside herdr panes translating Claude Code's
split-pane tmux calls into `herdr pane` CLI calls, so native teammates
land as real, steerable herdr panes. **Claude-teams-only by charter.**
Proven live end-to-end (#89, 2026-07-17): spawn, live `blockedBy` DAG,
clean dismissal. The ADR-0012 control-mode kill signal is cleared
(ADR-0013). Ground truth: `docs/research/spike-tmux-verbs-2026-07-16/`.

### 2.2 Mission control

The monitor/steer/gate stack over a native team, single-team v1 (data
model enumerates `~/.claude/teams/*` from day one):

- **Signal engine** — the single source of teammate-state facts. Four
  waiting-reason classes with top-down precedence (permission-prompt >
  blocked-on-dependency > stalled > turn-complete), two-tier stalled
  detection (quiet 5m / stalled 10m, transcript-mtime liveness,
  unread-inbox accelerator). Never display a wrong reason — degrade to
  reason-less "waiting". Full design: #92 resolution.
- **Board** — team telemetry: what state is each teammate in and why.
  Two tiers: sidebar tokens (display-only, `pane report-metadata`) and
  the full-screen TUI plugin pane (overview, per-agent rows, flat task
  list from native `~/.claude/tasks/` files, mailbox tail, metadata
  row). Progress is an honest proxy only (done/total, per-task
  elapsed); ETA prediction is banned.
- **Focus pane** — human focus: one next action + decision queue,
  rendered from the focus file. Distinct surface from the board (#90);
  consumes only human-needing items from the same signal engine, so
  the two surfaces cannot disagree.
- **Inbox-write steering** — pre-composed nudge to a stuck teammate,
  human-reviewed and confirmed before write; `.lock` +
  read-filter-atomic-rename discipline (#91). No auto-nudge in v1.
- **Recorder** — minimal append-only log of the engine's classified
  observations; log schema = engine output schema. Replay UI later.
- **Hook companion** — the three team hooks (`TeammateIdle`/
  `TaskCreated`/`TaskCompleted`) as a push source into engine +
  recorder; exit-2 gating capability ships default-off.

## 3. Data sources (v1)

Team files (`~/.claude/teams/`, `~/.claude/tasks/` — schema per #88,
drift-tolerant parsing), the herdr CLI (`HERDR_BIN_PATH`, schema
snapshot discipline), and session-transcript **mtime as a stat only**.
No session-JSONL parsing in v1 (cut; see §5).

## 4. V1 build order (#93, each stage dogfood-able)

| Stage | Deliverable | Dogfood |
|---|---|---|
| 0 | Version reconcile + baseline tag | releasable tree |
| 1 | Signal engine as lib module (attention.rs migrates in); focus pane + sidebar consume it | live reason badges on sidebar |
| 2 | Minimal recorder on the engine | replayable log of a live run |
| 3 | Read-only TUI plugin pane | full-screen board |
| 4 | Jump-to-pane + confirmed nudge (first inbox write) | steer from the board |
| 5 | Hook companion observe-first (gating default-off) | event-driven badges/log |

Shim ships as-is; focus pane already merged (`f0441f4`).

## 5. Cut line (post-v1)

JSONL tier (context bar, cost footer, per-agent activity tail);
task-DAG lane view; comm graph; multi-team board UI; recorder replay
UI; auto-nudge; hook gating on-by-default. Out of scope entirely:
web surfaces, beads task source, multi-orchestrator shim
genericization, ETA prediction, bespoke diff review UI.

## 6. Doctrine

- Evidence hierarchy: live > source > doc (ADR-0010); verify before
  parsing anything; feature-detect preview claims.
- Honesty: never a wrong reason, never a predicted ETA, no silent
  coverage caps.
- Risk layering: deterministic reads immediate; heuristics tiered and
  conservative; writes human-confirmed; blocking opt-in.
- Pushes = releases: gated (fmt/clippy/tests), version-bumped, tagged,
  only on Caio's word.
