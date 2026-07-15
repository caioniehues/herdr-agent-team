---
name: god
description: Coordinate herdr-agent-team workers from the god session. Use when spawning, briefing, monitoring, messaging, triaging, reviewing, or integrating a team run, including interrupted runs and stopped workers.
---

# God-side coordination

Act as the god for one team. Treat the run-board and inbox as durable truth; use Herdr presentation state only as a signal to investigate.

Set `RUN` to the absolute run dir returned by `spawn`, then pass `--run "$RUN"` on every command. Direct invocation also works without plugin-injected environment: when `HERDR_PLUGIN_STATE_DIR` and `HERDR_PLUGIN_CONFIG_DIR` are absent, the binary resolves Herdr's XDG/home layout from manifest id `caioniehues.agent-team`; explicit environment always wins.

## Brief workers by contract

Give every worker a brief that makes completion mechanically checkable. Use this template and replace every placeholder:

````markdown
# Brief — <worker>: <bounded responsibility>

Work only in the absolute directory `<absolute-cwd>` on branch `<branch>`.

## Deliverable

- Own only: `<exact files or seam>`.
- Do not change: `<explicit exclusions>`.

## Read first

1. `<absolute path>`
2. `<absolute path>`

## Verification

Run exactly:

```text
<command 1>
<command 2>
```

## Git contract

- Dedicated worktree: make small conventional commits, push this branch, and open a PR. Never merge, tag, or touch the default branch.
- Shared tree or adopted worker: do not run Git. The god owns all Git operations.

## Reporting

- Before becoming idle, blocked, or done, write `<absolute-run-dir>/inbox/<worker>.md`.
- Include status, files changed, verification results, blockers, commit SHA, and PR URL when applicable.
- Ping the god through the generated protocol's `msg` command after each material phase and about every 10 minutes during long work.
- If blocked, send an immediate `msg god` with the blocker and needed decision.
- After the report is safely written, print this line on its own:

```text
HERDR_TEAM_WORKER_COMPLETE
```
````

Keep paths absolute. Put long payloads in files and send pointers. Set worker cwd at pane creation; never ask a worker to repair cwd with `cd` in prompt text. The immutable generated worker protocol supplements the repository-authored `AGENTS.md`; it does not replace it.

## Spawn and resume

Spawn from a reviewed team spec:

```bash
herdr-agent-team spawn <spec>
```

If spawn stops after creating only part of the team, preserve the run dir and salvage it:

```bash
herdr-agent-team spawn --resume "$RUN"
```

Resume leaves running and adopted workers untouched, reuses live recorded panes and worktrees, recreates missing resources, and advances only pending workers. Do not start a second run to compensate for a half-spawned team.

## Wait

```text
herdr-agent-team wait [--run <dir>] --until any-report|report:<worker>|all-reports|blocked|attention|all-terminal [--timeout <s>] [--json]
```

Use the narrowest condition that unlocks the next coordinator action. Always specify a finite `--timeout` even though the default is 300 seconds. `--json` emits one line.

Exit codes:

- `0`: condition reached.
- `1`: usage, I/O, or configuration error.
- `2`: timeout; inspect, communicate, then wait again with a fresh bound.
- `3`: a required worker failed or became orphaned without a report; triage immediately.

Never key coordination on `done`: it is an idle-plus-unseen attention presentation. Report-file existence is completion truth. `blocked` and `attention` come from durable hook metadata; `all-terminal` comes from worker lifecycle and does not mean every report exists.

## Inbox

```text
herdr-agent-team inbox [--run <dir>] [--unread] [--json]
```

Inspect one row per worker: report presence and mtime, attention, read state, and `STOPPED-NOT-DONE`. Use `--unread` for the review queue; it retains missing reports and reports newer than their read mark. Exit `0` means success; exit `1` means usage, I/O, or run-resolution failure.

## Report

```text
herdr-agent-team report <worker> [--run <dir>] [--head N]
```

