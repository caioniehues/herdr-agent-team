# herdr-agent-team — v1 specification

Distilled from the design interview on 2026-07-14 (grilling session in the limux
repo). Decisions are recorded as ADRs in [adr/](adr/); this document is the
buildable behavior spec.

## 1. Overview

A Rust binary packaged as a Herdr plugin. It orchestrates a **team** of coding
agents (Claude Code, Codex, extensible via config) inside Herdr workspaces,
coordinated by a **god agent** — the user's main interactive agent session.

The plugin has two halves:

1. **CLI half** (invoked by the god or the human): `team spawn`, `team status`,
   `team kill`.
2. **Event half** (invoked by Herdr): a manifest `[[events]]` hook that fires on
   agent status transitions and delivers reports to the god.

There is no daemon. Durable state lives in files under
`$HERDR_PLUGIN_STATE_DIR`.

## 2. Team spec file — `herdr-team.toml`

Lives in the project repo (versionable). The `--agents` CLI shorthand generates
a throwaway spec with defaults.

```toml
# herdr-team.toml
name = "limux-wave3"
topology = "star"            # "star" (default) | "mesh"
cwd = "."                    # team root; worktrees are created relative to the repo here

# Optional: run in each freshly created worktree before the agent launches.
# Encodes project-specific worktree preflight (symlinks, skip-worktree, deps).
setup = ["./scripts/worktree-setup.sh"]

[god]
# How to reach the god session. v1: the pane the spawn command runs from,
# overridable with an explicit herdr agent/pane target.
target = "self"              # "self" | explicit herdr agent name / pane id

[[workers]]
name = "builder-1"
agent = "claude"             # key into the launcher table
role = "builder"             # free text, recorded in the worker protocol
worktree = true              # default: true for role=builder, false otherwise
branch = "feat/wave3-builder-1"   # worktree branch (required when worktree=true)
brief = "briefs/builder-1.md"     # path to brief file, injected at launch

[[workers]]
name = "reviewer-1"
agent = "codex"
role = "reviewer"
worktree = false
brief = "briefs/reviewer-1.md"
```

Validation: unique worker names; `agent` must exist in the launcher table;
`branch` required iff `worktree = true`.

## 3. Launcher table (data-driven agent roster)

Lives in `$HERDR_PLUGIN_CONFIG_DIR/agents.toml`. Ships with tested entries for
`claude` and `codex`; users add agents by config, not code.

```toml
[claude]
command = ["claude"]                 # argv, launched via herdr pane run
submit_verify = true                 # verify via `herdr agent wait --status working`
reads_agents_md = "pointer"          # needs a pointer line in the launch prompt
queues_midturn = true                # mid-turn pane run queues cleanly (live-verified)

[codex]
command = ["codex"]
submit_verify = true
reads_agents_md = "native"           # codex reads AGENTS.md from cwd natively
queues_midturn = true                # mid-turn pane run queues cleanly (live-verified, §9)
```

`reads_agents_md` describes how the agent consumes the repository's authored
`AGENTS.md`; it does not control the generated worker protocol. Every worker
receives an explicit absolute-path pointer to its own protocol.

`queues_midturn` records whether a mid-turn `pane run` into this agent's TUI
queues safely as a pending user message (verified for claude, spec §9).
`false`/absent means the `msg` verb (§11) must not deliver while the target is
`working` — it enqueues to the outbox instead. Conservative default: `false`.

Submission keys are not configurable. `herdr pane run` injects and submits the
pane-targeted prompt in one operation for every launcher.
When `submit_verify = true`, the plugin waits for agent status `working`; if
that verification times out, it performs one empty `pane run` to submit the
existing composer without duplicating the prompt, then verifies again. The
plugin never uses split send-text/send-keys submission.

## 4. `team spawn` behavior

Given a spec (file or shorthand):

1. **Preflight**: validate spec; check each worker's agent CLI exists on PATH;
   check `herdr` reachable (`HERDR_BIN_PATH`).
2. **Run dir**: create `$HERDR_PLUGIN_STATE_DIR/runs/<team>-<timestamp>/` with
   `run.toml` (resolved spec + live state), `protocols/`, and `inbox/`.
