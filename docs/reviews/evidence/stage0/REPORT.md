# Stage 0 live E2E report — issue #46

Date: 2026-07-15 (Europe/Lisbon)

- Plugin: v1.0.0, revision `aa0c0e05b0a26074e5f11328a781b41cb633f669`; the source revision was unchanged. The pre-existing dirty state was docs/planning material only (`run1/environment.txt:2-12`, `run2/environment.txt:2-12`).
- Runtime: Herdr 0.7.3, Claude Code 2.1.210, Codex CLI 0.144.4 (`run1/environment.txt:13-16`, `run2/environment.txt:13-16`).
- Platform: Linux x86_64, CachyOS kernel 7.1.3-2-cachyos (`run1/environment.txt:17`, `run2/environment.txt:17`).
- Live control plane: god pane `w1A:pK`, `HERDR_BIN_PATH=/home/caio/.local/bin/herdr`, socket `/home/caio/.config/herdr/herdr.sock` (`run1/environment.txt:18-20`, `run2/environment.txt:18-20`).
- Exact reproduction command: `/home/caio/Projects/herdr-agent-team/docs/reviews/evidence/stage0/e2e.sh run1 && /home/caio/Projects/herdr-agent-team/docs/reviews/evidence/stage0/e2e.sh run2`

The E2E uses the domain vocabulary literally: one god coordinates each worker by pointer injection and the `msg` verb; durable reports land in the inbox, deferred instructions land in the outbox, and a status flip drives hook reconciliation. The launcher override deliberately sets Codex `queues_midturn = false` to force the queued-drain path; Claude retains `queues_midturn = true` for immediate submission.

## Verdicts for the 12 formerly-Unverified rows

| Formerly-Unverified capability | New verdict | Retained proof or exact blocker |
|---|---|---|
| Create real Herdr workspaces/worktrees and run setup | Working | Both fresh runs reached `brief_submitted` with distinct real workspace, pane, and worktree paths (`run1/run-after-spawn.toml:36-57`; `run2/run-after-spawn.toml:36-57`). Final workspace inventory contains only the pre-existing god workspace (`run1/workspaces-after-kill.json:3`; `run2/workspaces-after-kill.json:3`). |
| Launch and submit briefs to Claude and Codex | Working | Both provider panes reached working, received absolute brief/protocol pointers, and produced reports in both runs (`run1/claude-pane-final.txt:6-17`, `run1/codex-pane-final.txt:17-37`; `run2/claude-pane-final.txt:6-24`, `run2/codex-pane-final.txt:17-37`). |
| Resume an interrupted spawn without duplicating completed work | Partial | Re-running resume was an idempotent no-op in both runs (`run1/resume.log:2-3`; `run2/resume.log:2-3`), but this E2E did not interrupt spawn at a pending checkpoint, so live resource recovery remains unexercised. |
| Submit instructions into real panes | Working | Claude visibly received the immediate mid-turn instruction in both provider TUIs (`run1/claude-pane-final.txt:188-213`; `run2/claude-pane-final.txt:69-85`). Codex received the forced queued instruction (`run1/codex-pane-final.txt:98-119`; `run2/inbox-after-reports/codex-worker.md:4-8`). |
| Enable mesh peer messaging | Blocked | Exact blocker: this scripted flow is star (`run1/team.toml:2`; `run2/team.toml:2`) because live `team adopt` rejects mesh runs to preserve immutable peer contracts. No worker-to-worker mesh submission was exercised, so this row cannot honestly move to Working. |
| Raise and clear explicit attention | Partial | Both workers raised attention through the self-contained worker-to-god `msg` verb, demonstrated by the live start/finalized messages and final inbox state (`run1/inbox.json:3`; `run2/inbox.json:3`). Attention was already false by observation; the E2E has no owned clear/ack action and therefore confirms presentation but not a durable raise/observe/clear contract. |
| Reconcile pane/workspace/worktree lifecycle changes | Partial | Live detected/status/closed events were appended and durable state moved to ended/released (`run1/events-final.jsonl:1-20`, `run1/run-after-kill.toml:41-71`; `run2/events-final.jsonl:1-24`, `run2/run-after-kill.toml:41-71`). However, duplicate hook handling produced `delivered` followed by a false `delivery_failed` for the same removed outbox file in both runs (`run1/events-final.jsonl:16-17`; `run2/events-final.jsonl:17-18`). |
| Accelerate board/wait observation through the public socket | Partial | Each run identified the live socket and `team wait` reached `all-reports` (`run1/environment.txt:20`, `run1/wait-all-reports.json:3`; `run2/environment.txt:20`, `run2/wait-all-reports.json:3`). No socket trace was enabled, so retained output cannot distinguish direct subscription from CLI fallback; backend acceleration remains unproven. |
| Push report pointers and append lifecycle events | Working | Both runs retained worker reports and lifecycle event streams; the god pane received real pointer injection messages before teardown (`run1/inbox-after-reports/claude-worker.md:1-14`, `run1/inbox-after-reports/codex-worker.md:1-9`, `run1/events-final.jsonl:13-20`; `run2/inbox-after-reports/claude-worker.md:1-10`, `run2/inbox-after-reports/codex-worker.md:1-9`, `run2/events-final.jsonl:11-24`). |
| Adopt an existing detected-agent pane as a full worker | Working | A separately created Codex pane was detected and adopted twice fresh (`run1/adopt-first.log:2-3`, `run1/run-after-adopt.toml:41-46`; `run2/adopt-first.log:2-3`, `run2/run-after-adopt.toml:41-46`). |
| Recover pending adopted participation | Partial | The second adopt was idempotent and did not resubmit (`run1/adopt-second.log:2-3`; `run2/adopt-second.log:2-3`), but no crash was injected between persist and submit, so pending-adoptee recovery itself remains unproven live. |
| Close/release real external participation | Working | Adopted workspaces remained live after release (`run1/adopt-workspace-after-release.json:3`; `run2/adopt-workspace-after-release.json:3`) while durable lifecycle became `released` (`run1/run-after-release.toml:41-46`; `run2/run-after-release.toml:41-46`). Spawn-owned workspaces closed. A deliberately dirty worktree caused the required refusal twice (`run1/kill-dirty-refusal.log:1-2`; `run2/kill-dirty-refusal.log:1-2`) before non-removing kill and owned cleanup. |

