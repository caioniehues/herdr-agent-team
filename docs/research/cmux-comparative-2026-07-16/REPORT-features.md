# REPORT-cmux-features.md — cmux product survey (idea bank for herdmates)

Generated: 2026-07-16
Scope: manaflow-ai/cmux as a PRODUCT (features, UX/UI, integrations) —
architecture already covered in BRIEF 2 / REPORT-cmux.md.

---

## 1. Feature catalog

Herdmates-surface legend: **D1** sidebar · **D2** TUI board · **D3** focus
pane · **shim** · **launcher** · **NEW** = a new idea not yet in scope ·
**herdr-ask** = request to herdr upstream.

| Feature | What it does for the user | Evidence | Verdict | Herdmates surface |
|---|---|---|---|---|
| Claude Code Teams launcher | `cmux claude-teams` — one command, teammates become native splits, "no tmux required" (nightly-only!) | [doc] README, en.json claudeCodeTeams | **STEAL** — this is literally ADR-0012's goal, already proven in production | shim / launcher |
| 4x third-party multi-agent shim reuse (omo/omx/omc/omp) | Same `__tmux-compat` shim genericized across OpenCode+plugin, OMX, OMC, and a hooks-based path for omp | [doc] en.json integration pages | **STEAL** — build the shim against "any tmux-panes agent," not Claude-only, from day one | shim |
| Notification rings + panel | Blue ring on pane + tab flash when an agent needs attention; central panel, jump to newest unread | [doc] README | **STEAL** — cheap, high-value attention-routing primitive | D1 / D3 |
| Cmd+Shift+U (jump to newest finished agent) | One shortcut: switch workspace, focus exact pane, flash it, mark read, cross-window focus-steal if needed | [doc] blog excerpt en.json | **STEAL** — near-zero cost, huge quality-of-life at fleet scale (their own example: 17 concurrent workspaces) | D3 / NEW |
| Feed / "Vibe Island" unified approval queue | Cross-agent queue for Permission requests / ExitPlanMode / AskUserQuestion, keyboard-first, semaphore-blocked (up to 120s) per request_id, ring-buffer + JSONL audit | [source] docs/feed.md | **STEAL** — directly maps to a focus-pane primitive; the request_id-keyed blocking-wait mechanism is a concrete, reusable pattern | D3 |
| Sidebar live-data schema (workspaces/tabs/clock) | Rich typed fields per workspace: branch, dirty, pr{}, prs[], progress{value,label}, latestMessage/Prompt/At, remote{} | [source] docs/custom-sidebars.md | **STEAL (schema, not tech)** — adopt the field list as a target schema for herdr's `pane report-metadata`, richer than current free-form tokens | D1 |
| Custom Sidebars (vibe-code your own) | User/agent-authored SwiftUI-DSL sidebar, hot-reload, in-process or sandboxed remote renderer with an eval budget | [source] docs/custom-sidebars.md | **ADAPT** — the SwiftUI-specific tech doesn't port, but "let users/agents author their own status view against a live-data API, with a safety-budgeted sandbox lane" is a portable idea | NEW |
| `status-board.swift` example | Groups workspaces by live signal (urgent bugs/review/progress/research/done) — a kanban board built AS a sidebar recipe, not a core feature | [source] docs/custom-sidebars.md | **STEAL (concept)** — cmux achieves "task board" via a sidebar recipe rather than a first-class feature; consider whether D2 TUI board should similarly be "a Feed/sidebar view," not a separate subsystem | D2 |
| Task Manager (`cmux top`) | CPU/RAM per agent process, mapped to owning workspace/pane, for troubleshooting stuck/slow agents | [doc] en.json taskManager | **ADAPT** — genuinely useful but NOT a project-task board (name collision risk with herdmates' D2 concept — see synthesis) | NEW |
| Dock (persistent right panel) | Same split/surface system as main area, but a separate per-window auxiliary panel for git views/logs/dashboards/dev servers, config-seeded (`dock.json`) | [source] docs/dock.md | **ADAPT** — a general "auxiliary panel, same primitives as main area" pattern worth considering distinct from D1/D2/D3 | NEW |
| Session restore (layout+cwd+scrollback+browser) | Full app-state restore on relaunch, explicitly NOT checkpointing arbitrary process state | [doc] en.json sessionRestore | **STEAL** — the "restore app-owned metadata, not process memory" boundary is the right scope | NEW |
| Per-agent resume hook-event adapter table | Different hook/event vocabulary per agent CLI (Claude `PermissionRequest`, Codex `PreToolUse,PermissionRequest`, OpenCode `beforeShellExecution`, Hermes own naming, some via "plugin event bus") | [doc] en.json sessionRestore supported-agents table | **STEAL (pattern)** — if herdmates ever supports non-Claude agents, budget for a per-agent adapter table, not a single unified event name | shim |
| Resume-command trust model | Secrets stripped before storing resume bindings; auto-run requires signed/approved trusted prefix; untrusted bindings stored for manual restore only | [doc] en.json sessionRestore | **STEAL** — directly applicable security boundary for any herdmates session-resume feature | NEW |
| Remote daemon (cmuxd-remote) | SHA-256-verified binary on remote host; 3 features: SOCKS5/HTTP-CONNECT browser proxy over daemon stdio, HMAC-authed reverse-TCP CLI relay (remote processes call back into local cmux socket), PTY-resize-aware session persistence | [doc] en.json daemon section | **ADAPT** — relevant only if herdmates goes SSH/remote; the reverse-tunnel CLI-relay pattern is the most portable part | NEW |
| Deeplink URL scheme | `cmux://` style links for SSH/Prompt open flows, with install-fallback UX and structured param-validation errors | [doc] en.json deeplink section | **SKIP** (low priority, macOS-app-specific affordance) | — |
| Keyboard model: tmux-style prefix chords | `"newSurface": ["ctrl+b", "c"]` — explicit two-step chord support for tmux muscle-memory migrants | [doc] cmux.com/docs/keyboard-shortcuts | **STEAL** — cheap onboarding win for users coming from tmux | NEW |
| Concepts hierarchy: Window→Workspace→Pane→Surface→Panel | Panes can hold MULTIPLE surfaces (tabs within a split region); each surface has a `CMUX_SURFACE_ID` env var | [doc] cmux.com/docs/concepts | **herdr-upstream-ask** — herdr's pane model (per BRIEF-2 recon) is 1:1 pane↔content; ask whether herdr wants a sub-tab-within-pane concept | herdr-ask |
| Onboarding: pre-seeded first workspace, CLI auto-on-PATH inside app | Zero-config first launch; CLI requires manual symlink only for outside-app use | [doc] cmux.com/docs/getting-started (defuddle) | **STEAL** — low-friction first-run pattern | NEW |
| Update pill in titlebar | Quiet, non-modal "update available" affordance instead of a blocking dialog | [doc] cmux.com/docs/getting-started | **STEAL (small)** — good non-intrusive pattern | NEW |
| Ghostty-config theming inheritance | Reads existing `~/.config/ghostty/config` for themes/fonts/colors — zero-migration theming for existing Ghostty users | [doc] README | **SKIP** (not applicable — herdr isn't Ghostty-based) | — |
| GPL relicensing (AGPL-3.0 → GPL-3.0) | Blog-documented licensing change | [doc] en.json blog metaDescription | not found (no rationale detail read) | — |
| "The Zen of cmux" philosophy | "Primitive, not a solution" — composable pieces, no opinionated workflow | [doc] blog excerpt | **judgment call, not steal/skip** — see synthesis below | — |

**Not found** (checked, absent or not surfaced in available sources):
- A first-class, standalone "project task/todo board" feature under any name
  other than the resource-monitor "Task Manager" — see synthesis point 1.
- Any admitted limitations/caveats section specific to claude-teams (only
  found for the unrelated SSH-tmux-mirroring feature).
- Windows/Linux support — README and requirements confirm macOS-only.

---

## 2. Top-10 ranked: pains to avoid + things worth copying

*(synthesis / judgment — not a direct source claim)*

1. **Copy: the Feed pattern (unified, blocking, cross-agent approval queue).**
   Of everything surveyed, this is the highest-value steal. A single
   request_id-keyed semaphore queue for permission/plan/question moments,
   surfaced identically whether the agent is Claude, Codex, or OpenCode,
   solves exactly the "many agents, one human" attention problem herdmates
   exists to address.
2. **Copy: Cmd+Shift+U-style "next thing needing me" navigation.** Trivially
   cheap relative to its payoff at fleet scale; pairs naturally with #1.
3. **Avoid: naming collision between "task manager" (resource monitor) and
   a project/task board.** If herdmates ships a D2 "TUI board," do not call
   it a task manager — cmux has already claimed that name for a different
   concept, and using it will create confusing comparisons/expectations for
   anyone who's used cmux.
4. **Copy: the sidebar's typed live-data schema** (branch/dirty/pr/progress/
   latestMessage etc.) as a target shape for herdr's pane metadata, replacing
   free-form tokens with a known field set that a status board can render
   without per-integration guesswork.
5. **Avoid shipping the tmux-shim as Claude-specific.** cmux's biggest
   architectural lesson (confirmed independently in BRIEF 2) is generalizing
   the shim against "any agent CLI that manages panes via tmux," which
   immediately paid off across 4 other tools without new shim code. Build
   herdmates' shim the same way from the start.
6. **Copy: the resume-command trust/security model** (strip secrets before
   persisting, require signed/approved prefixes to auto-run, default to
   manual restore for anything untrusted). Directly reusable if herdmates
   ever persists/replays launch commands.
7. **Note as a real risk, not just a design choice: claude-teams is
   nightly-only in cmux**, and their own CHANGELOG shows a "mutual exec
   loop" bug in the Claude shim. A mature team with a multi-year head start
   still hasn't stabilized this to their release channel — budget real time
   for shim edge cases, don't assume the recon's clean 36-line log
   (REPORT.md, BRIEF 1) means production-readiness.
8. **Consider (not clearly steal or skip): "primitive, not a solution."**
   cmux deliberately ships unopinionated building blocks (custom sidebars,
   Dock config, CLI/socket API) rather than a fixed board layout. Herdmates'
   D1/D2/D3 model is more prescriptive by design — that's a legitimate
   different bet (curated experience vs. raw primitives), but worth an
   explicit decision rather than drifting into "build everything
   configurable" by accident.
9. **Copy (small, cheap): tmux-style two-step prefix chords in the keyboard
   config**, and a quiet titlebar update-pill instead of a blocking update
   dialog — both near-zero-cost UX polish items with clear precedent.
10. **Ask herdr upstream: does a pane need to host multiple surfaces
    (tabs-within-a-pane)?** cmux's 5-level hierarchy (Window > Workspace >
    Pane > Surface > Panel) has a tab-within-split-region concept herdr's
    pane model currently lacks per BRIEF-2's recon. Worth a scoped ask
    rather than silently deciding herdmates doesn't need it.

