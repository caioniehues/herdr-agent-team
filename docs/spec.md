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

The generated protocol's Git contract follows the worker's `worktree` flag.
Worktree workers commit only on their configured branch, push it, and open a
PR with `gh`; they never touch the main/default branch, merge, or tag. Shared-
tree workers do not run Git: the coordinator owns Git operations, central
gates, and merges.

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
5. Launch all workers concurrently. Each worker independently:
   a. Launch agent CLI via `herdr pane run` in that workspace. **cwd is set at
      pane creation, never via a `cd` in the prompt.**
   b. Inject one launch-prompt line containing the absolute brief path and that
      worker's absolute protocol path, and submit it with one `herdr pane run`.
      When `submit_verify = true`, verify with
      `herdr agent wait --status working`; on timeout, retry once with an empty
      `pane run` and verify again.
   c. Persist the worker as `running` with launch checkpoint `brief_submitted`.
      Spawn checkpoint updates and hook reconciliation take a run-scoped
      advisory file lock around a fresh load, narrow mutation/reconciliation,
      and the existing atomic temp-file rename. Spawn threads and separate hook
      processes therefore cannot overwrite one another with stale snapshots.
      All later `run.toml` mutations, including adopt, attention, and kill,
      use this same transaction; unlocked save helpers are test-only.
6. After brief submission, poll lazily for each optional Herdr `agent_session`.
   Persist it when available; missing identity does not delay this or another
   worker's brief and does not fail an otherwise running worker.

`team spawn --resume <run-dir>` resumes an interrupted active run. Running and
adopted workers are untouched. For each spawn-owned `pending` worker, resume
reuses a live recorded pane and existing worktree, creates any absent resource,
and recreates a workspace when the recorded pane is gone. If the live pane
already reports an agent, resume skips the launcher command and continues with
brief submission; otherwise it launches the agent CLI first. Existing immutable
worker protocols are reused; missing protocols are generated before launch.
Resume advances checkpoints from `pending` to `resources_ready` to
`brief_submitted`. A run with no spawn-owned pending workers is an idempotent
no-op with a clear message.

## 5. Report flow (push, not poll)

- Manifest hooks reconcile `pane.agent_status_changed`, `pane.moved`,
  `pane.exited`, `pane.closed`, `workspace.closed`, `worktree.removed`, and
  `pane.agent_detected` against `run.toml` (dot-form manifest names; JSON uses
  underscore form). Reconciliation persists atomically: move migrates the
  public pane ID (including the god pane), exit/close orphan the worker, and
  workspace/worktree removal ends only the affected worker allocation. The run
  ends when its god allocation vanishes or no non-terminal workers remain;
  agent detection binds the optional identity.
- Hook receives `HERDR_PLUGIN_EVENT_JSON`; plugin matches the pane against
  active runs (ignores non-team panes — cheap exit).
- On a team worker flipping `blocked`, or completing as `working -> idle|done`:
  1. Append an entry to `<run>/inbox/events.jsonl` (durable).
  2. Inject **one line** into the god's pane:
     `[team <name>] <worker> is <status> — report: <abs path>` — pointer only,
     never report content (keeps god context lean).
- Workers are briefed to write their actual report to
  `<run>/inbox/<worker>.md` *before* going idle/done.
- Pointer injection is an at-most-once attention notification, not proof of
  turn completion: upstream can report background-wait panes as `idle`/`done`
  while work remains (`ogulcancelik/herdr#1217`). The durable report and its
  sentinel remain the completion truth.
- Manifest registrations are cached at link time. After any
  `herdr-plugin.toml` change, `herdr plugin disable` plus `enable` is
  insufficient: a stale reported version and missing/non-firing events or panes
  mean Herdr is still serving the cached manifest. Run
  `herdr plugin unlink caioniehues.agent-team`, then
  `herdr plugin link <absolute-plugin-path>`. Every post-release smoke test must
  relink and read the reported version back. If upstream later invalidates this
  cache automatically, this remains historical release guidance.

## 6. `team status` / `team kill`

- `status`: read `run.toml` + live `herdr agent list` — table of worker, agent
  kind, herdr status, last report time. `--json` for the god.
