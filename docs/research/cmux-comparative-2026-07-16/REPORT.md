# REPORT-cmux.md — cmux comparative research (ADR-0012 context)

Generated: 2026-07-16

**Official upstream identified:** `github.com/manaflow-ai/cmux` (docs at
`cmux.com/docs`) — a native macOS terminal built on Ghostty, with a
socket-based control API, purpose-built for managing multiple AI coding
agents. **[doc, inference]** ctx7 returned 5 distinct "cmux" hits; cross-
referencing install links in cmux.com's own docs (`brew tap manaflow-ai/cmux`,
`raw.githubusercontent.com/manaflow-ai/cmux/main/skills.sh`) confirms
cmux.com and github.com/manaflow-ai/cmux are the same product (website +
code), not two different projects. `karlorz/cmux` (near-identical README) is
almost certainly a fork, excluded. `soheilhy/cmux` is an unrelated Go
connection-multiplexing library, excluded. Not Caio's local `cmux-kde`.
Shallow-cloned read-only to `cmux-upstream/` (HEAD `25dc913`, 2026-07-16) and
confirmed genuine: Xcode project, Ghostty git submodule, native macOS app
layout.

---

## Q1: Claude Code native agent teams support

**Verdict: YES — explicit, first-class, actively maintained feature.**

- **[doc]** `cmux claude-teams` command launches Claude Code with agent teams
  enabled. Teammates "appear as native cmux splits instead of tmux panes,
  providing full sidebar metadata and notifications."
  (`cmux.com/docs/agent-integrations/claude-code-teams`)
- **[source]** Dedicated Swift package `Packages/macOS/CMUXAgentLaunch/` with
  test files named `ClaudeTeamsLaunchOptionTests.swift`,
  `ClaudeTeamsPolicyIsolationTests.swift`,
  `ClaudeTeamsPromptBoundaryRecoveryTests.swift`,
  `ClaudeTeamsPromptBoundaryRejectTests.swift` — this is not a thin wrapper,
  it's a maintained subsystem with dedicated policy/boundary test coverage.
- **[source]** `CHANGELOG.md`: original launcher PR #1179; since then, at
  least 4 more PRs specifically targeting claude-teams bugs/features
  (#2119 layout fix, #2238 SSH relay support, #6242 restore flags, #6499
  fix for Claude Code 2.1.183 compat) — herdmates tested against 2.1.211,
  only ~28 point-releases ahead of cmux's most recent tracked fix.

## Q2: tmux integration or emulation

**Verdict: YES — full binary-and-environment shim, reused across 5 integrations.**

- **[source]** Exact mechanism, from `web/messages/en.json` (source strings
  for the claude-code-teams doc page):
  1. Creates a tmux shim at `~/.cmuxterm/claude-teams-bin/tmux` that
     redirects to `cmux __tmux-compat`
  2. Sets `TMUX` and `TMUX_PANE` environment variables to simulate a tmux
     session
  3. Prepends the shim directory to `PATH` so Claude finds the shim before
     real tmux
  4. Enables `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1` and sets teammate mode
     to `auto`
- **[doc]** Env var semantics: `TMUX` = "fake tmux socket path encoding the
  current cmux workspace and pane"; `TMUX_PANE` = "fake tmux pane identifier
  mapped to the current cmux pane"; a socket-path var points the shim at
  cmux's control socket.
- **[source]** `CLI/CMUXCLI+TmuxCompatSupport.swift` (426 lines) implements
  the actual translation: geometry math for tmux format-string queries
  (`pane_width`, `pane_height`, `pane_left`, `pane_top`, `window_width`,
  `window_height`), `tmuxRespawnStartCommand`/`tmuxClaudeTeamsRespawnEnvironment`
  for `respawn-pane -k`, `tmuxSplitSizeCells` for split-size parsing.
- **Key difference from herdmates' own recon (this workspace, BRIEF.md /
  REPORT.md):** herdmates' tested shim only intercepted the `tmux` binary and
  let Claude Code talk to a REAL tmux session underneath (real `$TMUX` value).
  cmux fakes **both** the binary **and** the `TMUX`/`TMUX_PANE` env vars —
  there is no real tmux session anywhere in the loop. This is a materially
  different (more decoupled) architecture, and it is reused verbatim for four
  other agent CLIs (oh-my-opencode, omo, omx, omc), each with their own
  `~/.cmuxterm/<tool>-bin/tmux` shim pointing at the same `__tmux-compat`
  dispatcher — confirming this is cmux's general strategy for any tmux-based
  multi-pane agent display, not a one-off Claude Code hack.

