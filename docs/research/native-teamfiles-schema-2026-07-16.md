# Native Claude Code team-file schema — verified live (2026-07-16)

Resolves #88. Evidence hierarchy per ADR-0010: live files (this machine,
`~/.claude/tasks/`, `~/.claude/teams/`) first, official docs
(`code.claude.com/docs/en/agent-teams`, fetched via `defuddle`, doc dated
"as of v2.1.178") second, training data never used.

Sample: 44 task JSONs read in full across 6 session dirs (`c7a38ce5`
8afd3bf0`, `744c7913`, `fec1fae5`, `fab99523`, plus grep sweep across all
220 `~/.claude/tasks/session-*/` dirs and all task files in them for
enum/edge values). 12/12 live team configs read in full
(`~/.claude/teams/session-*/config.json`).

## 1. Task file — `~/.claude/tasks/{team-name}/{n}.json`

One file per task, `n` = 1-based integer id assigned in creation order
(also duplicated as the string `"id"` field inside).

| Field | Type | Presence | Observed values |
|---|---|---|---|
| `id` | string | always | `"1"`, `"2"`, … — matches filename stem |
| `subject` | string | always | short task title |
| `description` | string | always | longer instructions |
| `activeForm` | string | **sometimes** | present in 2 of 4 observed key-sets (e.g. `session-c7a38ce5`, `session-744c7913`); absent in others (`session-8afd3bf0`, `session-fab99523`). Gerund-form status line ("Vetting Patch 2.1 (mod 341)") — this is the UI's "what's happening now" label |
| `status` | string enum | always | exactly three values seen: `"pending"`, `"in_progress"`, `"completed"` — note **snake_case** `in_progress` in the JSON even though the docs prose says "in progress" |
| `blocks` | array | always | **empty `[]` in all 44 sampled files, and in a full-corpus grep across all 220 session dirs — zero non-empty instances found live** |
| `blockedBy` | array | always | same — **empty `[]` in every live file found, no dependency edges observed on this machine.** Shape of a populated edge is therefore unverified live; doc confirms the *mechanism* exists (see §5) but not the JSON shape |
| `owner` | string | **sometimes** | only 6/44 files had it, all in one session (`8afd3bf0`, a codex-worker-fleet run). Format: `"codex-1 (wB:p2)"` — free-text, not a structured agent-id reference. This looks like an artifact of that session's own convention (codex workers self-reporting an owner string), not a guaranteed schema field |
| `metadata` | object | **sometimes** | present in 2/44 sampled; freeform, only two keys ever seen inside it: `verdict` (string, human summary at completion) and `result` (seen once, not inspected in depth). Not a fixed sub-schema — treat as opaque bag |

Four distinct top-level key-sets observed across the corpus:
1. `id, subject, description, activeForm, status, blocks, blockedBy, metadata`
2. `id, subject, description, activeForm, status, blocks, blockedBy`
3. `id, subject, description, status, blocks, blockedBy, owner`
4. `id, subject, description, status, blocks, blockedBy`

`activeForm`, `owner`, and `metadata` are all optional/absent depending on
how the task was authored — a parser must treat them as optional, not
assume any subset is guaranteed beyond `id/subject/description/status/
blocks/blockedBy`.

Example (`~/.claude/tasks/session-c7a38ce5/5.json`):
```json
{
  "id": "5",
  "subject": "WASO (549): vet WeaponInfo.xml vs CE, ...",
  "description": "...",
  "activeForm": "Vetting WASO weapon overhaul",
  "status": "completed",
  "blocks": [],
  "blockedBy": [],
  "metadata": { "verdict": "WeaponInfo.xml (accuracy) installed — ..." }
}
```

No timestamp field exists inside any task JSON. Ordering/recency is only
recoverable from filesystem mtime (verified: `session-fab99523/*.json`
mtimes ranged 00:42–02:16 on 2026-07-10, ascending with id, consistent
with creation-order writes — not a schema guarantee).

## 2. `.lock` — `~/.claude/tasks/{team-name}/.lock`

One per session dir (44/44 sampled dirs that had task files also had a
`.lock`). **Always 0 bytes** in every instance checked (8 sampled
directly, byte count verified with `wc -c`). Its role per the docs (§5
below) is a claim/write lock during task mutation — the file itself
carries no persisted state, existence + OS-level locking (flock-style) is
the entire mechanism. mtime updates on each lock acquisition but that's
incidental, not schema.

## 3. `.highwatermark` — `~/.claude/tasks/{team-name}/.highwatermark`

**Not universal** — present in only some session dirs (24 of the ~220
scanned, e.g. `session-bc572f1f`, `session-121699c5`, `session-3492e231`).
Content: a single-line plain integer, no JSON, no trailing newline
counted beyond 1 byte (`session-bc572f1f` → `4`, `session-121699c5` →
`3`, `session-3492e231` → `4`). Values are small integers plausibly
tracking "highest task id ever allocated" for that session (as opposed to
current file count, which can differ if tasks are pruned) — this
interpretation is inferred from the name and value range, not confirmed
in docs; **flagged as an open gap**.

## 4. Team config — `~/.claude/teams/{team-name}/config.json`

Single file per team (12/12 live, no subdirectory, no lock file
alongside it). Full schema, cross-checked against doc's explicit claim
("contains a members array with each teammate's name, agent ID, and
agent type... holds runtime state such as session IDs and tmux pane
IDs"):

| Field | Type | Notes |
|---|---|---|
| `name` | string | `session-{8-char-id}`, matches dir name |
| `createdAt` | number | epoch millis |
| `leadAgentId` | string | `team-lead@{team-name}` |
| `leadSessionId` | string | full UUID of the lead's Claude Code session |
| `members` | array | one entry per team-lead + spawned teammate |

`members[]` entry (team-lead variant, minimal — seen in 7/12 configs
that never spawned a teammate):
```json
{
  "agentId": "team-lead@session-01c224bb",
  "name": "team-lead",
  "agentType": "team-lead",
  "joinedAt": 1783265984666,
  "tmuxPaneId": "leader",
  "cwd": "/home/caio/second-brain",
  "subscriptions": [],
  "backendType": "in-process"
}
```

`members[]` entry (spawned-teammate variant — observed live in this
session's own config, `~/.claude/teams/session-7ee975f4/config.json`,
which includes this very research task and its sibling `research-hooks`
teammate):
```json
{
  "agentId": "research-schema@session-7ee975f4",
  "name": "research-schema",
  "color": "blue",
  "joinedAt": 1784216691303,
  "tmuxPaneId": "in-process",
  "subscriptions": [],
  "agentType": "general-purpose",
  "model": "claude-sonnet-5",
  "prompt": "<the full literal briefing prompt text>",
  "planModeRequired": false,
  "cwd": "/home/caio/Projects/herdr-agent-team",
  "backendType": "in-process"
}
```

Notable, not documented on the page: **`prompt` stores the teammate's
entire original briefing verbatim**, plus `model`, `color`, and
`planModeRequired` — none of these are mentioned in the doc's members-array
description ("name, agent ID, agent type"). This is a doc-vs-live gap:
the live schema is richer than documented.

Field-by-field:
- `agentId` — `{name}@{team-name}`
- `name` — short handle used in SendMessage `to:`
- `agentType` — enum seen: `"team-lead"`, `"general-purpose"`. Only two
  values across all 12 configs; no evidence of other subagent-type
  strings landing here (would need a config with a non-general-purpose
  spawned teammate to confirm — not present on this machine)
- `color` — only on spawned teammates, not on team-lead. Free string
  (`"blue"`, `"green"`) — display hint for the sidebar/board, not
  semantic
- `joinedAt` — epoch millis
- `tmuxPaneId` — **only two values ever seen: `"leader"` and
  `"in-process"`**. No live tmux/iTerm2-backed pane ID captured on this
  machine — every teammate here ran `backendType: in-process`, consistent
  with CLAUDE.md's note that split-pane backends are tmux/iTerm2-only and
  this machine uses herdr (zero tmux surface), so agent-team spawns here
  never take the tmux path. This means teammux/shim work cannot verify
  real tmux pane-id shape from local samples — would need a machine
  actually running `teammateMode: tmux`
- `subscriptions` — array, **empty `[]` in all 12 configs, no live
  example of a populated subscription**
- `backendType` — only `"in-process"` observed
- `model` — spawned teammates only; literal model id string
  (`"claude-sonnet-5"`)
- `prompt` — spawned teammates only; full prompt text
- `planModeRequired` — spawned teammates only; boolean, `false` in both
  observed instances
- `cwd` — present on every member (lead and teammate)

## 5. Inbox / mailbox files — **none found live**

Explicit finding: `find ~/.claude/teams -type d -iname 'inbox*'` and
`find ~/.claude/teams -type f -iname '*inbox*'` both returned **zero
results** across all 12 team dirs. No `inboxes/` subdirectory exists
anywhere on this machine at time of writing, despite one team (this
session, `session-7ee975f4`) actively running two teammates that have
exchanged no messages yet by the time of this check.

Docs describe the mechanism in detail even though no live sample exists
here: "Each agent's mailbox is a JSON file at
`~/.claude/teams/{team-name}/inboxes/{agent-name}.json`. Claude Code
validates every entry when it reads a mailbox file. Entries that don't
match the message format are reported as errors and removed from the
file; the valid messages are still delivered. Before v2.1.207, a single
malformed mailbox entry caused a repeated error every second and blocked
delivery for that mailbox until you deleted the file manually."

This confirms the path convention CLAUDE.md already assumes
(`~/.claude/teams/{team}/inboxes/{agent}.json`) but the exact per-message
JSON shape (sender, type, read/unread flag, idle-notification form) is
**unverified — no live sample obtainable on this machine**. Best
hypothesis from the doc: inbox files are likely transient/delivery-queue
files, drained and possibly deleted once messages are delivered to the
recipient's context — which would explain why none persist to be
observed even in an active team. Flagged as an open gap requiring either
a live capture mid-message-delivery (race to `ls` the instant before
delivery) or upstream source inspection.

## 6. Doc-vs-live mismatches

1. **Team config directory lifecycle.** Doc states: "The team config
   directory is removed when the session ends." Live evidence
   contradicts this at face value: 12 team config directories exist on
   this machine, several dated 2026-07-05 (11 days before this
   investigation, e.g. `session-01c224bb`, `session-4cbf1e80`,
   `session-da79e7c4`, `session-f6f47ab6`, all `createdAt` ≈
   2026-07-05T00:2x–16:3x) — well past any plausible single-session
   lifetime. Either (a) those Claude Code sessions never cleanly
   "ended" (crash, force-quit, killed pane) and cleanup only fires on
   graceful exit, or (b) cleanup has a bug/gap. This directly affects a
   mission-control board: **team config presence is not a reliable
   "team is currently active" signal** — stale configs accumulate.
   Task-list directories persisting is *expected* per docs ("task list
   directory persists locally... resumed sessions keep their tasks");
   it's specifically the *team config* persistence past session end that
   contradicts the doc.
2. **Members-array richness.** Doc says team config's members array
   holds "each teammate's name, agent ID, and agent type." Live schema
   is richer: `model`, `prompt` (full text), `color`, `planModeRequired`,
   `cwd`, `subscriptions`, `backendType`, `joinedAt`, `tmuxPaneId` all
   present and undocumented on this page.
3. **Status casing.** Doc prose says tasks have states "pending, in
   progress, and completed" — live JSON uses snake_case `in_progress`
   for the middle state, not `"in progress"`. Minor but load-bearing for
   a parser doing string matching.

## 7. Schema stability / versioning

**No version field found in any task JSON, team config, `.lock`, or
`.highwatermark` file.** The doc self-dates ("This page describes agent
teams as of v2.1.178") and separately notes a mailbox-format behavior
change at v2.1.207 (malformed-entry handling) — meaning the on-disk
formats are known to have shifted across Claude Code releases without a
schema version marker in the files themselves. Local Claude Code build
present: `2.1.211` (`~/.local/share/claude/versions/2.1.211`).

**Drift risk: HIGH for a board that hard-codes field presence.** Given
(a) no version field to gate on, (b) four different observed key-sets
for task JSON depending on how/when the task was authored, (c) at least
one documented breaking behavior change (mailbox parsing, v2.1.207)
already landed silently, any mission-control parser must:
- treat every field beyond `id/subject/description/status/blocks/
  blockedBy` as optional,
- tolerate unknown extra fields (forward-compat),
- re-verify against a fresh live sample after any `claude` version bump,
  the same discipline this repo already applies to `herdr update`
  (CLAUDE.md's re-snapshot rule) — worth mirroring here for Claude Code
  itself.

## 8. Open gaps (not verifiable on this machine)

- Populated `blockedBy`/`blocks` edge shape — no live example exists
  anywhere in ~220 session dirs on this machine. Doc confirms the
  *mechanism* (dependency resolution, auto-unblock) but not the JSON
  representation (task-id strings? array of `{id}` objects?).
- Inbox/mailbox per-message JSON shape — no live file exists to sample.
- `tmuxPaneId` real-pane-id shape under `backendType: tmux`/`iterm2` —
  this machine has never run agent teams over a tmux/iTerm2 backend
  (herdr has zero tmux surface per CLAUDE.md), so only `"leader"` and
  `"in-process"` were observable.
- `metadata.result` key (seen once) — not deeply inspected, exact
  content/shape not catalogued.
- `.highwatermark` semantics — inferred from name + small-integer values
  only, not confirmed against docs or binary strings (binary corroboration
  step was skipped: `~/.local/share/claude/versions/2.1.211/claude` path
  from the brief did not resolve to a single `strings`-able executable
  file in the time available — the versions dir turned out to be a
  directory tree, not a flat binary path, and a targeted `find` for the
  actual executable was not completed).
- `agentType` values beyond `"team-lead"` / `"general-purpose"` — no
  live config on this machine has a spawned teammate with any other
  subagent type.

## 9. Coordinator verification addendum (2026-07-16, same day)

Independent spot-check of this report against live files closed the two
largest gaps in §8 — both with samples created *after* (or missed by)
the research sweep:

- **Populated edge shape VERIFIED LIVE**: `~/.claude/tasks/session-fab99523/`
  holds 5 tasks with non-empty edges. Shape is **arrays of task-id
  strings**, both directions, e.g.
  `{"id":"5","blockedBy":["2","3"],"blocks":["6"]}`. Not `{id}` objects.
- **Inbox file OBSERVED LIVE**: this coordinator session's own team
  produced `~/.claude/teams/session-7ee975f4/inboxes/team-lead.json`.
  Top-level shape is a **JSON array**; observed `[]` immediately after a
  teammate message was delivered to the lead — confirming §5's
  drained-on-read/transient hypothesis with evidence. Per-message entry
  shape remains uncaptured (drain won the race); the #89 E2E should
  capture a non-empty inbox mid-flight.
- Status enum corroborated across all live task files:
  21× `completed`, 5× `in_progress`, 18× `pending` — no fourth value.

Lesson for the board: inbox files are a *live wire*, not a log — a
poll-based mailbox tail will miss drained messages, so the recorder
(charter decision) needs tight poll cadence or must accept lossiness;
note for #92/#93.