Without `--head`, print the absolute report path. With `--head N`, print at most `N` lines. A successful read persists that report mtime as its read mark, so read the report only when ready to acknowledge that version. Exit `0` means success; exit `1` means usage, unknown worker, missing report, or I/O failure.

## Msg

```text
herdr-agent-team msg <god|worker|all|a,b> <text> [--attention] [--run <dir>]
```

From the god, target one worker, every worker with `all`, or a comma-separated worker list. Multi-target delivery deduplicates names. Each target retains its launcher readiness gate and outbox discipline: a successful exit can mean safely enqueued for later delivery. Use `--attention` only for the singular `god` target; workers use it to request explicit god attention. Exit `0` means every target was delivered or enqueued; exit `1` means validation, resolution, or delivery failed.

Use the msg verb only. Never brief workers to use raw `herdr agent send`. The msg verb uses `pane run`, honors `queues_midturn`, and puts messages for unsafe busy targets into the outbox until a later status flip drains them.

## Kill one worker

```bash
herdr-agent-team kill "$RUN" --worker <name>
```

Use worker-scoped kill to recover capacity without ending the run. It closes only that plugin-owned workspace, or marks an adopted worker released while leaving its pane intact. It retains the run while other workers remain and refuses dirty-worktree removal under the same salvage rule as team kill. Read or preserve the report and uncommitted work before teardown.

## Board

Open the native run-board control deck through the `open-board` plugin action, or run:

```text
herdr-agent-team board [--run <run-dir>]
```

Use `j`/`k` to select, `m` to message, `g` to acknowledge attention, `K` to kill only the selected worker, `o` to open its `report:` link, `p` to adopt a pane, and `q` to quit. The default pane placement is a durable tab; callers may override it to an overlay. Treat the board as a control deck over run-scoped task, report, and mailbox facts—not as completion truth independent of the inbox.

## Monitor without losing truth

1. Start a bounded wait for the next actionable condition.
2. On wake or timeout, run `inbox --unread` and inspect the run-board.
3. Open reports with `report`; verify claimed commands and artifacts before integration.
4. Send decisions and follow-up work through `msg`.
5. Repeat with a fresh bounded wait until required reports exist.

A pointer injection is an at-most-once notification that names the report path. It is not a completion guarantee: a status flip may inject a pointer before a report exists, or notification delivery may fail after the report is safely written. Reconcile every pointer against the inbox.

## Triage failures

| Symptom | Meaning | Action |
|---|---|---|
| Worker failed while its pane looks idle and capacity remains occupied | Failure-idle capacity crash | Check `inbox`, preserve worktree evidence, use worker-scoped `kill`, then reassign the bounded remainder. Do not wait on presentation state. |
| `STOPPED-NOT-DONE` | Worker is idle/done or terminal without its durable report | Message the worker if reachable; inspect its pane/worktree; require the report before accepting completion. If failed/orphaned, salvage evidence and reassign. |
| Run has pending workers and only part of the team launched | Half-spawned team | Keep the existing run dir and run `spawn --resume "$RUN"`; do not duplicate live workers or create a replacement run. |
| Pane, prompt, and Git output disagree about directory or branch | Split-brain cwd | Stop mutations. Compare the run-board worktree path, pane cwd, `pwd`, and `git status --short --branch`. Recreate the worker with cwd set at pane creation; never patch it with prompt-level `cd`. |

## Integrate and release

1. Review every report and diff. Merge the smallest, lowest-dependency changes first; rebase or refresh later branches against the new integration point when required.
2. Keep worker branches isolated. The god owns integration; workers never merge or tag.
3. Run the repository's full central gate once on the assembled result. Do not treat per-worker narrow verification as the release gate.
4. The coordinator alone bumps manifest/package versions when the integrated behavior requires it.
5. Release, tag, merge to a release-bearing default branch, or publish only on the human's explicit word. Until then, leave the work ready for review.
