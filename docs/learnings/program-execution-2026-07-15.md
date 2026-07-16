# Learnings — implementation-review program execution (2026-07-15)

Wave: full program execution (#47–#65 + Stage 3 + slices 1–6) in one
coordinated session, 6 worker sessions across 8 worktrees, all merged to
`integrate/program-wave1` (197 tests, gate clean, NOT pushed).

## What worked

- **Sentinel-file monitoring + living-session reuse.** Report-file sentinels
  (`LOOPNN GREEN`) greped by a background watcher caught every completion;
  re-briefing living sessions onto new absolute-path worktrees bypassed both
  the re-spawn cost AND (critically) the machine-wide new-session crash.
- **Salvage-with-suspicion briefs.** Every worker inheriting a dead
  predecessor's diff was told "verify at ground truth, treat as unverified" —
  all three found real problems in the salvage (loop47-50's predecessor had
  broken an existing test; loop49's had destroyed its own RED proof).
  RED-reconstruction-by-revert became the standard move.
- **Review-the-fixes-in-context.** Running slices 1–4 against the integration
  branch (not v1.0.0) caught a regression OUR wave introduced (#65, claim
  crash-window) within hours of writing it, plus the shallow-predicate
  divergence (#69: two readiness definitions).
- **Coordinator-confirms-at-source before filing.** Both 🔴 review claims
  (adopt create_new, UnixStream cfg) were verified against code before
  ticketing — kept the tracker free of plausible-but-wrong findings.

## What bit us

- **Vendor monoculture assumption.** The codex monthly limit died mid-wave
  with 4 workers in flight. Salvage worked (worktrees + git-visible state),
  but the wave lost ~40 minutes. Caio's directive: claude workers only now.
- **`/model <alias>` on the tweakcc-patched claude poisons global settings**
  (`model` key → >256-char string → API 400 for the session). Separately,
  new claude sessions began dying at startup (`e.toLowerCase`, "Baked for
  0s") — config-borne (pristine CLAUDE_CONFIG_DIR works), flaky, UNSOLVED;
  worked around by never spawning fresh sessions. Prime suspect: tweakcc.
- **Enter-swallow is universal.** Every claude pane needed the
  `wait --status working` check; roughly half needed one re-sent Enter.
- **Compound gh flags silently no-op.** `gh issue create --json` is invalid;
  with `2>/dev/null | tail -1` the failure was invisible — two issues
  silently didn't exist until a list-check caught it. Always verify tracker
  writes by reading back.

## Program-level

- The RED-first + twice-run + review-in-context pipeline found 19 new
  defects/decisions (#61–#83) beyond the 12 original Unverified rows —
  live evidence and adversarial review each caught things the other missed.
- Findings-filed-not-fixed (slice acceptance) is the correct regress
  boundary: the wave fixed its own regressions (#65) and everything the
  program scoped; #66–#83 are the new, Caio-prioritized frontier (two HIGH:
  #74, #75; two needs-decision: #77, #79).