## Q3: Programmatic pane spawning by child processes

**Verdict: Unix-socket JSON-RPC-style control API + CLI, with env-var socket discovery.**

- **[doc]** Socket protocol example: `{"id":"split-new","method":"surface.split","params":{"direction":"right"}}`
  (`cmux.com/docs/api`)
- **[doc]** `sidebar-state` is exposed both as CLI (`cmux sidebar-state
  [--workspace ID]`) and as a socket method (`sidebar_state --tab=<uuid>`).
- **[doc/source]** Socket path is discovered via an env var (the shim doc
  calls it "the cmux control socket" var, injected alongside `TMUX`/
  `TMUX_PANE`) — child processes/shims read this to find the socket, then
  speak JSON over it. This is the same discovery pattern herdr uses (a CLI
  that talks to a local server), but cmux additionally exposes the raw socket
  protocol directly rather than requiring shell-out to a CLI binary for every
  call.

## Q4: Agent/task board or sidebar-style status surfaces

**Verdict: YES — "sidebar" is a first-class, actively-designed subsystem with rich per-tab metadata.**

- **[doc]** `sidebar-state` dumps, per workspace/tab: cwd, git branch, ports,
  **status pills**, **progress bars**, log entries.
- **[source]** `docs/custom-sidebars.md` and `docs/data-driven-sidebar-plan.md`
  exist in the repo, confirming the sidebar is a designed subsystem, not a
  retrofit. Source-level sidebar UI code was not read in depth (out of this
  recon's time budget) — the doc-level API answer is sufficient to confirm
  the feature exists and its shape.
- Compared to herdr's current `pane report-metadata` (which supports
  `--state-label`, free-form `--token key=value`), cmux's sidebar model
  bundles git branch + ports + progress bars as **named, typed fields**
  rather than requiring the caller to synthesize them via generic tokens.

---

## Comparison to herdmates (5 sentences)

cmux already ships, in production, almost exactly what herdmates' ADR-0012
spike is scoping: a fake-tmux shim that intercepts Claude Code's native
agent-teams tmux calls and translates them into its own native pane-management
API, complete with dedicated test coverage and multiple point-release bug
fixes tracking Claude Code's own version churn. The key architectural
difference is that cmux fakes `TMUX`/`TMUX_PANE` env vars in addition to the
binary — decoupling entirely from real tmux — whereas herdmates' spike so far
only intercepted the binary while a real tmux session ran underneath; this is
worth adopting, since it removes real tmux as a runtime dependency and lets
the fake pane IDs be whatever the herdr-mapping layer wants them to be, rather
than needing to shadow real tmux's ID allocation. The single biggest thing
worth stealing is `tmuxEnrichContextWithGeometry` in
`CLI/CMUXCLI+TmuxCompatSupport.swift` — a from-scratch answer to tmux format-
string geometry queries (`#{pane_width}`, `#{pane_height}`, etc.) computed
from the host app's real pixel/cell geometry, which is a gap herdmates' own
REPORT.md flagged as unaddressed. Also worth stealing: cmux's `__tmux-compat`
single-dispatcher-subcommand pattern, reused identically across five different
agent CLIs, suggesting the shim-verb-translation logic should be built as one
reusable core rather than one-off per integration. Nothing in cmux's public
docs or source admits limitations specific to claude-teams (the one
"Limitations" section found is for an unrelated SSH-tmux-mirroring feature),
so no known failure modes to pre-empt were surfaced — absence of documented
limitations is not evidence of none existing, just nothing to cite.

---

## Method record

- ctx7 calls used: 3/3 allowed (`library cmux "..."`, `docs /websites/cmux
  "github repository..."`, `docs /websites/cmux "tmux integration pane spawn
  socket API teammate Claude Code"`)
- Clone: `git clone --depth 1 https://github.com/manaflow-ai/cmux.git` into
  `/home/caio/Projects/herdmates-spike-recon/cmux-upstream` (read-only,
  no further git operations performed)
- No modifications made to cmux-upstream/ or any file outside this workspace