3. **Workspace allocation**: prepare every worker's cwd, create every Herdr
   workspace, and record all returned workspace/pane IDs in `run.toml`. No agent
   CLI launches until allocation completes for the whole team.
   - If `worktree = true`: `herdr worktree create` (branch from spec), then run
     the team `setup` command inside it.
   - `herdr workspace create --cwd <dir> --label <worker-name>`.
4. **Worker protocols**: after all workspace IDs are allocated, but before any
   agent launch, create exactly one immutable generated file per worker at
   `<run>/protocols/<worker>.md`:
   - **star**: identity, report protocol (write to
     `<run>/inbox/<worker>.md`, then print the completion sentinel), and how to
     reach the god.
   - **mesh**: all star content plus the peer table and message envelope.
   Repository-authored `AGENTS.md` files remain untouched and in effect.
5. Per worker:
   a. Launch agent CLI via `herdr pane run` in that workspace. **cwd is set at
      pane creation, never via a `cd` in the prompt.**
   b. Inject one launch-prompt line containing the absolute brief path and that
      worker's absolute protocol path, and submit it with one `herdr pane run`.
      When `submit_verify = true`, verify with
      `herdr agent wait --status working`; on timeout, retry once with an empty
      `pane run` and verify again.
6. Record every worker's herdr agent id/name in `run.toml`.

## 5. Report flow (push, not poll)

- Manifest event hook on agent status change (socket event
  `pane.agent_status_changed`; exact manifest `on =` name to be verified against
  the herdr docs during build — see spec TODOs).
- Hook receives `HERDR_PLUGIN_EVENT_JSON`; plugin matches the pane against
  active runs (ignores non-team panes — cheap exit).
- On a team worker flipping `blocked` or `done`:
  1. Append an entry to `<run>/inbox/events.jsonl` (durable).
  2. Inject **one line** into the god's pane:
     `[team <name>] <worker> is <status> — report: <abs path>` — pointer only,
     never report content (keeps god context lean).
- Workers are briefed to write their actual report to
  `<run>/inbox/<worker>.md` *before* going idle/done.

## 6. `team status` / `team kill`

- `status`: read `run.toml` + live `herdr agent list` — table of worker, agent
  kind, herdr status, last report time. `--json` for the god.
- `kill`: close team workspaces (`herdr workspace close`), optionally
  `--remove-worktrees` (refuses if worktree dirty — salvage rule), mark run
  ended in `run.toml`.

## 7. Manifest surface (v1)

- `[[actions]]`: `spawn` (context: workspace), `status`, `kill` — thin wrappers
  over the binary for keybinding/palette use. The god calls the binary directly.
- `[[events]]`: agent status change → `<binary> on-agent-status`.
- No `[[panes]]` in v1 (dashboard is v1.1+), no link handlers.

## 8. Out of scope for v1 (roadmap)

- Dashboard pane (ratatui, overlay placement).
- `team restart` / reassign work — herdr tracks `agent_session_id` /
  `agent_session_path` per pane (`pane report-agent` flags), so restart can be
  real reattachment (`claude --resume <session_id>` via a launcher-table
  `resume_command`), not respawn.
- Run history browsing.
- opencode/gemini tested launchers (config entries welcome, untested).
- limux backend (extract shared generator crate only when that becomes real).
- Shared task-board files under the run dir (native-teammate TaskList parity:
  claimable tickets + blocked-by edges), if dogfooding demands it.
- `team wait [--worker <name>] [--until blocked|done|report]` — blocking
  god-side wait wrapping `herdr wait agent-status` + the completion
  sentinel, so a god outside the event hook never hand-rolls poll monitors
  (motivated by the 2026-07-15 silent-monitor incident during wave 2.5).
- Worker progress pings via `pane report-metadata --custom-status` (pending
  the §9 coexistence verification).

## 9. Build-time verification TODOs

