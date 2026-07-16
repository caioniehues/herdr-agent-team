# CONTEXT.md — domain glossary

Ubiquitous language for **herdmates** (formerly herdr-agent-team). One
meaning per word; challenge drift here first. Pivot vocabulary first;
legacy orchestration terms (still accurate for the v1.x surface) below.

## Herdmates (pivot vocabulary, ADR-0012)

- **Herdmates** — this plugin: Claude Code teammates, native in herdr.
  Three surfaces: shim, agent board, focus pane.
- **Native team** — a Claude Code agent-teams session (lead + teammates)
  as shipped by Anthropic: spawn, mailboxes, lifecycle all upstream. We
  never re-implement it; we host and observe it.
- **Lead** — the Claude Code session that spawns and coordinates a native
  team (Anthropic's term). Replaces "god" in pivot vocabulary.
- **Teammate** — a full independent Claude Code session spawned into a
  native team by the lead. In split-pane mode it occupies its own pane and
  is directly steerable.
- **Teammate mode** — Claude Code's display setting (`teammateMode`):
  `in-process` (rendered inside the lead's terminal) or split-pane
  (tmux/iTerm2 today). The shim's goal: make herdr a working split-pane
  host.
- **Shim** (mechanism name: **teammux**) — a fake `tmux` executable +
  `TMUX` env inside herdr panes that translates Claude Code's tmux
  invocations into `herdr pane` CLI calls, so teammates land as real herdr
  panes.
- **Recon spike** — the shim's go/no-go gate: log every tmux invocation a
  real team session makes (logging wrapper in real tmux), produce the verb
  inventory + verb→herdr mapping. Kill signals: control mode (`tmux -C`)
  or unmappable verbs.
- **Team files** — the documented on-disk state of a native team:
  `~/.claude/teams/{team}/config.json` (members, session IDs, pane IDs)
  and `inboxes/{agent}.json` (mailboxes). The boards' data source.
- **Agent board** — the surface answering "what are the agents doing?".
  Two forms: **sidebar-token board** (D1) — teammate state pumped into
  herdr sidebar rows via `pane report-metadata` tokens; **TUI board** (D2)
  — an interactive terminal UI in a zoomed/overlay plugin pane.
- **Focus pane** — the surface answering "what should the human be
  doing?": current task, the single next action, decision queue. Renders
  the focus file; ADHD-harness pattern (one thing at a time).
- **Focus file** — plain-file contract at
  `~/.local/share/herdmates/focus.md`. Anything may write it (human,
  agent, atomizer skill); the focus pane only renders it.
- **Atomizer** — companion skill that breaks a task dump into the single
  next concrete action and writes the focus file (pattern copied from
  human-harness, not depended on).
- **Plugin pane** — herdr surface: a plugin-declared TUI process in a
  herdr-managed pane (`plugin.pane.open`; placements overlay/split/tab/
  zoomed). Real pane, full pane APIs.
- **Popup pane** — herdr surface: session-modal floating pane, no pane id,
  invisible to pane/agent APIs, swallows all input, dies with its command.
  Quick-glance only; never the board's home.
- **Sidebar token** — named display value (`--token name=value`, rendered
  as `$name` in `[ui.sidebar.agents] rows`) attached to a pane via
  `pane report-metadata`. Display-only; never semantic state.

## Legacy orchestration vocabulary (v1.x surface, frozen at v1.1.0)

- **Team** — a named set of workers participating in one run, initially
  spawned from a spec or later adopted, plus their run state. One team ↔ one
  run dir.
- **Worker** — a single coding-agent CLI (claude, codex, …) running in its own
  Herdr workspace as part of a team. Identified by unique worker `name`.
- **God (agent)** — the user's main interactive agent session that coordinates
  the team: spawns, briefs, receives reports, decides. Exactly one per team;
  the plugin never spawns a god. (Term borrowed from herdr-orchestrate.)
- **Topology** — who may talk to whom. **Star**: workers ↔ god only. **Mesh**:
  workers also message each other peer-to-peer. Per-team flag; star is default.
- **Brief** — a per-worker instruction file the worker reads at launch.
  Delivered as a one-line pointer injection, never inline text.
- **Report** — a worker's durable output file at `<run>/inbox/<worker>.md`.
  Written by the worker before it goes idle/done.
- **Result ready** — a worker outcome whose report is finalized and safe for
  the god to consume. Mere report-path existence is not sufficient.
- **Completion sentinel** — a worker-emitted attention signal that follows a
  result becoming ready. It is not durable completion truth by itself.
- **Pointer injection** — the submission mechanism: one line typed into a pane
  naming durable file paths. Payload stays on disk; context stays lean.
- **Inbox** — the run dir's `inbox/` directory: report files + `events.jsonl`.
- **Run-board** — the durable record of a team run (`run.toml` + worker
  protocols + inbox): who was spawned, where, current lifecycle state.
- **Launcher table** — data-driven config mapping agent kind → launch argv,
  submission-verification policy, repository-authored AGENTS.md capability,
  and mid-turn queueability (`queues_midturn`). Adding an agent = adding a
  table entry.
- **Msg verb** — the plugin subcommand (`herdr-agent-team msg <target>
  <text>`) that is the only messaging channel workers are ever briefed on.
  Resolves name → pane, submits via `pane run`, verifies submission,
  readiness-gates per launcher policy (ADR-0008).
- **Outbox** — `<run>/outbox/<target>/` queue of pending messages for a
  target that can't safely receive mid-turn; drained in order by the status
  hook when the target flips idle/done. Counterpart of the inbox.
- **Queued** — an instruction is durably retained in the outbox but has not
  been submitted to its target.
- **Submitted** — Herdr has accepted the request to place an instruction into
  the target pane. This does not prove the worker read or acted on it.
- **Acknowledged** — the target worker has produced explicit evidence that it
  received an instruction.
- **Message lifecycle** — **Queued / Submitted / Acknowledged**. **Queued**:
  the message sits in the outbox awaiting drain (`MessageOutcome::Enqueued`).
  **Submitted**: the text was typed into the target pane's input and
  submission was verified per launcher policy — this is what the code and the
  durable audit event call `delivered` (**Delivered → Submitted**; the word
  is kept in `MessageOutcome::Delivered` and `events.jsonl` for
  compatibility).
- **Attention lifecycle** — the owned raise/observe/clear cycle of a worker's
  explicit attention request. Raised by the worker (`msg god <text>
  --attention`), persisted in durable run state (`attention_pending`),
  observable on the inbox/board and every metadata publish, and cleared only
  by an explicit god-side ack (`msg <worker> <text> --ack`). Status flips
  never consume it.
- **Queues mid-turn** — launcher-table property: whether a mid-turn `pane
  run` into that agent's TUI queues as a pending user message (claude:
  verified true; codex: verified true) or risks interrupting the turn.
- **Setup command** — team-spec command run inside each fresh worktree before
  the worker launches (project preflight: symlinks, deps, skip-worktree).
- **Worker protocol** — one immutable generated file per worker at
  `<run>/protocols/<worker>.md`: identity, report protocol, and (mesh only) the
  peer table + message envelope. It is passed by absolute-path pointer and is
  distinct from the repository's authored `AGENTS.md`, which remains untouched.
- **Status flip** — a Herdr agent-status transition (idle/working/blocked/
  done/unknown). Flips to `blocked`/`done` trigger the report flow.
- **Evidence hierarchy / authority tags** — claim authority labels: `live`
  (current observed behavior), `source` (local upstream source), and `preview`
  (runtime schema probe required), per ADR-0010.
- **Adopted worker** — an existing pane registered into a run as a full
  worker (`team adopt`, ADR-0009): protocol generated at adoption,
  `adopted = true` in run state. Kill releases it (notice injected)
  instead of closing its pane.
- **Released** — terminal lifecycle of an adopted worker after `team
  kill`: no longer a team member, pane untouched, report protocol void.
