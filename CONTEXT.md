# CONTEXT.md — domain glossary

Ubiquitous language for herdr-agent-team. One meaning per word; challenge
drift here first.

- **Team** — a named set of workers spawned together from one spec, plus their
  run state. One team ↔ one run dir.
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
- **Completion sentinel** — the final non-empty line
  `HERDR_TEAM_WORKER_COMPLETE` in a Report, written only after its content is
  complete.
- **Result ready** — a Report is ready for `wait report:<worker>` only when
  it carries the completion sentinel as its final non-empty line; file
  existence alone is not readiness.
- **Pointer injection** — the delivery mechanism: one line typed into a pane
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
  Resolves name → pane, delivers via `pane run`, verifies submission,
  readiness-gates per launcher policy (ADR-0008).
- **Outbox** — `<run>/outbox/<target>/` queue of pending messages for a
  target that can't safely receive mid-turn; drained in order by the status
  hook when the target flips idle/done. Counterpart of the inbox.
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
