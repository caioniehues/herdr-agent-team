# cmux × limux × herdr — three-way comparison (design input for herdmates)

Generated: 2026-07-16.
Companion to `cmux-comparative-2026-07-16/REPORT.md` (upstream architecture) and
`REPORT-features.md` (cmux feature catalog with STEAL verdicts). This report covers what
those don't: **the limux axis** (cmux-vs-limux feature diff, limux's tmux-compat and
extensibility state), the **verified mechanism of Claude Code agent teams** (official docs
+ local binary probes), and a **three-way "custom panes" comparison** including herdr.

Method: three parallel research agents (cmux docs crawl — all 24 pages at cmux.com/docs;
limux repo analysis at `~/Projects/cmux-kde/limux`; herdr API-schema + installed-plugin
analysis), plus direct string/argv probes of the locally installed `claude` binary
(v2.1.211) and spot-checked file:line anchors. Evidence tags: [doc] official docs,
[code] repo anchor, [bin] claude-binary probe, [spike] this repo's
`spike-tmux-verbs-2026-07-16/REPORT.md`.

---

## 1. Ground truth: how Claude Code agent teams actually works

Verified against https://code.claude.com/docs/en/agent-teams (current through v2.1.207)
and the installed claude 2.1.211 binary. This section supersedes the control-mode caveat
in ADR-0012 §"teammux" — see §1.3.

### 1.1 Two independent layers

**Coordination layer (always on, terminal-independent).** Teammates are full,
independent Claude Code instances with their own context windows. They coordinate via:

