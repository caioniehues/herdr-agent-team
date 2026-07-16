# ADR-0012 — Pivot: native Claude Code teams in herdr (herdmates)

Date: 2026-07-16
Status: accepted
Supersedes the product direction behind ADR-0002/0003/0006/0007/0008/0009
(those ADRs remain accurate history for the legacy orchestration surface).

## Context

The plugin was built to orchestrate heterogeneous coding-agent teams (spawn,
brief, message, monitor, tear down) because nothing else could do it inside
herdr. Two things changed:

1. **Codex workers are banned** (Caio, 2026-07-15). Our only live use is
   claude-only teams, which erases the heterogeneity premise for now.
2. **Claude Code native agent teams matured.** Verified against official docs
   (live-fetched 2026-07-16, code.claude.com/docs/en/agent-teams.md and
   terminal-config.md):
   - `teammateMode` supports `in-process` (default), `auto`, `tmux`,
     `iterm2`. Split-pane display is **hardcoded to tmux and iTerm2**;
     Ghostty explicitly unsupported; no pluggable backend, env var, or spawn
     template exists.
   - Each split-pane teammate is a **full independent `claude` session** in
     its own pane, directly steerable.
   - Team membership is fixed at spawn (session IDs in
     `~/.claude/teams/{team}/config.json`); mailboxes are JSON files under
     `inboxes/`. No adoption path for externally launched sessions.
   - Feature is experimental (`CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS`).
3. **Herdr has no tmux surface** (verified against upstream source,
   2026-07-16): never sets `TMUX`, no control-mode emulation, "Herdr is not
   tmux" (website/agent-guide.md). The only pane-creation surface for child
   processes is the `herdr` CLI over `HERDR_SOCKET_PATH`
   (`herdr pane split --current …`).
4. **Herdr 0.7.4 shipped the observability surface we lacked:** plugin panes
   (`plugin.pane.open`, placements overlay/split/tab/zoomed), session-modal
   popup panes (#1125), `pane report-metadata` display tokens rendered by
   configurable `[ui.sidebar.agents] rows`, `session.snapshot`,
   `workspace.metadata_updated` and `layout.updated` events. Native
   non-terminal plugin UI is explicitly "a later surface" — the sidebar
   token system and TUI-in-pane are the extension points that exist.

Caio's judgment call: native teammate mode will always beat our re-built
orchestration for claude-only teams — "no point reinventing the wheel;
it will not be better, it will be worse." Marketplace install base is
assessed as zero, so there is no compatibility obligation to the legacy
surface.

## Decision

Pivot this repo — renamed **`caioniehues/herdmates`**, plugin id
**`herdmates`**, tagline *"Claude Code teammates, native in herdr"* — from
building its own orchestration to **integrating Claude Code's native agent
teams into herdr**, in one repo with three surfaces:

1. **Shim (the "teammux" mechanism).** A fake `tmux` executable on `PATH`
   inside herdr panes (plus `TMUX` env), translating the tmux CLI calls
   Claude Code's split-pane mode makes into `herdr pane …` calls, so native
   teammates materialize as real, steerable herdr panes. Gated on a
   **recon-first spike** (below) because the tmux call surface is
   undocumented.
2. **Agent board.** D1: hooks pump teammate state (from
   `~/.claude/teams/*/config.json` + inbox JSONs) into sidebar tokens via
   `pane report-metadata --token …`, rendered by `[ui.sidebar.agents] rows`
   — the herdr sidebar becomes the board with zero rendering code. D2
   (later): a rich interactive TUI board as a zoomed/overlay plugin pane,
   bootstrapped from `session.snapshot` + event subscriptions.
3. **Focus pane.** A human-focus surface (ADHD-harness pattern) rendering
   current task, the single next action, and the decision queue from a
   plain-file contract at `~/.local/share/herdmates/focus.md`. A companion
   atomizer skill (modeled on dhasson04/human-harness, **copied not
   depended**, same rule as ADR-0005) writes that file. File contract, not
   Claude-internals coupling: editable by hand, survives upstream churn.

Boards read the documented team files and work with **in-process teams in
any terminal** — the shim is upside, not a foundation dependency.

### Legacy surface

- Frontier tickets #66–#83 closed `wontfix` (pivot); #77/#79 decisions moot.
  The four prepared batch worktrees are deleted (they were empty).
- `integrate/program-wave1` (v1.0.0 + 7 reviewed merges, 197 tests) merges
  to main and is tagged **v1.1.0** — the tombstone release of the
  orchestration line. Pivot work targets **2.0.0**.
- Legacy spawn/msg/teardown code remains in-tree at v1.1.0 but receives no
  further investment; removal is a 2.0.0-scope decision once boards replace
  its remaining value.

### Spike gate (shim go/no-go)

Recon before build, ~1 worker-day: run a real native team inside real tmux
with a logging `tmux` wrapper first on `PATH` (log argv verbatim, forward to
real tmux); produce the complete verb inventory and a verb→herdr-CLI mapping
table. **Kill signals:** Claude Code drives tmux via control mode (`tmux
-C`, a persistent bidirectional protocol, not discrete CLI calls) or uses
verbs with no herdr equivalent. A killed shim does not kill the pivot —
boards stand alone; the fallback posture is in-process teams + boards, plus
an upstream feature request for a pluggable `teammateMode` backend
(precedent: iTerm2 backend was added in a point release, v2.1.186, so the
backend list demonstrably extends).

### Execution order

1. Foundation: merge + tag v1.1.0, close #66–#83, delete batch worktrees,
   rename repo/manifest.
2. Recon spike → shim verdict.
3. D1 sidebar-token agent board.
4. D3 focus pane.
5. D2 rich TUI board / shim build, per spike verdict.

## Consequences

- **Fragility is quarantined by design:** the shim tracks undocumented,
  experimental Claude Code behavior and will break on upstream releases;
  boards and focus pane read documented file formats and herdr APIs. The
  product degrades gracefully to "boards over in-process teams".
- One repo couples shim and board release cadence. Accepted while the
  install base is zero; splitting the shim out later is cheap (separate
  binary already).
- The team-spec/msg/outbox domain model (CONTEXT.md legacy section) stops
  evolving; the reviewed correctness work shipped in v1.1.0 is preserved in
  history rather than extended.
- We inherit upstream risk in the other direction too: if Anthropic ships a
  pluggable teammate backend or herdr ships native team UI, surfaces here
  shrink to the board/focus layer — which is the part users see anyway.

## Alternatives rejected

- **Keep building our own orchestration** (frontier #66–#83): loses to
  native teams on mailboxes, lifecycle, and maintenance the moment teams
  exit experimental; judged "worse wheel" by the owner.
- **Separate shim repo** (original call this session): protects an install
  base that doesn't exist; costs cross-repo coordination now.
- **Upstream tmux emulation in herdr:** contradicts herdr's stated identity
  ("Herdr is not tmux"); unlikely to be accepted; latency kills momentum.
- **Depending on human-harness:** 39-star personal repo, private format;
  we copy the pattern (one next action, off-limits list) and own the file.