- `kill`: close team workspaces (`herdr workspace close`), optionally
  `--remove-worktrees` (refuses if worktree dirty — salvage rule), mark run
  ended in `run.toml`. `kill <run-dir> --worker <name>` tears down only that
  owned workspace (or releases an adopted pane), retains the run while other
  workers remain, and keeps the same dirty-worktree refusal.

## 7. Manifest surface

- `[[actions]]`: `spawn` (context: workspace), `status`, `kill`, and
  `open-board` — thin wrappers over the binary for keybinding/palette use.
- `[[events]]`: agent status change → `<binary> on-agent-status`.
- `[[panes]] board`: a durable tab control deck; callers may override placement
  to `overlay` for a quick popup. `[[link_handlers]] report` routes `report:`
  pointers to `open-report` using `HERDR_PLUGIN_CLICKED_URL`.
- A user keybind uses `type = "plugin_action"` and command
  `caioniehues.agent-team.open-board`.

## 8. Post-v1 roadmap (rewritten 2026-07-15 from the research wave)

Evidence-ordered sequence (sources: the four `docs/research/*2026-07-15*.md`
reports; decisions grilled with Caio 2026-07-15). Research evidence and
dogfooding pain both justify ordering; speculation still doesn't (ADR-0007).

**Not roadmap — shipped bug (filed as priority issue):** the hook listens
only to `pane.agent_status_changed`, so a `pane.moved` (which assigns a NEW
public pane id), `pane.exited`/`pane.closed`, `workspace.closed`, or
`worktree.removed` silently stales the run board and every later
command/event match. Lifecycle-event reconciliation ships as a fix, not a
feature.

1. **Lifecycle reconciliation** (the bug above): hook `pane.moved`
   (atomically migrate pane/tab/workspace ids from `previous_pane_id`),
   `pane.exited` vs `pane.closed` (dead vs removed), `workspace.closed`,
   `worktree.removed`, and `pane.agent_detected` (bind identity earlier).