- [x] Confirm the manifest `[[events]] on =` vocabulary for agent status
      transitions — RESOLVED 2026-07-14 by marketplace survey: shipped plugins
      `cobanov/herdr-ntfysh` and `horn553/herdr-ntfy` both hook
      `on = "pane.agent_status_changed"` in their manifests. Steal their
      payload handling as reference when implementing.
- [x] Confirm `HERDR_PLUGIN_EVENT_JSON` payload shape — RESOLVED 2026-07-14 by
      live test (herdr 0.7.3, protocol 16) with the linked fixture plugin
      `tests/fixtures/event-logger-plugin/`:

      ```json
      HERDR_PLUGIN_EVENT=pane.agent_status_changed
      HERDR_PLUGIN_EVENT_JSON={"event":"pane_agent_status_changed","data":{"type":"pane_agent_status_changed","pane_id":"wG:p2","workspace_id":"wG","agent_status":"idle","agent":"claude"}}
      ```

      Note the naming split: `HERDR_PLUGIN_EVENT` uses the dot form (manifest
      vocabulary); the JSON `event`/`data.type` use the underscore form (socket
      `EventKind`). `data` matches the socket schema's
      `pane_agent_status_changed` payload; nullable fields (`custom_status`,
      `display_agent`, `title`, `state_labels`) are omitted when null. Bonus:
      `HERDR_PLUGIN_CONTEXT_JSON` carries workspace/tab/focused-pane context,
      and `HERDR_PANE_ID`/`HERDR_WORKSPACE_ID`/`HERDR_TAB_ID` are set to the
      event's pane.
- [x] Live-verify inject-into-claude-pane mid-turn — RESOLVED 2026-07-14:
      `herdr pane run <pane> "<pointer line>"` into a working Claude Code pane
      lands as a queued message ("Press up to edit queued messages"), then
      auto-submits as a normal user turn when the current turn ends. No lost
      input, no interleaving into the active turn.
- [x] Live-verify Codex submission — RESOLVED 2026-07-14 (codex TUI,
      gpt-5.6-sol): one `pane run` submits reliably. The plugin never uses split
      send-text/send-keys submission. Keep
      `herdr agent wait --status working` as the submission check; on timeout,
      issue one empty `pane run` and verify again (ADR-0006).
- [x] Live-verify codex **mid-turn** `pane run` — RESOLVED 2026-07-15 (codex
      TUI, gpt-5.6-sol, herdr 0.7.3): injected a second `pane run` 2 s into a
      working turn; the active turn completed intact, the injected line
      landed as a separate queued follow-up turn and was answered normally.
      Codex queues mid-turn like Claude Code. Shipped codex entry flipped to
      `queues_midturn = true` (third-party messenger warning was
      over-conservative for current codex). Outbox path (§11) remains for
      launcher entries that declare `false`.
- [ ] Live-verify `pane report-metadata --custom-status` from a worker pane:
      does a plugin-sourced `--source` coexist with the claude/codex
      integration's own agent reporting, or fight it? If clean, workers can
      surface progress pings ("cluster 3/7") in herdr chrome and `team
      status` (roadmap §8, not v1).

## 10. Definition of done (v1)

Spawn a real 2-worker team (claude builder in a worktree + codex reviewer,
star topology) on the limux repo; both receive briefs and start; a completed
worker's status flip lands a pointer line in the god pane within seconds; the
report file exists at the pointer path; a `msg` round-trip works (god →
worker, worker → god reply, both submitted — not sitting in a composer);
`team kill` tears down cleanly and preserves the dirty worktree.

## 11. Worker messaging — `msg` verb + outbox (added 2026-07-15, ADR-0008)

Background: the original generated protocols briefed workers to reply via
`herdr agent send`, which writes literal text **without submitting** (herdr
help; ADR-0006 verification). Defect found 2026-07-15 — as briefed, worker
replies and mesh messages never submit. Fix: workers are never briefed on raw
herdr primitives; they get one plugin verb.

### `msg` subcommand

```
herdr-agent-team msg <target> <text> [--run <run-dir>]
```