---

## 3. Method record

- ctx7 calls used: 3/3 for this brief —
  1. `docs /websites/cmux "tmux integration..."` — reused from BRIEF 2 context, not recounted here
  2. `docs /websites/cmux "keyboard shortcuts onboarding getting started layouts quick glance"`
  3. `docs /websites/cmux "concepts primitives workspace pane surface mental model"`
- defuddle: 1 page fetched — `cmux.com/docs/getting-started`
- firecrawl-scrape: not needed (no Cloudflare gate encountered)
- Local clone mined (read-only, `git -C cmux-upstream pull` only git op run):
  README.md, CHANGELOG.md (top ~100 lines / release 0.64.18-0.64.19),
  skills.sh, skills/cmux/SKILL.md + directory listing of 20 skill dirs,
  docs/custom-sidebars.md (full), docs/feed.md (partial, architecture
  section), docs/dock.md (partial), docs/notifications.md (partial),
  web/messages/en.json (targeted greps: daemon, claudeCodeTeams, 4x
  oh-my-* integrations, taskManager, sessionRestore, blog excerpts for
  Cmd+Shift+U and Zen of cmux)
- Not read in full (noted in findings-features.md "not covered"): vault.md,
  workspace-groups.md, textbox doc, browser-automation doc, ios docs,
  data-driven-sidebar-plan.md (skimmed title only)
- All claims tagged [doc]/[source]/[inference] in findings-features.md;
  this report's catalog table omits inline tags for table readability —
  see findings-features.md for the tagged source trail per claim.
