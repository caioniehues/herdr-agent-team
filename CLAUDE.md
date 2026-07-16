# CLAUDE.md — project context for Claude Code

**Herdmates** (pivoted from herdr-agent-team, ADR-0012, 2026-07-16):
Claude Code teammates, native in herdr. Three surfaces: **shim**
("teammux" — fake tmux translating to herdr CLI so native split-pane
teammates land as real herdr panes), **agent board** (sidebar tokens, TUI
plugin pane later), **focus pane** (human focus: one next action +
decision queue). We do NOT build our own orchestration — native Claude
Code agent teams own spawn/messaging/lifecycle; we host + observe.

## Read in this order

1. `HANDOFF.md` — current state + exact NEXT steps (foundation checklist).
2. `docs/adr/0012-pivot-to-herdmates.md` — the pivot: full context,
   verified facts with citations, every decision, spike kill criteria.
3. `docs/adr/0001–0011` — locked decisions with the why. 0002/0003/0006/
   0007/0008/0009 describe the frozen legacy surface. No silent
   relitigating; new evidence → new ADR, ask Caio first.
4. `CONTEXT.md` — vocabulary. Pivot terms first (herdmates, native team,
   lead, teammate, teammate mode, shim, recon spike, team files, agent
   board, focus pane, focus file, atomizer, plugin pane vs popup pane,
   sidebar token); legacy terms below them.

## Hard rules

- Repo public (`herdr-plugin` marketplace topic). **Pushes to `main` are
  releases**: gate every push (fmt/clippy/tests), bump manifest `version`
  on behavior change, tag releases. No push without Caio's ask.
- The herdr CLI (via `HERDR_BIN_PATH`) is the entire plugin API — no SDK.
  Ground truth for verbs: `herdr <cmd> --help` and
  `docs/herdr-api-schema.snapshot.json` (re-snapshot + diff after any
  `herdr update`).
- Pane cwd is set at pane creation (`--cwd`), never via `cd` in prompt
  text (ADR-0004 — split-brain trap).
- Never re-implement native-team features (spawn, mailboxes, membership,
  lifecycle) — read the documented team files instead
  (`~/.claude/teams/{team}/config.json`, `inboxes/*.json`).
- Shim work is gated on the recon spike verdict (ADR-0012); do not build
  the translator before the verb inventory exists.
- Boards/focus pane must not require the shim — they work over in-process
  teams in any terminal.

## Verified facts (don't re-derive; authority tags per ADR-0010)

Herdr is **open source**: github.com/ogulcancelik/herdr; local clone
`~/Projects/herdr-upstream` — **`git pull` before citing, it goes stale**.
Evidence hierarchy: live = behavior, source = attribution, preview =
feature-detect (ADR-0010).

Pivot-relevant (all 2026-07-16, citations in ADR-0012):

- Claude Code `teammateMode`: `in-process` (default) | `auto` | `tmux` |
  `iterm2`. Split-pane hardcoded to tmux/iTerm2; Ghostty explicitly
  unsupported; NO pluggable backend `[doc]`. Teams experimental
  (`CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS`). Teammates = full independent
  `claude` sessions `[doc]`. Membership fixed at spawn (session IDs in
  team config); no external-session adoption `[doc]`.
- Herdr: zero tmux surface (never sets `TMUX`, no control-mode, "Herdr is
  not tmux") `[source]`. Child pane creation only via `herdr pane split
  --current` over `HERDR_SOCKET_PATH` `[source]`.
- Herdr 0.7.4: plugin panes (`plugin.pane.open`, overlay/split/tab/
  zoomed; overlay = real pane), popup panes (session-modal, NO pane id,
  invisible to pane/agent APIs, swallows all input — quick-glance only)
  `[doc]`. `pane report-metadata`: `--title`/`--display-agent`/
  `--state-label` + `--token name=value` (≤80 chars, ≤16 keys/report,
  ≤32/pane, TTL/seq guards; display-only, never semantic state) `[doc]`.
  Board plumbing: `session.snapshot`, `workspace.metadata_updated`,
  `layout.updated`, `terminal session observe` `[doc]`. Custom
  herdr-rendered plugin panels DO NOT exist ("native non-terminal plugin
  UI... later surface") `[doc]`.
- Legacy facts (msg/outbox/status semantics, `pane run` vs `agent send`,
  queues-midturn, status enum) remain documented in ADR-0006/0008 and the
  frozen legacy CONTEXT.md section — still true, no longer evolving.

## Environment note

Caio's machine has 10 marketplace plugins installed (ids + synergies:
user-level `/herdr-plugins` skill). Three hook `worktree.created` —
anything we ship that creates worktrees fires that event; test with those
installed (feature-interaction bugs, marketplace-notes.md pattern #3).

## Reference material in-repo

- `docs/reviews/` — executed v1.x review program (loops, slices, Stage 0
  evidence) + `frontier-plan-2026-07-16.md` (SUPERSEDED by ADR-0012,
  record only).
- `docs/learnings/` — wave learnings incl. worker traps (claude startup
  crash, Enter-swallow, gh flag silent no-ops).
- `docs/marketplace-notes.md` + `docs/marketplace-survey-2026-07-14.json`
  — marketplace survey conclusions.

## Agent skills

Config for the mattpocock/skills engineering workflow.

### Issue tracker

GitHub Issues via `gh` CLI (repo renames to `caioniehues/herdmates`; `gh`
follows the redirect). See `docs/agents/issue-tracker.md`.

### Triage labels

Canonical five-role vocabulary, default strings (`needs-triage` /
`needs-info` / `ready-for-agent` / `ready-for-human` / `wontfix`).
See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `CONTEXT.md` + `docs/adr/` at the repo root.
See `docs/agents/domain.md`.

### Research rules

Research external repos/libs/docs via **ctx7** (find-docs skill) first,
upstream source second, live behavior decisive. Never assume — verify
inherited claims before building on them. See `docs/agents/research.md`.

## Conventions

- Rust, `cargo fmt` + `clippy -D warnings` clean before commit.
- Every subcommand stub cites its spec/ADR section — keep when
  implementing.
- Regression tests alongside behavior; pure logic (file-contract parsing,
  token formatting, verb mapping tables) stays separate from
  process-spawning code so it's testable.
