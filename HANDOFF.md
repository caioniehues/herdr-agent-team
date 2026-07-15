# HANDOFF — next session orientation

Design locked 2026-07-14 via grilling interview in the limux repo. The scaffold
and core wave are committed; docs remain the contract.

## Read first

1. `docs/spec.md` — buildable v1 spec. §10 is the definition of done.
2. `docs/adr/0001–0007` — every locked decision + why. Don't relitigate
   silently; new evidence → new ADR.
3. `CONTEXT.md` — vocabulary (god, worker, star/mesh, pointer injection,
   run-board, launcher table).

## State

- Wave 0 scaffold: `0d75e23`. Wave 1 core (tickets 02-06): `a6e0ff9`.
- 2026-07-15: docs+ADR-0008 `2cd4e31`; waves 2+2.5 `15cd171` (spawn, hook,
  status/kill, `msg` verb, `queues_midturn`, msg-only protocols); ticket 15
  outbox drain `9e2f613`; ticket 10 worktree workers `20b8633`.
- Central gate green: build, fmt, Clippy `-D warnings`, 82 tests. Release
  binary pre-built. Manifest carries spawn/status/kill actions + the event
  hook. **Everything before the DoD is code-complete; only the human-watched
  DoD run (spec §10) remains.**
- Local git only — **NOT on GitHub yet.** Publishing = create public repo
  `caioniehues/herdr-agent-team` + topic `herdr-plugin` (marketplace auto-lists
  in ~30 min). Ask Caio before pushing.
- Source logic to port lives in the limux fork:
  `~/Projects/cmux-kde/limux/rust/limux-cli/src/main.rs` — `build_agents_md`,
  `agent_launch_command` (copy, don't depend — ADR-0005).

## NEXT steps (in order)

1. ~~Verify the four spec §9 TODOs~~ — **ALL RESOLVED 2026-07-14** by live test
   inside herdr 0.7.3 (protocol 16, matches snapshot). Findings + exact payload
   recorded in spec §9. Test fixture: `tests/fixtures/event-logger-plugin/`
   (linked but disabled; re-enable with
   `herdr plugin enable herdr-agent-team.event-logger`). Headlines:
   - `HERDR_PLUGIN_EVENT_JSON` = `{"event":"pane_agent_status_changed","data":{…socket payload…}}`;
     dot form in `HERDR_PLUGIN_EVENT`, underscore form inside the JSON.
   - Mid-turn `pane run` into Claude Code queues cleanly, auto-submits after
     the turn.
   - Codex: `pane run` submits in one call; double-Enter only needed for
     `agent send` + immediate `send-keys Enter` (debounce). Rule: always
     `pane run`.
2. ~~Wave 2 (tickets 07-09)~~ — **LANDED 2026-07-15** in `15cd171` (spawn
   happy path, event hook, status/kill + client-mismatch remediation).
3. ~~Ticket 10 worktree workers~~ — **LANDED 2026-07-15** in `20b8633`
   (worktree create before allocation, setup in worktree cwd with captured
   output, ADR-0004 cwd discipline; 82 tests).
4. ~~Messaging wave (tickets 12–15 — ADR-0008, spec §11)~~ — **LANDED
   2026-07-15** (`15cd171` waves 2+2.5, `9e2f613` ticket 15; 79 tests).
   `msg` verb + `queues_midturn` + protocols brief msg-only + outbox drain
   in the hook. Codex mid-turn `pane run` live-verified: QUEUES cleanly —
   both shipped launchers `queues_midturn = true`; outbox covers launchers
   declaring false. Background research:
   `docs/research/native-teammate-parity-2026-07-15.md` +
   `docs/research/herdr-agent-messenger-2026-07-15.md`.
5. Run ticket 11 manifest actions and the live limux DoD from the god session
   with Caio watching. DoD now includes a live `msg` round-trip (spec §10)
   and ticket 08's deferred live pointer-injection check.
6. Only then talk to Caio about publishing; never push or add the
   `herdr-plugin` topic without explicit approval.

## Context that doesn't fit the docs

- Marketplace survey (175 plugins, 2026-07-14) is applied: curated conclusions
  in `docs/marketplace-notes.md` (patterns to steal with source pointers,
  competitive watch, Caio's install list); raw verdicts in
  `docs/marketplace-survey-2026-07-14.json`. Spec §9 TODO #1 (event name) is
  resolved from it.
- Caio plans to run coordinator (god) sessions inside herdr from now on —
  which this plugin's ADR-0002 report path requires anyway.
- Herdr is closed-source. Compatibility contract: snapshot `herdr api schema
  --json` into the repo and diff on herdr updates (not yet done — worth adding
  as a small script + CI-less check).
