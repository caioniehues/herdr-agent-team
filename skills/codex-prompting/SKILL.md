---
name: codex-prompting
description: Drive Codex pane workers from a god session. Use when writing Codex briefs, choosing Codex launcher flags or models, sending mid-turn follow-ups, or translating a Claude skill-dependent task for a Codex worker.
---

# Codex worker prompting

Treat Codex as its own launcher, not as Claude Code with a different binary.
Write a self-contained brief, preserve the repository-authored `AGENTS.md`, and
pass the generated worker protocol by absolute-path pointer injection.

## Discover the live command surface

Run `codex --version` and `codex --help` before relying on a release-specific
flag. In an interactive Codex pane, type `/` without submitting; read the popup
as the live slash-command authority. Recheck after every Codex upgrade.

Live-verified against `codex-cli 0.144.4` on 2026-07-15:

```text
/model /fast /ide /permissions /keymap /vim /experimental /approve
/memories /skills /import /hooks /review /rename /new /archive /delete
/resume /fork /init /compact /plan /goal /agent /side /copy /raw /diff
/mention /status /usage /title /statusline /theme /pets /mcp /plugins
/logout /exit /feedback /ps /stop /clear /personality /subagents
```

The popup is contextual: feature-gated or unavailable commands may be hidden.
Do not copy a list from another Codex release into a brief without probing the
target pane.

## Write Codex-native briefs

- Put durable project rules in repository-authored `AGENTS.md`. Codex loads
  applicable `AGENTS.md` files natively from the project hierarchy.
- Keep the generated worker protocol separate. Inject its absolute path and the
  brief's absolute path; do not overwrite or synthesize repository
  `AGENTS.md`.
- Invoke loaded skills with `$<skill-name>`. Type `$` in the composer without
  submitting to open Codex's live skill/template picker; select from the names
  shown there. `$ask-matt <question>`, for example, loads and follows the Ask
  Matt skill `[live 2026-07-15, codex-cli 0.144.4]`.
- Never translate that invocation to Claude-style slash syntax. `/ask-matt`
  fails with `Unrecognized command`; `/` opens Codex's own command surface,
  while `$` opens its skill/template surface.
- Route skill-dependent tasks to Codex with `$<skill-name>` when the required
  skill appears in its picker. Inline the relevant procedure, checklist,
  template, and acceptance rules only as optional hardening or when the skill
  is unavailable. Route to a Claude worker when the task specifically requires
  a skill that only Claude loads. Claude pane workers also load user-level
  Claude skills `[live 2026-07-15]`.
- Specify owned files, exclusions, exact verification commands, Git contract,
  report path, and completion sentinel. Assume no implied workflow from a
  coordinator-side skill name.

Codex's `$` skill invocation and `/` command invocation are separate surfaces.
Probe both in the target pane after a Codex upgrade; never assume a skill name
is also a slash command.

## Submit and follow up safely

Use one `herdr pane run` per prompt. It sends the text and Enter as one request;
never split paste and submit. Keep `herdr agent wait --status working` as the
submission check required by the launcher policy.

Codex queues a mid-turn `pane run` as a separate pending user turn and submits
it after the active turn finishes `[live 2026-07-15]`. Set
`queues_midturn = true` only for a Codex version verified this way. Send worker
messages through the msg verb; never use raw `herdr agent send` as a messaging
channel.

## Select models and permissions explicitly

Set the launch model with `codex --model <model>` (or `-m <model>`). Change it
interactively with `/model`. State reasoning or behavior requirements in the
brief; do not paste Claude-only model directives and assume Codex interprets
them.

`--dangerously-bypass-approvals-and-sandbox` disables both confirmation prompts
and Codex sandboxing. It is not merely an automatic-approval flag. Use it only
when the worker already runs inside an externally enforced sandbox and the
team explicitly accepts unrestricted execution. The default Codex sandbox can
deny the Herdr socket used by worker-to-god msg; choose the permissive launcher
trade-off deliberately rather than weakening every worker by default.
