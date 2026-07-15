# HANDOFF — next session orientation

Last updated 2026-07-15 (post research wave + docs overhaul).

## Read first

1. `docs/spec.md` — v1 spec; §8 = post-v1 roadmap (research-backed wave),
   §9 = authority-tagged verified facts.
2. `docs/adr/0001–0011` — locked decisions + why. New evidence → new ADR,
   ask Caio first. ADR-0010 (evidence hierarchy) and ADR-0011 (socket
   backend) are the newest.
3. `CONTEXT.md` — vocabulary. `docs/agents/research.md` — research rules
   (ctx7 first; never assume; verify inherited claims).

## State

- **v1 SHIPPED + PUBLISHED** (2026-07-15, Caio's go-ahead):
  https://github.com/caioniehues/herdr-agent-team — public, topic
  `herdr-plugin`, tag `v0.1.0`, marketplace-listed. Pushes to `main` are
  releases: gate (fmt/clippy/tests), bump manifest version for behavior
  changes, tag, never push without Caio's ask.
- DoD passed 2026-07-15 (run 2, limux repo, live): spawn, worktrees,
  pointer injection, msg round-trip, kill preserving dirty worktree.
- `team adopt` shipped (`10a855a`, closes #1): existing panes become full
  workers; ADR-0009, spec §12.
- **Herdr is OPEN SOURCE** — github.com/ogulcancelik/herdr (Rust core,
  vendored Zig libghostty-vt). The old "closed-source" note was an
  unverified assumption, corrected 2026-07-15 (ADR-0010). Local clone:
  `~/Projects/herdr-upstream`. Schema-snapshot discipline stays as drift
  detection (`docs/herdr-api-schema.snapshot.json`, protocol 16).
- **Research wave 2026-07-15** (4 reports in `docs/research/`): upstream
  architecture + claims audit, integration opportunities, herdr-claude-teams
  competitor analysis (verdict: pattern-source, not threat), awesome-herdr
  ecosystem survey (133 entries). Key corrections live in spec §9; roadmap
  rewritten in spec §8 from this evidence (grilled decisions Q1–Q6 with
  Caio, 2026-07-15).
- Central gate green at last commit: build, fmt, clippy `-D warnings`,
  98 tests.

## NEXT steps (in order)

0. **v0.6.0 RELEASED 2026-07-15** (#7): native board pane = the human's
   CONTROL DECK (variant-D prototype verdict; branch prototype/board-pane is
   the primary source). `[[panes]] board` + open-board action + report: link
   handler; real per-row verbs incl. NEW `kill <run> --worker <name>`;
   optional worker `task` field; deps deferred. Collection behind
   `BoardCollector` trait for #8's socket swap. 128 tests. Architecture
   consolidation pass scoped next (HerdrApi seam, before #8).
1. **v0.5.0 RELEASED 2026-07-15** (third release today): #6 schema-gated
   metadata tokens (`src/metadata.rs` maps team facts onto the REAL 0.7.3
   tokens — title/display_agent/custom_status/state_label; runtime
   `api schema --json` gate with fallback; probe results on issue #6) +
   aggregate notifications (once-only team-complete / blocked-beyond-threshold
   with all-event sweep / worker-exit / explicit `msg --attention`).
   123 tests. NEXT: #7 board pane — /prototype the layout FIRST (Caio's
   instruction), then ticket. Then #8 socket backend (consider architecture
   pass before it: HerdrApi duplicate, FakeHerdr x4, hook.rs seam).
1. **v0.4.0 RELEASED 2026-07-15** (same day as v0.3.0 hook-correctness
   wave): #5 full `agent_session {source,agent,kind,value}` +
   `HerdrSessionIdentity` (HERDR_SOCKET_PATH/HERDR_SESSION) persisted per
   run at spawn/adopt, legacy run.toml still loads; #15 generated protocols
   now encode the git contract by worktree flag (worktree workers
   commit/push/PR their own branch; shared-tree and adopted panes stay
   no-git). 117 tests. Open follow-ups: #14 spawn dies midway (pending
   lifecycles), #16 manifest changes need plugin unlink+link, #17 serial
   90s agent-info timeout delays worker N+1.
4. **Roadmap step 4 / Issue #7:** native board pane (`[[panes]]` + action +
   keybinds + link handler).
5. **Roadmap step 5 / Issue #8:** direct socket backend behind `HerdrApi`
   (ADR-0011); #2 team wait rides it.
6. **Roadmap step 6 / Issue #9:** run-scoped broadcast, bounded previews, and
   conservative restart (blocked by #5).
7. **Roadmap step 7:** later/optional declarative layouts, Kitty-graphics
   board enrichment, run-history browsing, tested opencode/gemini launchers,
   and limux backend extraction.

Work them via codex pane workers (never implement in this repo from the
coordinator — memory rule), one ticket per worker worktree. Git contract
(2026-07-15): worktree workers commit/push/PR their own branch; coordinator
reviews, runs the shared gate once centrally, merges, and releases on Caio's
word.

## Context that doesn't fit the docs

- Marketplace survey (175 plugins) + awesome-herdr survey conclusions:
  `docs/marketplace-notes.md`; raw verdicts in the two survey JSON/report
  files. Competitive watch: herdr-factory, dual-author, herdr-orchestrator,
  Shepherd, herdr-symphony, herdr-factory-loop-skill, herdr-claude-teams.
- Caio runs god sessions inside herdr; research/analysis fan-outs run as
  **visible herdr pane teammates** (codex yolo), never invisible Agent-tool
  subagents (2026-07-15 incident: mailbox-spawned agents never started).
- Watch item: optional Claude-native visible-team compatibility mode
  (herdr-claude-teams proved feasibility) — separate experiment, never core.