- Mailboxes: `~/.claude/teams/{team-name}/inboxes/{agent-name}.json` [doc]
- Task files with locking: `~/.claude/tasks/{team-name}/` ("uses file locking to prevent
  race conditions") [doc]
- Team config: `~/.claude/teams/{team-name}/config.json` (members, session IDs, tmux pane
  IDs as runtime state) [doc]
- The native `SendMessage` tool — "always available to a teammate" [doc]; corroborated by
  `SendMessageTool`, `inboxes`, `mailbox_write_failed`, `team/` strings in the binary [bin]

**Display layer (`teammateMode`, settings-file key only — no CLI flag).** Values:
`in-process` (DEFAULT since v2.1.179 — agent panel inside the main terminal, works
anywhere), `auto`, `tmux`, `iterm2`. Split-pane mode is hardcoded to tmux + iTerm2
backends; docs explicitly list split panes as **unsupported in Ghostty**, VS Code
terminal, and Windows Terminal [doc]. Precedent that the backend list extends: iTerm2
was added in v2.1.186.

**Consequence:** native team coordination (mailboxes + SendMessage + task list) works
today inside a herdr pane, a limux surface, or anything else — just export
`CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1`. A tmux shim buys exactly one thing: the
**pane-per-teammate view** (visibility/steerability), never coordination.

### 1.2 The tmux contract (what a shim must implement)

Claude Code probes `$TMUX` / `$TMUX_PANE` and PATH-resolves `tmux` [bin] — it binds to
tmux's CLI surface, not to any API. The observed teammate spawn call [bin]:

```
tmux split-window -d -t <pane> -h -l 70% -P -F '#{pane_id}' -- <teammate-cmd>
```

Verbs present in the binary: split-window, send-keys, capture-pane, select-pane,
new-session, new-window, kill-pane, list-panes (select-window/kill-window absent) [bin].
This matches the 36-call argv capture in [spike] — all discrete CLI invocations.

### 1.3 Kill-signal cleared (ADR-0012 correction)

ADR-0012 gated teammux on the risk that Claude Code uses tmux control mode (`-C`,
persistent bidirectional protocol). The spike **already ran and cleared it**: no `-C`/
`-CC`, no hooks, no wait-for anywhere in the captured lifecycle [spike]; independently,
no control-mode invocation pattern was found in the binary [bin]. The shim is a go on
argv translation alone. The hardest real work is the `%N` ↔ herdr-pane-id translation
table (herdr's `pane split` returns the new id synchronously — the `split-window -P`
semantics needed).

---

## 2. cmux feature inventory (condensed) and the cmux-vs-limux diff

Full inventory lives in the agent transcript; condensed here by category, then diffed.
cmux is macOS 14+ only, built on Ghostty, DMG/Homebrew, Sparkle auto-update [doc].

### 2.1 What cmux has (by category)

- **Model:** Window → Workspace → Pane → Surface (tab) → Panel (Terminal | Browser).
- **Workspace Groups:** named collapsible sidebar sections, icon/color, anchor, full CLI
  (`cmux workspace-group` — 16 subcommands).
- **Session Restore:** layout + cwd + best-effort scrollback + browser history; agent
  sessions resume via native IDs (`cmux hooks setup`, 10+ agents); versioned JSON
  snapshots; History menu.
- **Task Manager** (`cmux top`): per-window/workspace/pane/surface CPU/mem, agent-process
  attribution, stop/restart/split actions.
- **Base:** persistent cloud VM (personal/team), cross-client attach, remote PTYs
  surviving relaunch (`cmux vm ...`, preview).
- **Agent integrations:** `claude-teams` launcher (tmux shim, §1); omc (19-agent Claude
  orchestration), omx (30+ Codex roles + HUD), omo (multi-model OpenCode, shadow config,
  idle auto-cleanup), omp (Pi fork); generic hooks for 10+ agents; Copilot CLI hooks;
  agent-status sidebar pills/spinner; idle-agent hibernation.
- **Control surface:** JSON-RPC Unix socket (`/tmp/cmux.sock`) + `cmux` CLI: workspace/
  split/surface lifecycle, send/send-key, **set-status (icon/color/priority pills),
  set-progress (0.0–1.0 bars), log (5 levels)**, notify + notification shell hooks +
  OSC 777 / OSC 99 protocols, sidebar-state dump, reload-config, `__tmux-compat`.
- **Browser automation:** full Playwright-shaped CLI/API (open/open-split, wait-for,
  click/type/fill, snapshot/screenshot, **eval/JS + script/style injection**, cookies/
  storage, tabs, dialogs, iframes, downloads) — documented and live.
- **Custom Commands:** `.cmux/cmux.json` action registry (stable IDs → behaviors, icons,
  shortcuts) in palette/tab-bar/context menus; multi-pane layout templates; override
  built-ins; per-fingerprint trust for project-local actions.
- **Dock:** declarative JSON sidebar controls, each a Ghostty-backed terminal running a
  command (feeds/logs/dev servers/TUIs); git-shareable; trust-gated.
- **Vault:** search past agent transcripts by content; drag session into workspace.
- **Viewers:** Markdown viewer (live reload), diff viewer, file explorer/editor.
- **Remote:** `cmux ssh` (remote workspaces, browser traffic proxied through remote,
  scp drag-drop, auto-reconnect, deep links); remote tmux mirroring via `tmux -CC`
  (sessions→workspaces, two-way control) — distinct from the teams shim; iOS companion.
- **Extensibility:** CmuxExtensionKit (beta) — see §3.1.

### 2.2 In cmux, NOT in limux (the requested diff)

Limux inventory source: [code] README, full `limux-cli` dispatch (main.rs:3390-3600),
GTK host (window.rs), hooks/, scripts/.

| cmux feature | limux status |
|---|---|
| `claude-teams` launcher + working `__tmux-compat` shim | **Missing the load-bearing half.** limux ported cmux's *secondary long-flag* verb surface (`run_tmux_compat`, main.rs:3109: pipe-pane, wait-for, buffers, swap/break/join-pane…) but NONE of the short-flag core verbs claude-teams drives (split-window/send-keys/select-pane/capture-pane…), no launcher, no PATH/env shim, no key-name table [code] |
| omc / omx / omo / omp launchers | Absent (limux has its own `agent-team` instead — pty-injected `<agent-msg>` XML protocol via generated AGENTS.md; cmux has no analog of that) |
| Sidebar status pills / progress bars / log lines (set-status/set-progress/log) | Absent — limux's only decoration verb is `notification.create` (toast + unread badge) |
| Workspace Groups (+ 16 CLI subcommands) | Absent (favorites/pinning only) |
| Session Restore (scrollback replay, browser history, snapshots, History menu) | Partial — workspace persistence + agent-session hooks (3 agents: codex/claude/gemini; cmux: 10+) |
| Task Manager (`cmux top`) | Absent |
| Base cloud VM | Absent |
| Notification shell hooks + OSC 777 / OSC 99 protocols | Absent (Bell → unread ring only) |
| Custom Commands action registry | Absent |
| Dock (declarative sidebar terminal controls) | Absent |
| Vault (transcript search) | Absent |
| Markdown viewer / diff viewer / file explorer-editor | Absent |
| Remote tmux mirroring (`tmux -CC`) | Absent |
| SSH remote workspaces (+ browser proxy) / iOS app | Absent |
| CmuxExtensionKit sidebar extensions (beta) | Absent (ADR-0007 north star, unbuilt) |
| Browser automation **wired live** | **Present in core, dead in practice**: ~71 `browser.*` methods exist in limux-core (Playwright-shaped, incl. addscript/addinitscript/addstyle) but ZERO are wired into the live GTK bridge, and socket-side browser-pane creation is explicitly rejected (control_bridge.rs:394-410, named regression test) [code] |
| Rebindable shortcuts w/ `when` contexts, command palette, TextBox composer, workspace colors | Absent/fixed (limux: static Ctrl+Alt map, some Cmd remap support) |

**Structural cause, not drift:** the live GTK bridge wires only **19 of ~90** core
Dispatcher methods with a hard-error default and no core fallthrough
(control_bridge.rs:19-36, :675) [code]. Several *already-shipped* limux CLI verbs
(resize-pane, swap/break/join-pane, clear-history…) fail against the real GUI today —
they only pass against the headless core. Any limux capability claim must be checked
against the bridge, not the core.

**In limux, not in cmux:** Linux itself (packaging: deb/AppImage/rpm/tarball/AUR); the
`agent-team` AGENTS.md `<agent-msg>` generator (cmux verified to have no analog).

---

## 3. Custom panes: can cmux / limux be extended, and how does herdr compare?

Three senses, per the design question: (a) plugin-rendered pane types, (b) programmatic
pane control/decoration by external processes, (c) built-in non-terminal content.

### 3.1 cmux

- **(a) Plugin pane types: no — sidebar extensions only, beta.** CmuxExtensionKit
  (changelog-documented only, v0.64.10–.15; no public API reference) renders custom
  *sidebar* views (attention queue, dev-server status…), out-of-process with an isolated
  interpreter, security-scoped to workspace metadata (no terminal buffers/env/secrets).
  Not pane/split content; interface unstable for third parties today. The Dock is
  config-time sidebar extensibility, but each control is a real terminal running a
  command — not rendered by a plugin.
- **(b) Programmatic control: strongest of the three.** Socket/CLI lets any external
  process create/close/focus splits and surfaces, send input, and decorate: status pills
  (icon/color/priority), progress bars, leveled log lines, notifications (+ shell-hook
  filtering, + OSC protocols) — all scoped to workspace/pane/surface IDs.
- **(c) Non-terminal content: real and drivable.** Browser panel is first-class and
  externally scriptable (`cmux browser open-split` … `eval`/script/style injection) —
  i.e. an external process CAN stand up an arbitrary web-rendered UI in a real pane
  today. Markdown viewer is a second non-terminal surface; Vault/Task Manager are
  utility panels, not pane content.

### 3.2 limux

- **(a) Plugin pane types: nothing today; committed direction, unbuilt.** ADR-0007
  (accepted) fixes the mechanism — external-process plugins over a versioned JSON control
  protocol; in-process `.so` and embedded scripting VMs explicitly rejected — but v1
  ships only two seams (event-push channel; formalized pane-type registry), with loader/
  sandbox/discovery/manifest and the UI-injection/theming API all post-v1. A third party
  cannot add a pane type without forking. The Qt/QML port (ADR-0004, phase2 spec) is
  explicitly motivated as the extensibility unlock ("loadable UI plugins and live theming
  … want a declarative UI runtime (QML)") but builds seams, not the plugin system.
- **(b) Programmatic control: narrow.** The 19 live methods cover terminal-pane
  create (command+cwd), workspace lifecycle/rename, read/send text+keys, health, and one
  decoration verb (`notification.create`). No pills/progress/logs/colors/badges/custom
  sidebar entries; no event push (that's the unbuilt ADR-0007 seam).
- **(c) Non-terminal content: exists in GUI, unreachable programmatically.** The
  WebKitGTK browser pane would be a natural poor-man's custom-UI surface (core even
  carries `browser.addscript/addinitscript/addstyle`), but the path is blocked twice:
  socket `pane.create type=browser` is hard-rejected by design, and zero `browser.*`
  methods are bridge-wired [code]. QWebEngineView is the planned Qt-side replacement.

### 3.3 herdr

- **(a) Plugin pane types: yes — but always terminal-rendered.** The manifest
  (`herdr-plugin.toml`) exposes `[[panes]]` (id/title/placement: overlay|popup|split|tab|
  zoomed/command), `[[actions]]`, `[[events]]` (rich vocabulary: pane.created/exited/
  agent_detected/agent_status_changed/output_matched…, tab.*, workspace.*, layout.*,
  plugin.*), `[[link_handlers]]` (regex → action on clickable output), `[[build]]`.
  A plugin pane is a subprocess's TUI rendered in a terminal pane (`plugin.pane.open`);
  confirmed for file-viewer and reviewr (both compiled TUI binaries). **No plugin-drawn
  non-terminal UI exists** ("Native non-terminal plugin UI = future, does not exist" —
  HANDOFF.md; echoed by ADR-0012).
- **(b) Programmatic decoration: good, display-only.** `pane report-metadata`
  (tokens ≤16/report ≤32/pane, TTL ≤24h, seq; title; display_agent; state_labels) feeds
  herdr's built-in sidebar template engine (`[ui.sidebar.agents] rows` with builtins +
  `$token` cells) — any external process may push, not just declared plugins. No color/
  badge/icon fields yet (per-token fg/bold/dim styling is unreleased upstream). Built-in
  agent-status detection (foreground process + screen-manifest evaluation + hook events →
  Idle/Working/Blocked) powers `herdr agent wait/send/read` and fires
  `pane.agent_status_changed` events plugins can subscribe to.
- **(c) Non-terminal content: none.** No browser/image/preview pane type; the popup pane
  has no pane id and is invisible to the APIs.

### 3.4 Verdicts

- **"Can cmux/limux be extended to have custom panes?"** cmux: not pane *types* (sidebar
  extensions only, beta), but its drivable browser panel already delivers external-process
  custom **web UI in a pane** — the only one of the three that can. limux: no, not today,
  by explicit ADR decision to defer; the Qt port is the prerequisite; expect nothing
  third-party-usable before the post-v1 loader.
- **vs herdr:** herdr has the only *shipped, third-party-usable plugin pane system* — at
  the price that all plugin content is TUI-in-a-terminal. For terminal-native agent
  tooling (herdmates' domain) that constraint is mostly irrelevant; for rich dashboards
  it's the gap (D2's flagged future upstream ask).
- Event model: herdr plugins get push events (subscribe/react); cmux external processes
  poll or hook notifications; limux has no push at all until the ADR-0007 seam lands.

---

## 4. Implications for herdmates

Context locked by the maintainer: all agents are Claude Code now (Codex workers frozen at
v1.1.0), `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1` is enabled, herdr's bash/CLI-driven
model is a feature (not a gap) — and the teammux shim is in active development.

1. **The shim's contract is now fully specified and de-risked.** §1.2's verb set +
   [spike]'s 36-call capture + no-control-mode confirmation (three independent sources:
   spike argv, binary strings, official docs' discrete-pane-ID model). Build against argv
   translation; the `%N`↔pane-id table is the core work; 7 cosmetic styling verbs may
   no-op; startup probes (`show -Av mouse` etc.) can return static constants.
2. **Genericize the shim beyond claude-teams** (cmux precedent: one `__tmux-compat`
   serves claude-teams + omc + omx + omo). Matches REPORT-features.md's STEAL verdict.
   Costs little now, buys every tmux-driving orchestrator later.
3. **Don't wait for the shim to deliver value** — the coordination layer (§1.1) already
   works in herdr. D1's file-based board (team config + inboxes → sidebar tokens) is
   correctly aimed at the durable, documented substrate; the shim remains the quarantined
   upside layer, exactly as ADR-0012 frames it.
4. **limux is not a competitor for this workflow horizon.** Its teams integration is
   missing the load-bearing half (§2.2), its host bridge exposes 19 methods, its GTK host
   is mid-replacement by the Qt port, and its plugin system is post-v1. Revisit only if
   the Qt host lands with the pane-type registry + event channel seams — at which point
   porting a then-mature teammux verb table to `limux __tmux-compat` is straightforward
   (the long-flag half and the lockfile'd compat store already exist there).
5. **Decoration ideas worth stealing into D1/D3** (from cmux's surface): status pills
   with icon/color/priority, progress bars, leveled log lines, and jump-to-newest-unread.
   Herdr's token system emulates the textual part today; per-token styling is the
   upstream ask to watch. attention.jump already covers the ⌘⇧U analog.
6. **Rich non-terminal board (D2 ambition):** no path in herdr today (§3.3c); cmux's
   drivable browser panel is the proof that a webview pane is the right upstream ask —
   but a TUI board via `plugin.pane.open` is the shippable v2.0 form, as already scoped.

---

## 5. Source anchors

- Claude Code agent teams docs: https://code.claude.com/docs/en/agent-teams
- claude binary probes: `~/.local/share/claude/versions/2.1.211` (strings: env-var gate,
  teammateMode, spawn argv, mailbox/inboxes/team file, TMUX/TMUX_PANE checks)
- cmux docs (24 pages): https://cmux.com/docs — key pages: /agent-integrations/*, /api,
  /browser-automation, /dock, /vault, /custom-commands, /remote-tmux, /ssh, /changelog
  (CmuxExtensionKit), /concepts, /skills
- limux [code] anchors: `rust/limux-cli/src/main.rs` :3109 (tmux-compat long-flag),
  :3390-3600 (CLI dispatch), :1951/:2192 (agent-team/build_agents_md);
  `rust/limux-host-linux/src/control_bridge.rs` :19-36 (19 wired methods), :394-410
  (browser pane.create rejection + regression test), :675 (hard-error default);
  `rust/limux-core/src/lib.rs` (~90 method arms; browser.* cluster ~3844);
  `docs/adr/0007-plugin-architecture.md`, `docs/adr/0004-ui-toolkit-qt6.md`,
  `docs/specs/phase2-qt-host-spec.md` :46-48/:143-150/:208-266
- herdr: `~/.config/herdr/plugins.json` (herdr 0.7.4);
  `docs/research/herdr-api-schema.snapshot.json` (`$defs.PluginPaneOpenParams`,
  `$defs.PaneReportMetadataParams`); `herdr-plugin.toml` (manifest shape);
  `docs/research/sidebar-rows.toml`; `docs/research/upstream-architecture-claims-2026-07-15.md:29`
- This repo: `docs/adr/0012-pivot-to-herdmates.md` (note §1.3 correction),
  `docs/research/spike-tmux-verbs-2026-07-16/REPORT.md`,
  `docs/research/cmux-comparative-2026-07-16/{REPORT.md,REPORT-features.md}`