2. **Full `agent_session` persistence** — **SHIPPED (#5, 2026-07-15)**:
   store `{source, agent, kind, value}` per worker (while retaining the
   legacy `agent_id` projection for old run boards), plus the exact
   `HERDR_SOCKET_PATH` / `HERDR_SESSION` identity per run at spawn and adopt.
   Prerequisite for any real restart; restart logic remains out of scope.
3. **Schema-gated metadata + aggregate notifications — SHIPPED (#6,
   2026-07-15)**: the runtime probe maps the team facts to the installed
   `pane report-metadata` surface: compact `team/role` in `custom_status`,
   task (when available) in `title`, and a status label. It uses a stable
   plugin `--source` and monotonic per-worker `--seq`; explicit attention
   pings use `--ttl-ms`. `herdr api schema --json` is cached per run before the
   first write: absent custom tokens fall back to `display_agent`/title.
   `notification show` is once per aggregate team-complete, blocked-duration,
   unrecoverable-exit, or explicit needs-attention condition — never per
   ordinary status flip. `HERDR_AGENT_TEAM_BLOCKED_THRESHOLD_MS` configures
   the blocked-duration policy (default: five minutes; checked on subsequent
   lifecycle/status events because this plugin has no daemon).
4. **Native board pane — SHIPPED (#7, 2026-07-15): repurposed 2026-07-15 (issue #7 comment, after
   prototype round 1): the human's CONTROL DECK for the god's team, not a
   status dashboard.** Per-worker row actions are first-class — msg worker,
   ack/answer attention, kill worker, adopt pane, open report — issued from
   the board without interrupting the god session (`[[panes]]` entrypoint
   with durable tab + popup variant, `open-board` action, `plugin_action`
   keybinds, link handler making report/task pointers hot). Informational
   core = run-scoped facts nothing else renders: optional tasks stored in the
   worker spec and run state, report links,
   mailbox state. Status rendering stays minimal: one team strip; per-pane
   status belongs to the sidebar via step-3 metadata — never a generic
   agent list. Dependencies remain deferred. Collection is CLI polling behind
   a small collector seam; #8 replaces it with socket snapshots/subscription.
5. **Herdr seam unification (#22)**: one `HerdrApi` trait in herdr.rs, one
   shared FakeHerdr, hook.rs trait-injected; launcher loading deduped.
   Behavior-neutral enabler for the socket backend and the god toolkit.
6. **God toolkit (#23, #24, #25)** — reshaped 2026-07-15 (god-lens review):
   `team wait --until any-report|report:<w>|all-reports|blocked|attention|
   all-terminal` over run state + inbox (report existence = completion truth;
   never pane attention states; CLI-polling v1 behind a collector trait);
   `inbox`/`report` verbs with read-marks and stopped-not-done triage;
   zero-ceremony invocation (self-resolved plugin dirs); `msg all`;
   god-side skill `skills/god/` shipping the coordination playbook.
7. **Direct socket backend behind `HerdrApi`** (ADR-0011): `SocketClient`
   adapter; board + `team wait` collectors swap to snapshot/subscribe with
   no interface change; CLI stays default/fallback.

   Selection is explicit: set `HERDR_TEAM_BACKEND=socket`. The adapter uses
   only the public NDJSON socket named by `HERDR_SOCKET_PATH`, validates a
   protocol-16 `ping` handshake and requires `herdr api schema --json` to match
   the checked-in schema baseline. It falls back cleanly if the path is absent,
   an I/O deadline expires, runtime schema drifts, or the handshake is
   unsupported. Mutating `HerdrApi` calls always delegate to the CLI backend.
   Frames are capped at 1 MiB; reconnects have a fixed cap, backoff, and overall
   deadline; response IDs and typed result/event shapes are validated. Board and
   aggregate wait bootstrap with `session.snapshot`, then retain one multiplexed
   `events.subscribe` connection for the durable run's worker pane IDs across
   refresh cycles. Ordinary read timeouts preserve that stream; transport loss
   drops it, re-snapshots, and reconnects. An immediate subscription failure
   spends only the remaining wait budget in bounded CLI polling. Completion
   truth remains `run.toml` and inbox state.
   Set `HERDR_TEAM_SOCKET_TRACE=<path>` for redacted JSONL diagnostics (request
   ID, method, result type, latency, and a fixed error category only; server
   text is never written).
8. **Bounded previews + conservative restart** (broadcast moved to the god
   toolkit):
   `team msg --all` loops run members with per-target results; board
   previews via bounded `recent-unwrapped` reads; `team restart` only for
   launchers with a deliberately implemented, tested `resume_command`
   (upstream has no public targeted resume — delete ours if one appears).
7. **Later/optional**: declarative layouts (`layout.export/apply`) for
   deterministic team topologies; Kitty-graphics board enrichment; run
   history browsing; opencode/gemini tested launchers; limux backend
   extraction.

**Cancelled** (native overlap, `docs/research/upstream-integration-opportunities-2026-07-15.md` §9):
- generic plugin statusline/agent list (herdr sidebar + rollups own it);
- per-worker CLI-wait fan-out for team wait;
- `pane report-metadata --custom-status` progress pings — `custom_status`
  does not exist in current upstream source; superseded by schema-gated
  tokens (step 3).

**Watch item**: an optional Claude-native visible-team compatibility mode
(Claude owns team/mailbox; herdr panes provide visibility — proven feasible
by herdr-claude-teams). Separate experiment, never the core
(`docs/research/herdr-claude-teams-analysis-2026-07-15.md` §5).

## 9. Verified facts (authority-tagged)

Every fact carries its authority per ADR-0010: `[live <date>]` = observed on
the installed herdr (decisive for behavior); `[source <date>]` = upstream
checkout (decisive for attribution/surface); `[preview]` = in upstream
source but unconfirmed on our runtime — feature-detect before use. Dense
reference detail lives in `docs/research/` (ADR-0010 §3), not here.

### Corrections from the 2026-07-15 source audit

(`docs/research/upstream-architecture-claims-2026-07-15.md` Part B)

- **Herdr core is Rust, not Zig** `[source 2026-07-15]`. Zig is the vendored
  `libghostty-vt` terminal engine behind FFI; Rust owns multiplexing, agent
  detection, plugins, CLI, IPC.
- **`pane run` is one API request carrying text + `keys:["Enter"]`; herdr
  has no paste-debounce** `[source 2026-07-15]`. The historical "double-Enter
  after `agent send`" observation `[live 2026-07-14]` is agent-TUI behavior
  (the TUI swallowing an immediately following Enter), not a herdr timer.
  Behavior rule unchanged: always `pane run`, never split
  send-text/send-keys (ADR-0006).
- **Mid-turn queueing lives in the agent TUIs, not herdr**
  `[source 2026-07-15]`. Herdr writes bytes to the PTY immediately; Claude
  Code and Codex queue the injected turn themselves `[live 2026-07-14/15]`.
  `queues_midturn` therefore stays a *launcher* property — exactly where the
  launcher table put it.
- **`custom_status` does not exist in current upstream** `[source
  2026-07-15]`; our protocol-16 snapshot predates its removal. Current
  source `PaneInfo` exposes metadata `tokens` instead `[preview]`. Roadmap
  step 3 is schema-gated on this.
- **Event payload may add optional fields** `[source 2026-07-15]`: `agent`
  is omitted when none; `title`, `display_agent`, `state_labels` may appear.
  Parsers must tolerate unknown/absent optional fields.
- **Lifecycle reconciliation wins over waits** `[source 2026-07-15]`:
  `pane.moved` carries `previous_pane_id` and a replacement `PaneInfo`;
  `pane.exited`/`pane.closed`, `workspace.closed`, and `worktree.removed` are
  hookable lifecycle truth. Do not trust a hanging status wait when a pane
  vanishes (`ogulcancelik/herdr#1439`).
- **`done` is an attention state** `[source 2026-07-15]`: derived from
  internal `Idle` when the pane is unseen; the detector knows only
  idle/working/blocked/unknown. Explains why `agent wait` rejects `done`
  while `wait agent-status` accepts it. Status enum
  idle/working/blocked/done/unknown confirmed exhaustive.
- **Env injection**: all managed panes also get `HERDR_SOCKET_PATH` (plus
  `TERM`/`COLORTERM`) `[source 2026-07-15]` — previously undocumented here.
  Event hooks additionally get the `HERDR_PLUGIN_*` set; full matrix in the
  research report Part A §2.
- **Run identity persistence**: named sessions are selected with
  `HERDR_SESSION`, and `HERDR_SOCKET_PATH` is the low-level override
  `[source 2026-07-15]`. Each new or adopted run persists both values when
  present, alongside the complete upstream `agent_session` reference per
  worker; older run boards without these additive fields still deserialize.
- **Plugin surface**: 21 hookable manifest events (not just
  `pane.agent_status_changed`); actions, panes, keybinds, link handlers
  available `[source 2026-07-15]`. High-frequency kinds
  (`pane.output_changed`, `layout.updated`, …) are deliberately not
  plugin-hookable — direct subscription territory (ADR-0011).
- **Metadata surface and coexistence**: `pane report-metadata` on installed
  herdr 0.7.3 accepts `--title`, `--display-agent`, `--custom-status`,
  `--state-label`, `--seq`, and `--ttl-ms` `[live 2026-07-15]`. A
  plugin-scoped `--source caioniehues:herdr-agent-team` write on an active
  Codex pane preserved its native `agent`/`agent_status` while adding only
  presentation metadata `[live 2026-07-15]`; the plugin never calls
  `pane report-agent`. The tested `--applies-to-source herdr:codex` filter
  produced no visible update despite that pane's `agent_session.source`, so
  the plugin deliberately omits that restrictive filter and relies on its
  own stable source id. Schema absence still selects the fallback path
  `[preview gate]`.

### Build-time verification TODOs (resolved)

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
- [x] ~~Live-verify `pane report-metadata --custom-status`~~ — CANCELLED
      2026-07-15: `custom_status` is gone from current upstream source
      (`[source]`, corrections above). Superseded by the schema-gated
      metadata-token plan (§8 step 3): probe `herdr api schema --json` for
      token support at runtime; the coexistence question (plugin `--source`
      vs agent integration authority) transfers to that verification.

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
herdr-agent-team msg <target> <text> [--attention] [--run <run-dir>]
```

- `<target>`: `god` or a worker name from the active run. Resolution: name →
  pane id via `run.toml`. Ambiguity or unknown name = hard error listing
  candidates (never guess — marketplace pattern #2).
- God-side fan-out accepts `all` or a comma-separated worker list. Every
  distinct worker uses the same readiness gate and outbox discipline.
  `--attention` remains valid only for the singular `god` target.
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
- `--attention` is valid only for a worker-to-`god` message: the worker's
  `HERDR_PANE_ID` identifies the run member, the hook metadata records one
  pending transient metadata ping, and `notification show` is emitted only
  once for that worker. This is the explicit attention channel; agent status
  values remain Herdr's fixed enum.

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
  several active runs without `--run` = hard error listing candidates. `--team`
  names a new ad-hoc team: it is a hard error when any run is active (pass
  `--run <dir>` or kill the active run instead), and it cannot be combined with
  `--run`. No active run → bootstrap an ad-hoc star run (name from `--team`,
  default `adhoc`; god = current pane; cwd = adopted pane's cwd; reconstructed
  minimal spec lives only in `run.toml`).
- **Topology:** star-only. Adopting into a mesh run is a hard error
  (immutable peer tables would go stale — ADR-0009 defers the amendment
  mechanism).
- **Git:** adopted workers are shared-tree workers (`worktree = false`), so
  their generated protocol keeps the no-Git rule; the coordinator owns Git
  operations for the adopted pane's working tree.
- **Agent kind:** from the pane's detected agent label, mapped into the
  launcher table. Unknown label → conservative synthetic policy
  (`submit_verify = true`, `queues_midturn = false`) + warning naming the
  `agents.toml` entry to add. No detected agent → refuse.
- **Brief:** `--brief` injects brief + protocol pointers in one line
  (launch-prompt style); otherwise protocol pointer only.
- **Crash recovery:** rerunning `adopt` with the same worker name and pane
  completes a persisted `pending` adoptee through protocol generation and
  prompt submission, then atomically records launch checkpoint
  `brief_submitted` and marks it `running`. If that checkpoint was already
  persisted, recovery completes the pending lifecycle without resubmitting the
  prompt. It never runs a launcher command in the existing pane. Rerunning it
  for an already-running adoptee is an idempotent no-op with a clear message;
  other duplicate names remain errors.
- **Kill semantics:** `team kill` closes only plugin-created workspaces.
  Adopted workers are marked `released` in `run.toml` and receive one
  injected release notice; their panes and workspaces survive.

## 13. God CLI ergonomics — durable wait and inbox (issues #23, #24)

These verbs read the run-board and inbox files only. They never infer
completion from Herdr pane attention/done presentation, avoiding the
done-versus-idle trap and upstream #1439's closed-pane subscription hang.

```
herdr-agent-team wait [--run <dir>] --until any-report|report:<worker>|all-reports|blocked|attention|all-terminal [--timeout <seconds>] [--json]
herdr-agent-team inbox [--run <dir>] [--unread] [--json]
herdr-agent-team report <worker> [--run <dir>] [--head N]
```

- `wait` polls through `GodCollector`, a small snapshot seam intended for the
  future socket backend. Default timeout is 300 seconds and is always bounded.
  Reached exits 0, timeout exits 2, and an orphaned/failed required worker
  without its report exits 3. A run that becomes inactive during the wait
  returns the distinct `inactive_run` verdict and exits 4; an explicitly
  selected inactive run is rejected before polling. Usage, resolution, and I/O
  errors exit 1. `--json` emits one stable single-line verdict.
- Report-file existence is completion truth. `blocked` and `attention` come
  from durable hook metadata; `all-terminal` comes from worker lifecycle.
  `all-terminal` is literal: failed and orphaned workers count as terminal and
  the condition exits 0. For `blocked`/`attention`, an all-terminal team that
  cannot satisfy the condition returns the dead-worker verdict instead.
- `inbox` emits one worker row with report presence/mtime, attention, read
  state, and `STOPPED-NOT-DONE` when an idle/done pointer state has no report.
  `--unread` retains missing reports and reports newer than their read mark.
- `report` prints the absolute report path, or at most `N` lines with
  `--head`, then transactionally persists the report mtime as its read mark in
  `run.toml` so the mark survives process and agent-context restarts.
  Printing the durable path is an intentional pointer handoff and counts as
  read even when `--head` is omitted.
- When `HERDR_PLUGIN_STATE_DIR` / `HERDR_PLUGIN_CONFIG_DIR` are absent, the
  executable derives Herdr's stable XDG/home plugin layout using its manifest
  id `caioniehues.agent-team`. Explicit environment always wins.
  This fallback targets release Herdr's `herdr/` app directory; debug Herdr's
  `herdr-dev/` layout requires explicit injected environment.
