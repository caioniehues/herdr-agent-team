# Root cause: live E2E (#85 commit 9) blocked on `list-panes` / idmap

Written by the lead session (`489655e5`) attempting the live 2-teammate E2E
through `herdmates teammux-launch`. Session was launched with
`--settings {"teammateMode":"tmux"}`, confirmed via `ps -ef` and env
(`TMUX=teammux,0,0`, `CLAUDE_CODE_CHILD_SESSION=1`,
`CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1`). Claude version:
`2.1.211 (Claude Code)` (matches `claude-version.txt` in this directory).

## Symptom

Spawning either teammate (`alpha`, `beta`) via the Agent tool with the team
in tmux mode fails immediately:

```
Could not determine pane count for current window
```

Both spawns fail identically — no teammate pane is ever created, so the
E2E cannot proceed past step 1.

## Root cause

Claude Code's split-pane spawn path needs to enumerate the current
window's panes before deciding how to split. It calls teammux as:

```
tmux list-panes -t @0 -F '#{pane_id}'
```

`parse_list_panes` in `src/tmuxargs.rs` (line 281) parses this shape
correctly — **this is not an "unrecognized shape" bug** (an earlier repro
in this session tried malformed shapes — bare `list-panes`, `-t @0` with
no `-F`, `-a` — and got `unrecognized shape` errors that don't reflect
what Claude Code actually sends; those were dead ends).

The real failure is in `Verb::ListPaneIds` handling
(`src/teammux.rs:482`, `list_panes_at`, around line 508):

```
teammux: list-panes: herdr pane `w1A:p1` in tab `w1A:t1` has no tmux id
registered in idmap
```

`herdr pane list` for tab `w1A:t1` (window `@0`) returns **two** panes:

- `w1A:p1` — a pre-existing, unrelated Claude session (an observer/monitor
  session started independently, not launched via `teammux-launch`)
- `w1A:p15` — this lead session's own pane, self-registered in idmap as
  `%0` at launch (see `teammux/w1A_p15.json`:
  `{"entries": {"%0": "w1A:p15", "@0": "w1A:t1"}}`)

`list_panes_at` iterates every herdr pane in the tab and does a hard
`idmap.lookup()` per pane, erroring out on the **first** pane with no
registered id, rather than skipping unmapped panes or lazily assigning
them a new `%N`. Because `w1A:p1` was never registered (it isn't part of
this teammux session at all — it just happens to share the tab), the
whole enumeration fails, and Claude Code's spawn path — which only asked
"how many panes are in this window" — gets a hard error instead of a
count.

`src/idmap.rs` confirms there is no lazy-registration path: it only
exposes `lookup()` (read-only reverse lookup), no
`lookup_or_assign()`/`register_if_missing()` equivalent that
`list_panes_at` could fall back to.

## Why this matters beyond this one test

Any tab that contains a pane not launched through this specific
`teammux-launch` invocation — a human's own terminal pane, another
project's session, a leftover pane from a prior run — will permanently
block spawning teammates in tmux mode for the *whole tab*, not just
conflict on that one foreign pane. This is a structural gap, not a
one-off: teammux assumes it owns every pane in the tab it's launched in,
which doesn't hold in practice (this run's tab had exactly one foreign
pane, and that was enough to kill it).

## Suggested fix direction (not implemented — flagging for triage)

In `list_panes_at` (`src/teammux.rs:482`), either:
1. Skip herdr panes with no idmap entry instead of erroring (matches
   real tmux behavior more loosely — panes never seen by *this* tmux
   session just wouldn't have an assigned id in a fresh tmux server
   either, but they also wouldn't exist in that server's pane table at
   all, so "skip" is closer to the real semantics than "fail"), or
2. Lazily assign a new `%N` id to any herdr pane discovered via
   `list-panes` that isn't yet in idmap, persisting it back to the state
   file (matches tmux's actual behavior: `list-panes` always reports
   every pane in the window, and every pane always has a valid id).

Option 2 seems more correct — Claude Code's spawn path is trying to
*count* panes to decide split geometry, and a pane that silently vanishes
from the count would produce wrong geometry decisions downstream, whereas
a pane with a lazily-minted id behaves exactly like real tmux.

Needs a regression test: `list-panes -t @N -F '#{pane_id}'` against a
tab containing one self-registered pane and one pane with no prior idmap
entry should succeed and return both ids, not error.

## Outcome of this E2E attempt

Per Caio's decision: stopped after root-causing the blocker rather than
attempting an in-process fallback (that would require ending this lead
session, since `teammateMode` is fixed by the `--settings` CLI flag for
the process's lifetime — not something the running session can flip).
Tasks #1–#3 (native task list, this session's implicit team) left
pending/unowned. No teammates were ever successfully spawned. `alpha`/
`beta` never sent PING-ALPHA/PING-BETA. `E2E-COMPLETE` was intentionally
NOT printed — the test did not complete.
