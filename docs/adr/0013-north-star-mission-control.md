# ADR-0013 — North star: herdmates mission control (wayfinder map #87 record)

Date: 2026-07-17
Status: accepted
Supersedes ADR-0012 where they conflict (specifically the teammux
control-mode kill-signal caveat, corrected below). ADR-0012's pivot
decision, identity, and legacy disposition stand unchanged. ADRs
0001–0011 remain accurate history for the frozen v1.x surface.

## Context

ADR-0012 pivoted this repo to herdmates: hosting and observing native
Claude Code agent teams in herdr, never re-implementing orchestration.
It left the destination underspecified: shim gated on a recon spike,
board/focus surfaces sketched, no build order. The wayfinder map
(issue #87) closed that gap across seven tickets (2026-07-16/17), each
resolved with evidence per the ADR-0010 hierarchy (live > source > doc)
and grilled with Caio. This ADR is the durable record; the rewritten
`docs/spec.md` is the buildable north star.

## Decisions

### Charter (grilled 2026-07-16, #87 body table)

| Question | Call |
|---|---|
| Identity | Herdr plugin, period — no standalone reader, no web app |
| Monitor/gate stack | Full: file-reader monitor + inbox-write steering + hook companion |
| Team scope | Single-team v1; data model enumerates `~/.claude/teams/*` from day one |
| Shim | First-class pillar, explicitly Claude-teams-only |
| Recorder | V1, minimal append-only observed-state log; replay UI later |
| ETA | Honest proxy only (done/total, per-task elapsed); time prediction banned |
| Rich surface | Full-screen herdr TUI plugin pane owns the rich tier |
| Task source | Native `~/.claude/tasks/` files; beads out of scope |

### #88 — task-file schema `[live 2026-07-16]`

Stable core: `id/subject/description/status/blocks/blockedBy`; status
enum `pending|in_progress|completed`; edges = task-id string arrays;
everything else optional. No version field — drift risk HIGH; parsers
tolerate unknown/absent fields. Inboxes are transient (drained on
read) — polling tails are lossy. Stale team configs persist: config
presence ≠ active team. Findings:
`docs/research/native-teamfiles-schema-2026-07-16.md`.

### #91 — hook surface `[doc+live 2026-07-16]`

Exactly three team hook events (`TeammateIdle`/`TaskCreated`/
`TaskCompleted`), no matchers, only exit-2 blocks. No spawn/message/
plan-approval hooks (the cannot-gate list). Plugins can ship hooks.
Inbox writes require `.lock` sidecar + read-filter-atomic-rename.
Mailboxes materialize in ALL teammate modes. Findings:
`docs/research/hook-companion-surface-2026-07-16.md`.

### #89 — shim proven `[live 2026-07-17]`

Live E2E PASS (attempt 2): native teammates spawned as real herdr
panes through teammux, live `blockedBy` DAG completed, clean
dismissal. Bonus schema: inbox entry
`{from,text,timestamp,msgV,msg_id,type,read}`; dismissal prunes
members from team config; team creation implicit on first named
spawn; task `owner` observed as `""` AND `null`. Evidence:
`docs/research/teammux-e2e-2026-07-16/attempt-2-results.md`.

### ADR-0012 correction: control-mode kill signal CLEARED

ADR-0012 gated the shim on the risk that Claude Code drives tmux via
control mode (`tmux -C`). Three independent sources cleared it: the
spike's 36-call argv capture `[live]`, binary probes finding no
control-mode invocation pattern `[bin]`, and the live E2E `[live]`.
See `docs/research/cmux-limux-herdr-comparison-2026-07-16.md` §1.3.
The shim is unconditionally a pillar; the spike gate is discharged.

### #92 — waiting-reason + deadlock signals (grilled 2026-07-17)

Four-class taxonomy, precedence top-down, one badge per teammate:
permission-prompt (herdr `agent_status: Blocked`, pane-backed only,
never inferred) > blocked-on-dependency (idle + owned task with
incomplete `blockedBy`) > stalled > turn-complete (unbadged default).
Stalled is two-tier multi-signal: "quiet" soft at 5 min, "stalled"
hard at 10 min (T configurable); liveness ground truth = session-
transcript mtime (mode-independent, immune to the idle-vs-done
attention-state trap); unread-inbox accelerator ~2 min. Stance:
never display a wrong reason — degrade to reason-less "waiting"
(same honesty doctrine as the ETA ban). Affordances tiered by
surface: sidebar display-only; TUI pane jump-to-pane + human-
confirmed suggested nudge; NO auto-nudge in v1. Full table:
#92 resolution comment.

### #90 — focus pane: distinct surface, shared signal engine

Board = team telemetry (per-agent state + reason); focus pane =
human focus (one next action + decision queue). The #92 signal
engine is the single source of blocked/stalled facts — board renders
all of it, focus pane consumes only human-needing items; neither
re-derives, so they cannot disagree. `feat/86-focus-pane` merged to
main `f0441f4` (merge-now-refactor-later); the `attention.rs`
blocked-worker derivation migrates into the shared engine as build
stage 1.

### #93 — v1 build order + cut line (grilled 2026-07-17)

Six stages, each ending dogfood-able:

0. Version reconcile (Cargo.toml vs manifest) — release-blocking chore.
1. **Tracer bullet**: extract the signal engine as a lib module
   (attention.rs migrates in); focus pane + sidebar tokens both
   consume it. Dogfood = live reason badges on the sidebar.
2. Minimal recorder appending the engine's classified observations;
   log schema = engine output schema.
3. Read-only TUI plugin pane: overview, per-agent rows (glyphs +
   reason badges), flat task list, mailbox tail, metadata row.
4. Pane affordances: jump-to-pane + human-confirmed nudge — the
   first inbox write (#91 lock+atomic discipline), its own gated
   stage after the read surface is dogfooded.
5. Hook companion, observe-first: the three hooks push into engine +
   recorder (event-driven with polling fallback, closing the lossy-
   polling gap); exit-2 gating ships default-off.

Shim ships as-is (proven); focus pane already merged (its engine
rewiring IS stage 1).

**Cut line (post-v1):** the JSONL tier — context bar, cost footer,
per-agent activity tail (v1 data sources = team files + herdr CLI +
transcript mtime-stat only; no JSONL parsing); task-DAG lane view;
comm graph; multi-team board UI; recorder replay UI; auto-nudge;
hook gating on-by-default.

## Consequences

- The build plan is fully decided; execution issues follow the six
  stages (creation gated on Caio's word).
- Risk posture is layered: deterministic reads shown immediately;
  heuristics (stalled) tiered and conservative; writes (nudge) human-
  confirmed; blocking (hook gate) default-off. Each capability class
  earns trust before the next activates.
- The JSONL parsing surface is deliberately deferred — #88's HIGH
  drift risk applies doubly to an undocumented transcript format;
  the honesty doctrine prefers no number to a wrong one.
- `docs/spec.md` is rewritten as the north star; the legacy v1 spec
  moves to `docs/legacy/spec-v1.md` (frozen, still accurate for the
  v1.1.0 tombstone surface).

## Alternatives rejected

Recorded per ticket in the resolution comments (#88–#93): notably
surface-first or recorder-first tracer bullets (#93), context-bar-only
JSONL parsing (#93), auto-nudge and gating-on-by-default (#92/#93),
merging board and focus pane into one surface (#90).
