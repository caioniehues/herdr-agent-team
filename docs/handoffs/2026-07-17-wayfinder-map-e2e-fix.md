# Handoff — wayfinder map live; E2E attempt 1 root-caused + fixed, re-run pending

For: continuing THIS session or a fresh one. Written 2026-07-17 (session
spanning 2026-07-16 evening). Complements root `HANDOFF.md` (wave-1
closeout, still valid EXCEPT its step 3 — the #85 E2E — which this
session absorbed into the wayfinder map and advanced).

## 1. The wayfinder map (the session's main product)

North-star effort charted per /wayfinder on GitHub Issues:

- **Map: #87** — destination: ADR-0013 + `spec.md` rewrite locked +
  ticketed mission-control v1 build order. All charter decisions
  (grilled with Caio 2026-07-16) are in the map body's table: herdr
  plugin period; monitor + inbox-write + hook companion; single-team v1;
  shim = pillar, Claude-only; recorder v1 minimal; ETA banned (proxy
  only); TUI plugin pane owns rich tier; native task files (beads out);
  spine accepted as candidate set.
- **Closed (verified + resolution comments on the issues):**
  - #88 task-file schema — findings + coordinator addendum:
    `docs/research/native-teamfiles-schema-2026-07-16.md`. Core fields
    stable; status enum `pending|in_progress|completed`; edges = arrays
    of task-id strings; inboxes transient (drained on read → polling
    tail lossy); stale team configs persist (presence ≠ active).
  - #91 hook surface — findings + addendum:
    `docs/research/hook-companion-surface-2026-07-16.md`. Exactly 3 team
    hooks (TeammateIdle/TaskCreated/TaskCompleted), no matchers, only
    exit-2 blocks; no spawn/message/plan hooks (= cannot-gate list);
    plugins can ship hooks; inbox writes need `.lock` sidecar +
    read-filter-atomic-rename; mailboxes materialize in ALL modes.
- **Frontier:** #89 live E2E (CLAIMED, in progress — see §2),
  #90 focus-pane fate (grilling w/ Caio + salvage-check
  `~/Projects/herdmates-issue84`), #92 waiting-reason/deadlock signals
  (grilling, now fully fed by #88+#91).
- **Blocked:** #93 build order (by 89/90/92) → #94 write north star
  (closes map). Native sub-issues + blocked-by edges are wired in
  GitHub — frontier is visible in the UI.
- Wayfinder discipline: ONE ticket per session (research excepted);
  claim by assigning; resolution comment + close + map Decisions-so-far
  line (map body source of truth: edit via `gh issue edit 87 --body-file`).

## 2. #89 E2E — attempt 1 failed, bug fixed, RE-RUN IS THE NEXT ACT

Attempt 1 (claude 2.1.211, lead pane `w1A:p15`, session `489655e5`):
lead started behind the shim correctly (fake TMUX env verified live),
created the 3-task DAG, then **teammate spawn failed**:
`tmux list-panes -t @0` → teammux hard-errored on the coordinator's own
foreign pane `w1A:p1` (not in idmap). Root cause doc (written by the
lead): `docs/research/teammux-e2e-2026-07-16/root-cause-list-panes-blocker.md`.
Structural: ANY foreign pane in the tab killed spawning for the tab.

**Fix applied + gated this session** (`src/teammux.rs::list_pane_ids`):
unmapped panes get a lazily minted persistent `%N` via
`IdMap::allocate` (tmux semantics: every pane in a window has an id).
Regression test `list_pane_ids_lazily_registers_a_foreign_pane_missing_from_idmap`
replaces the old fails-loudly test. fmt + clippy -D warnings clean,
286/286 tests, release binaries rebuilt. Committed on main (see git log).

**Re-run procedure (fresh session does exactly this):**

1. Cleanup: close stale lead pane `w1A:p15` if still open
   (`herdr pane close`/kill via UI); optionally clear old idmap state
   `~/.local/state/herdr/plugins/caioniehues.herdmates/teammux/w1A_p15.json`.
   Team `session-489655e5` + its tasks are dead residue — leave or rm.
2. Start poller (captures transient inboxes + task transitions):
   `bash docs/research/teammux-e2e-2026-07-16/scripts/e2e-poller.sh` in
   background. Note: it writes under the `2026-07-16` evidence dir; fine.
3. Launch (from a herdr pane, repo cwd):
   `./target/release/herdmates teammux-launch --model sonnet
   --dangerously-skip-permissions '<E2E prompt>'` — the prompt used in
   attempt 1 is in this session's transcript; essentials: 2 teammates
   alpha/beta (sonnet), 3 tasks with C blockedBy A+B, teammates only
   send PING messages (no files/git), lead completes C, dismisses both,
   prints E2E-COMPLETE.
4. Watch: `scripts/e2e-watcher.sh` (edit its IDMAP path to the NEW lead
   pane's state file printed by teammux-launch). Proof = teammate panes
   appear in `herdr pane list` then disappear on dismissal.
5. Evidence to keep: pane timeline, idmap snapshots (%N growth),
   NON-EMPTY inbox capture (the one gap #88/#91 left — entry schema),
   `tmuxPaneId` values in the new team's config.json, task transitions.
6. Resolve #89: resolution comment (pass/fail + evidence paths), close,
   add map Decisions-so-far line, per §1 discipline.

Known traps: watched panes complete as `idle` not `done`; don't key on
attention states. The lead cannot flip teammateMode at runtime
(--settings is process-lifetime).

## 3. Bonus evidence already captured (attempt 1)

`docs/research/teammux-e2e-2026-07-16/captures/`: task files of team
`489655e5` show a LIVE dependency edge (`3.json`: blockedBy ["1","2"])
plus an `owner` field observed as `""` AND `null` (new schema datum —
fold into #92's signal design; owner is nullable-or-empty, don't
string-match). `task-status-log.jsonl` has the transition log.

## 4. Repo/git state

- Fix commit on main this session (src/teammux.rs); docs commit with
  research findings + evidence + this handoff. NOTHING pushed —
  pushes = releases, Caio's word only. Version reconcile still pending
  (Cargo.toml 1.0.0 vs manifest 2.0.0, root HANDOFF step 5).
- Untracked-by-design: `findings.md progress.md task_plan.md` (stale
  planning residue at repo root, excluded from commits).
- #86 focus pane: UNCHANGED this session — branch + worktree salvage
  state as described in root HANDOFF.md §next-steps 1–2; its FATE is
  now map ticket #90 (don't merge before that decision).

## 5. Working agreements active

- Sonnet for research agents (Caio, standing). Model always explicit.
- Verify worker/teammate claims at ground truth before acting — this
  session refuted 2 researcher claims (empty-edges "fact", in-process
  no-mailbox inference) by cheap probes; both corrections are in the
  findings docs' addenda.
- Caio gates: pushes, releases, new issues. Issue comments/closures
  within the authorized wayfinder flow are fine.
