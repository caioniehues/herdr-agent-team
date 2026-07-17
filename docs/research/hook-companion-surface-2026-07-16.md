# Hook-companion surface — verified facts (issue #91)

Evidence hierarchy applied: live probes (this machine) > binary strings (`claude`
2.1.211, `~/.local/share/claude/versions/2.1.211`) > official docs
(`code.claude.com/docs/en/hooks`, `.../en/agent-teams`, fetched 2026-07-16) >
training data (not used for any claim below).

## 1. Verified hook-event inventory (full list, not just the three team events)

Source: `code.claude.com/docs/en/hooks` (fetched live 2026-07-16).

| Event | Fires when | Team-relevant |
|---|---|---|
| `SessionStart` / `Setup` / `SessionEnd` | session begins/resumes / `--init(-only)`/`--maintenance` / session ends | no |
| `UserPromptSubmit` / `UserPromptExpansion` | prompt submitted / skill-command expands | no |
| `Stop` / `StopFailure` | Claude finishes responding / turn ends on API error | no |
| `PreToolUse` / `PermissionRequest` / `PermissionDenied` / `PostToolUse` / `PostToolUseFailure` / `PostToolBatch` | tool-call lifecycle | indirectly (teammates run tools too) |
| **`SubagentStart` / `SubagentStop`** | a **subagent** (not a teammate) spawns/finishes | no — subagents ≠ teammates |
| **`TeammateIdle`** | an agent-team teammate is about to go idle | **yes** |
| **`TaskCreated`** | a task is being created via `TaskCreate` | **yes** |
| **`TaskCompleted`** | a task is being marked completed | **yes** |
| `FileChanged` / `CwdChanged` / `ConfigChange` / `InstructionsLoaded` | filesystem/config watchers | no |
| `MessageDisplay` / `Notification` | display/notification pipeline | no |
| `PreCompact` / `PostCompact` | context compaction | no |
| `WorktreeCreate` / `WorktreeRemove` | worktree lifecycle | no (relevant to this repo's shim work, not to teams) |
| `Elicitation` / `ElicitationResult` | MCP server requests user input | no |

**There are exactly three team-specific hook events: `TeammateIdle`, `TaskCreated`,
`TaskCompleted`.** No `TeammateSpawn`, `TeammateStop`, `MessageSent`, or
`MessageReceived` event exists in the current inventory — confirmed by the full
table above, cross-checked against the agent-teams doc's "Enforce quality gates
with hooks" section, which lists the same three and no others.

### Payload notes (team events)

The agent-teams doc's compatibility note is the only place a payload field is
documented for these three: pre-v2.1.178 payloads carried a `team_name` field
(now session-derived and **deprecated**) in `TaskCreated`, `TaskCompleted`, and
`TeammateIdle`. The hooks page's detailed per-event JSON schema section for
these three did not resolve in the fetch (page is long / anchor-gated); the
common fields documented as present on **every** hook payload are: `session_id`,
`prompt_id`, `transcript_path`, `cwd`, `permission_mode`, `effort`,
`hook_event_name`, `agent_id` (subagent only), `agent_type`. Treat exact
task/teammate-specific fields (task id, status, teammate name) as **unverified
pending a second fetch pass or a live capture** — see Gaps.

## 2. What exit code 2 actually blocks

Source: `code.claude.com/docs/en/hooks`, "Exit Code 2 Behavior Per Event" table
and "Summary: Exit Code 2 Quick Reference" (live fetch 2026-07-16).

| Event | Exit 2 effect |
|---|---|
| `TeammateIdle` | **Prevents the teammate from going idle** — it keeps working, sees the hook's stderr as feedback |
| `TaskCreated` | **Rolls back the task creation**, feedback shown |
| `TaskCompleted` | **Prevents the task being marked complete**, feedback shown |
| `PreToolUse` | Blocks the tool call |
| `PermissionRequest` | Denies the permission |
| `Stop` / `SubagentStop` | Prevents stopping, conversation continues |
| `PostToolUse` / `PostToolUseFailure` | **No block** — tool already ran/failed; stderr shown to Claude only |

Global rule stated in the doc: **only exit code 2 blocks anything; exit code 1
is a non-blocking error** even though 1 is the conventional Unix failure code.
JSON output (`{"decision":"block","reason":...}` or `{"continue":false,
"stopReason":...}`) is the structured alternative to raw exit codes and is
explicitly supported for `TaskCreated`/`TaskCompleted`/`Stop`/`SubagentStop`
per the "Top-Level `decision`" section of the same doc.

**Can a hook block SendMessage sends or teammate spawn?** No — confirmed by
absence: there is no `MessageSent`/`MessageReceived`/`TeammateSpawn` event in
the inventory (section 1), so there is no exit-2 hook point for either. **Can a
hook block plan approval?** No — the agent-teams doc states plan approval
decisions are made "autonomously" by the lead session itself, not gated by any
documented hook event; no `PlanApproval`-named event appears in the inventory.

## 3. Hook configuration surface

Source: `code.claude.com/docs/en/hooks` (live fetch).

| Location | Scope | Shareable |
|---|---|---|
| `~/.claude/settings.json` | all your projects | no (machine-local) |
| `.claude/settings.json` | one project | yes, committable |
| `.claude/settings.local.json` | one project | no, gitignored |
| Managed policy settings | org-wide | yes, admin-controlled, highest priority |
| **Plugin `hooks/hooks.json`** | while plugin enabled | yes, bundled with plugin |
| Skill/agent frontmatter | while component active | yes |

**Plugins CAN ship hooks — confirmed, yes.** Citation: the doc's "Plugin-Bundled
Hooks" example shows a full `hooks/hooks.json` manifest (`{"description":...,
"hooks":{"PostToolUse":[{"matcher":"Write|Edit","hooks":[{"type":"command",
"command":"${CLAUDE_PLUGIN_ROOT}/scripts/format.sh",...}]}]}}`), and states
plugin hooks "merge with your user and project hooks" using the same format,
with `${CLAUDE_PLUGIN_ROOT}` resolving to the plugin's bundled path. An
enterprise admin can neutralize this via `allowManagedHooksOnly`, which "block[s]
user, project, and plugin hooks" — except hooks from plugins force-enabled
through managed `enabledPlugins`, which stay exempt (so orgs can distribute
vetted hooks through a managed marketplace).

Matcher syntax (for events that support it — `TeammateIdle`/`TaskCreated`/
`TaskCompleted` do **not**, per the doc's "No matcher support" row, so they
always fire unconditionally): exact string or `|`/`,`-separated list for
plain-token patterns, else unanchored JS regex (`RegExp.prototype.test`).

## 4. What hooks canNOT gate — explicit boundary

1. **Teammate spawn.** No hook event fires when a teammate is created. A
   companion cannot veto or observe "teammate about to be spawned" — only
   after-the-fact state (task list, mailbox, `TeammateIdle`) is visible.
2. **SendMessage sends/deliveries.** No `MessageSent`/`MessageReceived` event
   exists. A hook cannot intercept, delay, or reject an inter-teammate message.
3. **Plan approval decisions.** The lead approves/rejects teammate plans
   autonomously; no hook event exposes this decision point.
4. **Non-lead teammate's own hook config in split-pane/tmux mode is a
   separate process** with its own settings resolution — a hook configured in
   the lead's `~/.claude/settings.json` does still apply per-session (hooks are
   evaluated per Claude Code process, and each teammate is a full independent
   session per the agent-teams doc), but a companion cannot assume a single
   hook install governs the whole team without confirming every teammate
   process picked it up.
5. **Task claiming's file-locking mechanism itself.** The doc states "task
   claiming uses file locking to prevent race conditions," but no hook fires on
   claim/lock-acquisition — only on create/complete.
6. **Direct mailbox writes.** Nothing hooks a raw filesystem write to
   `inboxes/{agent}.json` — see §5, this is a live file-mutation surface, not a
   tool-call surface, so `PreToolUse`/`PostToolUse` never see it.
7. **Anything before the experimental flag is set.** With
   `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS` unset, "no team is set up at session
   start, no team directories are written" — team hooks are simply inert, not
   silently degraded.

## 5. Inbox-write safety verdict

### Documented behavior (agent-teams doc, live fetch)

> "Each agent's mailbox is a JSON file at
> `~/.claude/teams/{team-name}/inboxes/{agent-name}.json`. Claude Code
> validates every entry when it reads a mailbox file. Entries that don't match
> the message format are reported as errors and removed from the file; the
> valid messages are still delivered. **Before v2.1.207, a single malformed
> mailbox entry caused a repeated error every second and blocked delivery for
> that mailbox until you deleted the file manually.**"

This is the load-bearing fact: pre-2.1.207, a malformed write **could hang
delivery indefinitely**, requiring manual file deletion. Current version
installed here is 2.1.211, so the read-side self-healing (strip-and-continue)
applies, but this confirms mailbox files are schema-fragile enough that the
product team had to ship a specific fix for malformed-entry handling.

The doc does **not** document any write-side locking/versioning contract for
external writers — locking is only explicitly claimed for **task claiming**
("uses file locking to prevent race conditions"), not for mailbox writes.

### Binary corroboration (claude 2.1.211, `strings` probe)

Confirmed present: `mailbox_write_failed` (exact string, 1 occurrence),
`TeammateMailbox`, `readMailbox`, `clearMailbox`, `pruneInvalidMailboxEntries`,
`getInboxPath`, `InboxPoller`.

Found and decompiled a mailbox-mutation function (name obfuscated to `w6r`)
that Claude's own process uses to prune/write mailbox state:

```js
async function w6r(e,t,r){
  let n=_xt(e,r), o=`${n}.lock`, i;
  try{
    i=await Fb(n,{lockfilePath:o,...uZt});      // acquire {inboxPath}.lock
    let s=await bWe(e,r);                        // read current entries
    if(s.length===0) return;
    let a=s.filter(l=>!l.read&&!t(l));            // filter (e.g. drop read/matched)
    await os().atomicWrite(n, Le(a,null,2));       // atomic write-back
  } catch(s){ if(zt(s)==="ENOENT") return; xe(s) }
  finally{ if(i) try{await i()}catch{} }
}
```

This is exactly the discipline an external writer must replicate: **acquire a
sidecar `.lock` file** (the `Fb(n,{lockfilePath:o,...})` call shape matches the
`proper-lockfile`-style API), **read-modify-write**, and **write back via an
atomic replace** (`atomicWrite`, i.e. temp-file-then-rename, not an in-place
truncate). Claude's own process does not do naive appends.

### Live-file evidence — and a load-bearing negative finding

Checked all 12 live team directories on this machine
(`~/.claude/teams/session-*/`): **every one contains only `config.json`; none
has an `inboxes/` subdirectory.** All members in the sampled config
(`~/.claude/teams/session-7ee975f4/config.json`) have `"backendType":
"in-process"`. Cross-referenced against the agent-teams doc's default
(`teammateMode` default is `"in-process"`, split-pane requires tmux/iTerm2):
**this strongly suggests mailbox JSON files are only materialized for
split-pane/tmux-or-iTerm2-backend teammates, not for in-process teammates**,
which are presumably message-passed through in-memory/IPC rather than a
polled file. This machine has never run split-pane mode in an active team
this session, so the mailbox file format itself could not be directly
observed — only inferred from the binary and docs.

### Verdict

**Conditionally legitimate, not safe by default.** An external process MAY
write into a teammate's inbox file, but only under the exact discipline
Claude's own code follows: acquire `{inboxPath}.lock` before mutating, read
current contents, write back via atomic replace (never append/truncate
in-place), and conform exactly to Claude Code's validated message-entry
schema (unverified exact shape — see Gaps) since malformed entries are
auto-stripped on next read (self-healing in 2.1.211+, but still lossy — the
malformed message never gets delivered) and were fully self-destructive
pre-2.1.207. It is also **backend-dependent**: this rung of the
mission-control design (inbox-write steering) may be a no-op for the
default in-process teammate mode, since no inbox file exists to write to in
that mode on this machine — this must be re-verified against a live
split-pane team before the spec assumes inbox-write steering works
universally.

## Open gaps

1. Exact JSON schema for a mailbox message entry (sender, type, read/unread
   flag, timestamp, idle-notification shape) — not found in docs fetch, no
   live `inboxes/*.json` file existed to sample. Needs either a live
   split-pane team capture or a deeper binary string pull.
2. Exact per-field payload schema for `TeammateIdle`/`TaskCreated`/
   `TaskCompleted` hook JSON stdin — the hooks doc's per-event schema section
   did not resolve in this fetch pass; only the deprecated `team_name` field
   is confirmed via the agent-teams doc's compatibility note.
3. Whether `.lock` files under `inboxes/` (if/when they exist) are
   `proper-lockfile`-compatible (stale-lock timeout, PID+hostname compromise
   detection) — inferred from call shape (`lockfilePath` option) but the
   locking library itself was not identified by name in the strings dump.
4. Whether task-file locking (`~/.claude/tasks/{team}/*.lock`, confirmed to
   exist as a concept in the agent-teams doc) uses the same lock primitive as
   mailbox writes — not cross-checked.

## Findings file

`/home/caio/Projects/herdr-agent-team/docs/research/hook-companion-surface-2026-07-16.md`
(this file).

## Coordinator verification addendum (2026-07-16, same day)

Independent spot-checks by the coordinating session:

- **The in-process-no-mailbox inference is REFUTED by counterexample.**
  This coordinator session's own live team
  (`~/.claude/teams/session-7ee975f4/`) has ALL members at
  `backendType: "in-process"` AND a real
  `inboxes/team-lead.json` (JSON array, observed drained to `[]`
  seconds after a delivery). Mailbox files DO materialize for
  in-process teams — they are transient (drained on read), which is
  why sweeps of idle/stale teams find none. Inbox-write steering is
  therefore NOT mode-gated; the real constraint is the write
  discipline (sidecar `.lock` + read-filter-atomicWrite) plus the
  race against drain.
- **Binary corroboration verified** (the schema report's skipped
  step, closed here): `~/.local/share/claude/versions/2.1.211` is
  itself the flat executable (>20 MB file, not a directory).
  Confirmed strings: `TeammateIdle` ×33, `TaskCreated` ×28,
  `TaskCompleted` ×37, `TeammateMailbox` ×83,
  `mailbox_write_failed`, `pruneInvalidMailboxEntries`,
  `getInboxPath`.
- Open gap #1 (mailbox entry schema) stays open but is now known to
  be capturable on THIS machine with a fast poll during live team
  traffic — no split-pane team required. Fold into the #89 E2E.
