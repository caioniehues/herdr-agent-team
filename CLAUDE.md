# CLAUDE.md — project context for Claude Code

Herdr plugin: spawn + run heterogeneous coding-agent teams (Claude, Codex, …)
under a coordinating "god" session. Pre-v1: docs are the contract, binary is
stubs.

## Read in this order

1. `HANDOFF.md` — current state + exact NEXT steps.
2. `docs/spec.md` — buildable v1 spec. §9 = open verification TODOs, §10 =
   definition of done.
3. `docs/adr/0001–0007` — locked decisions with the why. Don't relitigate
   silently; new evidence → new ADR, ask Caio first.
4. `CONTEXT.md` — vocabulary. Use these words exactly (god, worker, star/mesh,
   pointer injection, run-board, launcher table, status flip).

## Hard rules

- **Local git only. Never push / create the GitHub repo / add the
  `herdr-plugin` topic without Caio's explicit go-ahead** — the topic
  auto-publishes to the herdr marketplace within ~30 min.
- The herdr CLI (via `HERDR_BIN_PATH`) is the entire plugin API — no SDK.
  Ground truth for verbs: `herdr <cmd> --help` and
  `docs/herdr-api-schema.snapshot.json` (protocol 16 baseline; re-snapshot and
  diff after any `herdr update`).
- Port logic from limux-cli
  (`~/Projects/cmux-kde/limux/rust/limux-cli/src/main.rs`: `build_agents_md`,
  `agent_launch_command`) by **copying, not depending** (ADR-0005).
- Pane cwd is set at pane creation (`--cwd`), never via a `cd` in prompt text
  (ADR-0004 — split-brain trap).
- Report pointer injection into the god pane carries a file path only — never
  report content (ADR-0002).

## Verified facts (don't re-derive)

- Manifest event `on = "pane.agent_status_changed"` is valid — shipped plugins
  `cobanov/herdr-ntfysh` and `horn553/herdr-ntfy` use it (spec §9 TODO #1
  resolved). Payload-shape verification still pending.
- Codex TUI often needs two Enters to submit injected text; verify submission
  with `herdr agent wait --status working` (launcher table data, ADR-0006).
- Herdr agent status enum: idle/working/blocked/done/unknown.

## Environment note

Caio's machine has 10 marketplace plugins installed (ids + synergies: the
user-level `/herdr-plugins` skill, `~/.claude/skills/herdr-plugins/SKILL.md`).
Three of them hook `worktree.created` (tdi.worktree-setup, persiyanov.reviewr
auto-open, blurname.git-tab-name) — this plugin's spawn flow will fire that
same event per worker worktree, so test spawn WITH those installed; layout
races here are a feature-interaction bug, not a user config problem
(marketplace-notes.md pattern #3).

## Reference material in-repo

- `docs/marketplace-notes.md` — curated survey conclusions: patterns to steal
  (with source pointers), competitive watch list, race-avoidance convention.
- `docs/marketplace-survey-2026-07-14.json` — raw survey verdicts (69 deep
  dives) if the notes lack detail.

## Conventions

- Rust, `cargo fmt` + `clippy -D warnings` clean before commit.
- Every subcommand stub cites its spec section — keep that when implementing.
- Add regression tests alongside behavior; pure logic (spec parsing, AGENTS.md
  generation) stays separate from process-spawning code so it's testable.
