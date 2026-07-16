---
name: atomizer
description: Turn a rambling task dump into the single next concrete action and write it to the herdmates focus file (~/.local/share/herdmates/focus.md). Use when the user brain-dumps a task, a scattered list of TODOs, or half-formed context and wants one clear next step captured for the herdmates focus pane — not a full plan, not a TODO list.
---

# Atomizer

Modeled on the human-harness ADHD pattern (dhasson04/human-harness) —
copied, not depended on (same rule as ADR-0005: port the pattern by hand,
don't add it as a dependency). A task dump usually contains many possible
next steps; this skill's job is finding the *one* that is truly next and
concrete enough to start on immediately, not producing a plan or a TODO
list.

## What "atomize" means here

- Read the user's dump (a message, a pasted note, a stream of half-formed
  thoughts — whatever they gave you).
- Pick the single smallest next physical, concrete step that moves the
  task forward right now. Not "figure out the approach," not "look into
  X" — an actual verb + object someone could start doing in the next five
  minutes.
- State it as one sentence, imperative mood, no hedging, no "maybe" or
  "consider."
- If the dump also names the broader task (not just the next step),
  capture that too — it becomes `## Task`, distinct from the one
  `## Next Action`.

## Writing it to the focus file

Never hand-edit `~/.local/share/herdmates/focus.md` for this — the focus
file's `## Decisions` section is owned by other writers (the focus pane,
a human), and hand-editing risks clobbering it. Use the script; it
preserves the existing `## Task` and `## Decisions` sections untouched
and only replaces `## Next Action`:

```bash
${CLAUDE_PLUGIN_ROOT:-.}/skills/atomizer/atomize.sh "<the one next action, one sentence>"
```

Pass a second argument to also update `## Task`:

```bash
${CLAUDE_PLUGIN_ROOT:-.}/skills/atomizer/atomize.sh "<next action>" "<broader task>"
```

The script is self-contained POSIX `sh` (no dependency on the herdmates
Rust crate or binary, and no other tool dependency) — it reads the
existing file (if any), keeps `## Task` (unless overridden) and
`## Decisions` verbatim, and writes the canonical three-section format
that `src/focusfile.rs` parses (see `docs/focus-file.md`).

After writing, tell the user in one line what got extracted (the next
action, and the task if you set one) so they can correct it before
opening the focus pane — this skill makes a call, it doesn't ask first,
because re-atomizing is a one-command retry.