- `<target>`: `god` or a worker name from the active run. Resolution: name →
  pane id via `run.toml`. Ambiguity or unknown name = hard error listing
  candidates (never guess — marketplace pattern #2).
- Delivery: one `herdr pane run <pane_id> <text>`; submission verified per
  launcher policy (`herdr agent wait --status working`, one empty `pane run`
  retry on timeout — ADR-0006 discipline).
- Readiness gate: if the target's launcher entry has `queues_midturn = true`,
  deliver immediately regardless of status. Otherwise deliver immediately only
  when the target's agent status is `idle`/`done`/`unknown`; if `working`,
  write the message to the outbox and return 0 immediately (sender never
  blocks — deliberately unlike herdr-agent-messenger's 3 s × 300 s
  sender-side poll).
- Text is treated as opaque payload; the mesh `<agent-msg>` envelope
  (ADR-0003) travels inside it. Long/durable content goes in a file and the
  message carries the pointer — same rule as everywhere else.

### Outbox + hook drain

- Queue location: `<run>/outbox/<target>/<seq>.msg` (zero-padded sequence,
  content = exact text to deliver).
- The `pane.agent_status_changed` hook (§5), on any team member flipping to
  `idle` or `done`, drains that member's outbox in sequence order: deliver via
  `pane run`, verify, delete the file, append a `delivered` entry to
  `inbox/events.jsonl`. Drain happens before report-pointer injection logic.
- Failed delivery leaves the file in place (retried on the next flip) and
  logs a `delivery_failed` event.
- No daemon; the hook is the only drain trigger. Worst case latency = time to
  the target's next status flip, which is exactly when it can read the
  message anyway.

### Protocol briefing

Generated worker protocols (star and mesh) carry a **self-contained
invocation** — shell-quoted absolute binary path (resolved via
`current_exe` at spawn time) plus an explicit `--run <run-dir>` — so a
bare worker pane needs no PATH or env provision (live-verified deviation,
DoD run 2):

- `'<abs-binary>' msg god "<text>" --run '<run-dir>'` — reply / escalate.
- mesh: same form with `msg <peer>` and the `<agent-msg>` envelope.

The peer table lists names only; pane ids stay in `run.toml`.

**Sandbox caveat (live-verified 2026-07-15):** codex's default sandbox
denies herdr socket access (`Operation not permitted`), so a plain-codex
worker can only run `msg` behind an interactive approval. Teams relying on
codex worker→god messaging should configure a permissive launcher entry
(see `examples/agents.toml`); the shipped default stays sandboxed.

## 12. `team adopt` — existing panes as workers (added 2026-07-15, ADR-0009)

```
herdr-agent-team adopt <pane-id> --name <worker> [--role <text>]
                 [--brief <path>] [--run <run-dir>] [--team <name>]
```

- **Membership:** full worker. Generates the worker's immutable protocol at
  adoption time (identity, report path, sentinel, self-contained `msg`
  invocation), pointer-injects it (one `pane run`, submit-verified per
  launcher policy), records the worker in `run.toml` with `adopted = true`.
  Hook push, `msg`, `status`, inbox — identical to spawned workers.
  Immutability invariant: **immutable since generation**.
- **Run targeting:** newest active run by default; `--run` explicit;
  several active runs without `--run` = hard error listing candidates. No
  active run → bootstrap an ad-hoc star run (name from `--team`, default
  `adhoc`; god = current pane; cwd = adopted pane's cwd; reconstructed
  minimal spec lives only in `run.toml`).
- **Topology:** star-only. Adopting into a mesh run is a hard error
  (immutable peer tables would go stale — ADR-0009 defers the amendment
  mechanism).
- **Agent kind:** from the pane's detected agent label, mapped into the
  launcher table. Unknown label → conservative synthetic policy
  (`submit_verify = true`, `queues_midturn = false`) + warning naming the
  `agents.toml` entry to add. No detected agent → refuse.
- **Brief:** `--brief` injects brief + protocol pointers in one line
  (launch-prompt style); otherwise protocol pointer only.
- **Kill semantics:** `team kill` closes only plugin-created workspaces.
  Adopted workers are marked `released` in `run.toml` and receive one
  injected release notice; their panes and workspaces survive.
