# HANDOFF — next session orientation

Scaffold committed 2026-07-14 (design locked same day via grilling interview in
the limux repo). You are picking up a **pre-v1 scaffold**: docs are the
contract, binary is stubs.

## Read first

1. `docs/spec.md` — buildable v1 spec. §10 is the definition of done.
2. `docs/adr/0001–0007` — every locked decision + why. Don't relitigate
   silently; new evidence → new ADR.
3. `CONTEXT.md` — vocabulary (god, worker, star/mesh, pointer injection,
   run-board, launcher table).

## State

- `cargo build --release` compiles; every subcommand is an explicit todo
  pointing at its spec section.
- Local git only — **NOT on GitHub yet.** Publishing = create public repo
  `caioniehues/herdr-agent-team` + topic `herdr-plugin` (marketplace auto-lists
  in ~30 min). Ask Caio before pushing.
- Source logic to port lives in the limux fork:
  `~/Projects/cmux-kde/limux/rust/limux-cli/src/main.rs` — `build_agents_md`,
  `agent_launch_command` (copy, don't depend — ADR-0005).

## NEXT steps (in order)

1. **Verify the four spec §9 TODOs first** — cheapest de-risking:
   - manifest `[[events]] on =` name for agent status changes (docs example
     vocabulary vs socket's `pane.agent_status_changed`) — test with a linked
     dummy plugin that logs `HERDR_PLUGIN_EVENT` env;
   - event JSON payload shape;
   - pointer-line injection into a mid-turn Claude Code pane queues cleanly;
   - codex double-Enter under `pane run` vs `agent send`.
2. Implement `spawn` happy path (spec §4) against a throwaway 2-worker spec.
3. Event hook `on-agent-status` (spec §5).
4. `status` / `kill` (spec §6).
5. Live DoD run on the limux repo (spec §10), then talk to Caio about
   publishing.

## Context that doesn't fit the docs

- A survey of all 175 marketplace plugins ran on 2026-07-14 (workflow in the
  limux Claude session) — its report should list overlap plugins and patterns
  to steal; check with Caio if not already applied here.
- Caio plans to run coordinator (god) sessions inside herdr from now on —
  which this plugin's ADR-0002 report path requires anyway.
- Herdr is closed-source. Compatibility contract: snapshot `herdr api schema
  --json` into the repo and diff on herdr updates (not yet done — worth adding
  as a small script + CI-less check).