## Discrepancies between run 1 and run 2

- The successful retained runs reproduced the same capability outcomes. `all-reports` reached after 19,262 ms in run 1 and 27,889 ms in run 2 (`run1/wait-all-reports.json:3`; `run2/wait-all-reports.json:3`).
- Run 2's Claude worker finalized its initial report before the god's immediate message arrived, then updated the report after receiving that message (`run2/claude-pane-final.txt:40-53`, `run2/claude-pane-final.txt:69-85`). Run 1 received the message while the worker was still holding the turn (`run1/claude-pane-final.txt:188-213`). This exposes a spawn-return/worker-progress race, not a submission failure.
- In both runs, one queued Codex message was eventually removed and recorded `delivered`, immediately followed by `delivery_failed` with `No such file or directory (os error 2)` for the same path (`run1/events-final.jsonl:16-17`; `run2/events-final.jsonl:17-18`). This is repeatable contrary evidence of concurrent duplicate drain handling.
- Marketplace `worktree.created` hooks created transient layout workspaces in addition to the plugin's worker workspaces. The script treated these as observed layout races and cleaned only matching worktree workspaces it indirectly created. Final live inventory had no leaked E2E workspace.
- Before the two successful fresh runs, an explicitly retained exploratory attempt used Claude bypass-permissions and stopped at Claude's first-run responsibility confirmation (`run1/aborted-claude-pane.txt:4-14`). The script was corrected to the shipped normal Claude launcher; this attempt is contrary/setup evidence, not one of the two qualifying runs.

## Stage 2 re-rank input

1. Move **Messaging & outbox** ahead of the current read/wait slice: the twice-reproduced `delivered` then false `delivery_failed` race means the hook/outbox boundary emits contradictory durable truth.
2. Keep **Event → durable truth** at priority 1: lifecycle delivery worked, but duplicate status-hook processing created the outbox contradiction.
3. Keep **Durable truth read/wait** high but narrow its socket claim: wait worked twice, while the direct-socket acceleration path lacks trace evidence.
4. Lower ordinary **Orchestration** risk for fresh spawn and healthy adopt/release; keep pending-spawn and pending-adopt recovery as focused unproven sub-slices.

STAGE0 BLOCKED: mesh peer messaging and crash-window pending-adopt recovery were not exercised by the star adopt-compatible E2E
