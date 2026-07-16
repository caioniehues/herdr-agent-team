# Handoff — PIVOT to herdmates (ADR-0012); foundation in progress

Updated 2026-07-16, after the pivot grilling session with Caio.

## What changed

The project direction pivoted (decision record: `docs/adr/0012-pivot-to-herdmates.md`
— read it FIRST, it contains the full context, verified facts, and every
decision). One-paragraph version:

Claude Code native agent teams own orchestration from now on — we stop
building our own (Caio: "no point reinventing the wheel; it will be
worse"). This repo becomes **herdmates**: (1) a **shim** ("teammux") that
fakes tmux inside herdr panes so native split-pane teammates land as real
herdr panes, (2) an **agent board** (sidebar tokens first, TUI plugin pane
later), (3) a **focus pane** for the human (one next action + decision
queue, file contract at `~/.local/share/herdmates/focus.md`). Old frontier
#66–#83 closed wontfix; zero install base assessed, no maintenance
backlog.

## Foundation checklist (execution order, ADR-0012 §Execution)

1. [ ] Gate `integrate/program-wave1` centrally (fmt/clippy/tests, worktree
       `~/Projects/herdr-agent-team-loops/integration`), bump manifest
       version → 1.1.0, merge → main, tag `v1.1.0`, push (Caio authorized
       2026-07-16 in the pivot session).
2. [ ] Commit pivot docs on main (ADR-0012, CONTEXT.md, HANDOFF.md,
       CLAUDE.md, program learnings + docs/reviews records).
3. [ ] Close issues #66–#83 wontfix with pivot comment linking ADR-0012
       (#77/#79: comment "moot under pivot").
4. [ ] Delete empty batch worktrees + branches: fix-teardown-batch,
       fix-hook-batch, fix-godcli-batch, fix-msg-batch (under
       `~/Projects/herdr-agent-team-loops/`).
5. [ ] Rename repo → `caioniehues/herdmates` (`gh repo rename`), then
       first 2.0.0-line commit: manifest id `herdmates`, name "Herdmates",
       description "Claude Code teammates, native in herdr".

## Next work after foundation

1. **Recon spike** (shim go/no-go, ~1 worker-day): logging `tmux` wrapper
   first on PATH in a REAL tmux session; run a real native team
   (`CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1`, `teammateMode: tmux`);
   capture complete argv inventory; map verbs → herdr CLI. Kill signals:
   control mode (`tmux -C`) or unmappable verbs. Shim death ≠ pivot death:
   boards work over in-process teams; fallback includes upstream feature
   request for pluggable `teammateMode` backend.
2. **D1 sidebar-token agent board**: hook pumps team-file state
   (`~/.claude/teams/*/config.json` + `inboxes/*.json`) into
   `pane report-metadata --token …`; ship `[ui.sidebar.agents] rows`
   config. Exercises all data plumbing D3 reuses.
3. **D3 focus pane**: plugin pane rendering the focus file; companion
   atomizer skill (human-harness pattern, copy not depend).
4. **D2 rich TUI board / shim build** per spike verdict.

## Key verified facts for the new work (2026-07-16, full citations in ADR-0012)

- `teammateMode` split-pane = tmux + iTerm2 only, hardcoded; Ghostty
  explicitly unsupported; no pluggable backend. Teams experimental
  (`CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS`). Teammates = full independent
  `claude` sessions. No external-session adoption; membership fixed at
  spawn in `~/.claude/teams/{team}/config.json`; mailboxes JSON under
  `inboxes/`.
- Herdr has zero tmux surface (no `TMUX`, no control mode, "Herdr is not
  tmux"). Child processes create panes only via `herdr pane split
  --current` over `HERDR_SOCKET_PATH`.
- Herdr 0.7.4 observability surface: plugin panes
  (overlay/split/tab/zoomed, `plugin.pane.open`), popup panes
  (session-modal, NO pane id, invisible to pane/agent APIs — never the
  board's home), `pane report-metadata` (tokens ≤80 chars, ≤16/report,
  ≤32/pane, TTL, seq; display-only), `session.snapshot`,
  `workspace.metadata_updated` + `layout.updated` events,
  `terminal session observe`. Native non-terminal plugin UI = future,
  does not exist.
- Unreleased in upstream docs/next: per-token sidebar styling
  (fg/bold/dim) — relevant to D1 polish.

## State inherited from the orchestration line (context, not tasks)

- v1.0.0 released (`aa0c0e0`). Implementation-review program #46–#58 fully
  executed 2026-07-15 (Stage 0 twice-run E2E, 5 RED-first loops GREEN, 6
  review slices, Stage 3 vocabulary) + wave fixes #59/#61–#65. All on
  `integrate/program-wave1`: 197 tests, fmt/clippy clean — becomes v1.1.0
  (tombstone release of the orchestration line; 2.0.0 = pivot work).
- Reports: `docs/reviews/loops/`, `docs/reviews/slices/`; Stage 0 evidence
  `docs/reviews/evidence/stage0/`; learnings
  `docs/learnings/program-execution-2026-07-15.md` (worker traps: claude
  startup crash, Enter-swallow, gh flag no-ops — still relevant to future
  pane workers).
- `docs/reviews/frontier-plan-2026-07-16.md` is SUPERSEDED by ADR-0012
  (tickets closed, worktrees deleted). Keep as record only.
- Research branch `research/current-upstream-runtime-constraints`
  (worktree `/tmp/herdr-agent-team-constraints`, commit `8e5f105`) stays
  unmerged/unpushed — reference material; do not silently publish.

## Standing rules unchanged

- Pushes to main are releases — every push gated (fmt/clippy/tests),
  version bump on behavior change, tag releases, Caio's word required.
- Delegate implementation to claude workers in herdr panes; coordinator
  never implements in this repo. Fresh claude panes crash intermittently
  at startup (config-borne, unsolved) — verify alive before briefing.
- Verify external claims via ctx7/upstream source before building
  (ADR-0010 evidence hierarchy). The local `~/Projects/herdr-upstream`
  clone goes stale — `git pull` before citing it.
