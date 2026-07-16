# Feature reality audit — intended product versus v1.0.0

Date: 2026-07-15  
Reviewed revision: `v1.0.0` → `aa0c0e05b0a26074e5f11328a781b41cb633f669`  
Wayfinder ticket: [Compare intended capabilities with observed reality](https://github.com/caioniehues/herdr-agent-team/issues/44)

## Executive conclusion

The released plugin substantially exceeds its original thin-v1 feature cut.
The original accepted core—team description, launch orchestration, immutable
worker instructions, durable run state, push report pointers, status, and safe
teardown—grew through explicit decisions into messaging/outboxes, adoption,
lifecycle reconciliation, metadata and notifications, a control deck, resumable
spawn, god-side inbox/wait tools, and an experimental socket collector.

The implementation is strongest where the plugin owns the truth: parsing,
protocol generation, durable run transactions, pure lifecycle reconciliation,
queue ordering, read marks, and local teardown policy. It is weakest where it
turns external presentation or request acceptance into a stronger domain claim.
Five concrete capabilities are Partial because the released source can claim
completion, delivery, acknowledgment, metadata, or teardown more strongly than
the available evidence supports.

Fresh external proof was deliberately not fabricated. Herdr and both supported
agent CLIs are installed, and the live Herdr schema matches the checked-in
protocol-16 snapshot exactly, but this session did not create and tear down two
fresh teams. Consequently, twelve Herdr/provider-dependent capabilities remain
Unverified under the agreed proof standard even where historical dogfooding and
fake-backed tests are strong.

## Method

This audit applies:

- [historical-intent evidence ledger](feature-reality-evidence-ledger.md);
- [outcome-first capability map](feature-capability-map.md);
- [proof standard](feature-proof-standard.md);
- current [domain glossary](../../CONTEXT.md);
- current external-constraint research on local branch
  `research/current-upstream-runtime-constraints`, commit
  `8e5f105c766694d0ef9829bbc8b29fe6a2e4e14b`.

The original accepted baseline is `docs/spec.md` and ADRs 0001–0007 at root
commit `2416889`. ADRs 0008–0011 and later decision-bearing specification/issue
changes are chronological amendments. Current README/spec “shipped” text,
closed tickets, worker reports, and learnings are claims and evidence pointers,
not behavioral proof by themselves.

Behavior verdicts mean:

- **Working** — meets the capability-relative threshold in the reviewed local
  envelope;
- **Partial** — material accepted behavior or truth semantics are missing or
  wrong;
- **Unverified** — an implementation path exists, but external proof is below
  threshold;
- **Absent** — no implementation path provides the outcome.

Constraints are annotated separately as documented external (`Ext-doc`),
undocumented external (`Ext-undoc`), self-imposed implementation (`Self`), or
none identified (`None`).

## Evidence snapshot

- `cargo fmt --check`: pass.
- `cargo clippy --all-targets -- -D warnings`: pass.
- `cargo test`: 183 passed, 0 failed.
- File-spec and `--agents claude,codex` dry runs: pass.
- Live read-only environment: Herdr 0.7.3, Claude Code 2.1.210, Codex CLI
  0.144.4.
- `herdr api schema --json`: byte-identical to
  `docs/herdr-api-schema.snapshot.json`, SHA-256 `7d1eede56d8009fe5b1b2f76e9cedf50b730124cec821fdc3b68ca07cd545f83`.
- Plugin list: `caioniehues.agent-team` enabled from this local checkout.
- No stateful external capability received the two consecutive fresh runs
  required for a Working verdict.

## Scope evolution

### Original accepted cut

At `2416889`, the intended plugin was deliberately thin:

- standalone Rust Herdr plugin, with Herdr as control plane;
- existing god session coordinating workers;
- star default and mesh opt-in;
- configurable Claude/Codex launchers;
- team spec plus shorthand;
- worktree/setup support;
- worker instruction generation;
- durable run state;
- status-event hook writing inbox events and pushing report pointers;
- status and teardown with dirty-worktree preservation.

Dashboard, restart/reassignment, run-history browsing, additional tested agents,
and a limux backend were explicitly deferred.

### Explicitly evolved scope

- ADR-0008: self-contained `msg`, mid-turn policy, and durable outbox.
- ADR-0009: full-worker adoption, ad-hoc team bootstrap, recovery, and release
  rather than ownership teardown.
- ADR-0010: corrected evidence hierarchy and schema/source discipline.
- ADR-0011: experimental, opt-in public socket collector with CLI default and
  fallback.
- Later accepted work: lifecycle reconciliation; optional task and session
  identity persistence; schema-gated metadata/notifications; native control
  deck; resumable/parallel spawn; god-side inbox/report/wait and skill; atomic
  run updates.

### Consciously deferred or cancelled

- Dependencies between worker tasks remain deferred (`docs/spec.md:249-254`).
- Provider-neutral agent restart remains deferred because upstream offers no
  reliable targeted-resume contract (`docs/spec.md:291-296`).
- Mid-run mesh adoption remains rejected until an append-only protocol
  amendment has a real user (`docs/spec.md:532-534`).
- Run history, additional tested launchers, limux backend extraction, and
  declarative layouts remain optional (`docs/spec.md:297-300`).
- Generic statusline/agent list and per-worker CLI-wait fan-out were cancelled
  in favor of native Herdr/sidebar and aggregate collection
  (`docs/spec.md:302-307`).

## Capability matrix

### 1. Describe a team

| Capability | Verdict | Constraint | Evidence and divergence |
|---|---|---|---|
| Parse defaults and validate team specs | Working | None | `TeamSpec`/`WorkerSpec` model (`src/types.rs:7-49`), validation (`src/spec.rs:180-223`), examples, dry run, and deterministic positive/negative tests agree with original spec section 2. |
| Configure launcher mechanics without code changes | Working | Ext-doc | Launcher table is data-driven and conservatively defaults unknown mid-turn behavior; parsing/replacement tests pass. Actual provider compatibility is a separate runtime claim. |
| Express topology, roles, tasks, worktree policy, and setup | Working | None | Fields persist from spec into run state (`src/types.rs:9-49`, `85-114`); tests cover optional tasks, defaults, branches, and worktree invariants. |
| Express task dependencies | Absent | Self | Explicit conscious deferral, not an original unmet commitment (`docs/spec.md:249-254`). |
| Build an ad-hoc team description from `--agents` | Working | None | Shorthand materializes safe briefs and validates launchers (`src/spec.rs:99-177`); live-local dry run and tests pass. |

### 2. Start a team

| Capability | Verdict | Constraint | Evidence and divergence |
|---|---|---|---|
| Preflight before mutation and preserve partial state | Working | None | Source preflights before run creation and records failed allocation state; tests cover absent CLIs, unreachable Herdr, unsafe names, setup/worktree failure, and partial launch failure. |
| Create real Herdr workspaces/worktrees and run setup | Unverified | Ext-doc | Typed `HerdrApi` path and fake-backed integration tests are strong; no two fresh stateful runs were performed. Herdr availability and project setup commands remain external. |
| Generate immutable per-worker protocols without overwriting authored `AGENTS.md` | Working | None | All resources are allocated before `create_new` protocol writes (`src/spawn.rs:462-479`); shared-cwd and star/mesh golden tests pass. This consciously revised the original generated-`AGENTS.md` mechanism while preserving its intent. |
| Launch and submit briefs to Claude and Codex | Unverified | Ext-doc | Concurrent launch, retry, checkpoint, and optional-session logic are tested, but successful TUI receipt requires current provider runs. Codex sandbox approval remains documented (`docs/spec.md:506-510`). |
| Resume an interrupted spawn without duplicating completed work | Unverified | Ext-doc | Durable checkpoints and recovery branches are extensive (`src/spawn.rs:491-541`) and tested, but real pane/worktree recovery was not freshly repeated. |
| Persist optional agent/session identity without blocking launch | Working | Ext-doc | Null/missing identity is explicitly modeled (`src/types.rs:96-149`) and covered by compatibility and delayed-identity tests. Provider-native resumability is not promised. |

### 3. Direct collaboration

| Capability | Verdict | Constraint | Evidence and divergence |
|---|---|---|---|
| Resolve singular, subset, and all-live targets before side effects | Working | None | Fan-out prevalidates every target, filters terminal workers, deduplicates, and aggregates errors (`src/msg.rs:132-187`); deterministic tests cover each rule. |
| Queue non-ready targets durably and drain in order | Working | Ext-doc | Outbox files, ordering, failure retention, deletion-after-submission, and audit events are covered at real filesystem seams (`src/msg.rs:483-510`, `src/hook.rs:257-300`). Drain latency depends on a later status flip by design. |
| Submit instructions into real panes | Unverified | Ext-doc | `pane_run` plus working-status retry is implemented (`src/msg.rs:457-480`), but submission into current Claude/Codex TUIs was not freshly repeated. |
| Establish target acknowledgment | Absent | Self | No acknowledgment protocol exists. Source names pane submission `Delivered` and writes `delivered` audit events (`src/msg.rs:370-377`, `src/hook.rs:288-299`). This is primarily documentation/domain drift: accepted ADR-0008 promised submission verification, not proof of agent comprehension. |
| Enable mesh peer messaging | Unverified | Ext-doc | Immutable peer names and self-contained `msg` invocations are golden-tested; end-to-end peer receipt is provider/runtime-dependent. |
| Raise and clear explicit attention | Unverified | Ext-doc | Worker-to-god attention metadata and notification paths are tested. Real metadata/notification presentation needs live proof, and the board acknowledgment gap below weakens the full outcome. |

### 4. Understand team state

| Capability | Verdict | Constraint | Evidence and divergence |
|---|---|---|---|
| Preserve durable membership, checkpoints, identity, and lifecycle | Working | None | Atomic temp-file replacement plus advisory locking centralize updates (`src/run.rs:65-198`); round-trip, compatibility, and stale-writer tests pass. |
| Reconcile pane/workspace/worktree lifecycle changes | Unverified | Ext-doc | Pure reconciliation covers move, exit, close, removal, god loss, and agent detection (`src/reconcile.rs:117-321`); manifest registers all events (`herdr-plugin.toml:57-83`). Fresh hook delivery was not repeated. |
| Publish schema-gated team/role/task/status metadata | Partial | Self | Capability detection and mapping work, but the hook always passes `task: None` (`src/hook.rs:132-141`) despite accepted “task when available” behavior (`docs/spec.md:230-236`). |
| Emit truthful aggregate notifications | Partial | Self | Deduplication and blocked/exit policies are tested, but “Team complete” fires when every agent presentation status is merely `idle|done` (`src/reconcile.rs:222-238`), contradicting the product’s own stopped-not-done discipline. |
| Operate the native control deck | Partial | Self | Rendering, task/report rows, key mapping, and subprocess reuse are tested (`src/board.rs:112-203`). However `g` only sends literal `acknowledged` to the selected worker (`src/board.rs:162-168`); it does not clear durable attention, despite the public “ack/answer attention” claim (`docs/spec.md:242-248`). Live terminal behavior is also unverified. |
| Accelerate board/wait observation through the public socket | Unverified | Ext-doc + Self | Exact schema gate, typed frames, caps, deadlines, reconnect controller, redacted trace, and CLI fallback have strong fake-server coverage (`src/socket.rs:137-225`, `543-560`; `src/socket_backend.rs:55-180`). It remains opt-in/experimental, current live parity was not repeated, and the direct backend is unavailable on non-Unix platforms (`src/socket.rs:463-472`). |

### 5. Collect durable results

| Capability | Verdict | Constraint | Evidence and divergence |
|---|---|---|---|
| Give every worker a durable report contract | Working | Ext-doc | Generated protocols carry absolute report path, ordering, sentinel, and self-contained messaging; golden tests pass. Worker compliance remains external. |
| Push report pointers and append lifecycle events | Unverified | Ext-doc | At-most-once transition logic and exact pointer construction are tested (`src/reconcile.rs:200-213`, `src/hook.rs:239-255`); actual manifest invocation and god-pane submission were not freshly repeated. |
| Inspect inbox, reports, read marks, and stopped-not-done state | Working | None | Snapshot and read-mark logic is durable and tested (`src/god_cli.rs:132-191`, `325-376`). Missing reports remain visible rather than inferred complete. |
| Wait for a finalized result | Partial | Self | `any-report`, `report:<worker>`, and `all-reports` become reached from file mtime/existence alone (`src/god_cli.rs:259-270`, `349-375`). No finalization or completion marker is checked, so a newly created but incomplete report counts as ready. |
| Distinguish terminal workers from missing required reports | Working | None | Wait returns distinct timeout, dead-worker, inactive-run, and reached verdicts; tests cover ended/orphaned/failed and stable JSON. |

### 6. Change or recover a team

| Capability | Verdict | Constraint | Evidence and divergence |
|---|---|---|---|
| Adopt an existing detected-agent pane as a full worker | Unverified | Ext-doc | Star-only targeting, protocol generation, conservative unknown-agent policy, submission checkpoints, ad-hoc bootstrap, and idempotent recovery are deeply tested (`src/adopt.rs:313-439`). Real pane detection/submission was not freshly repeated. |
| Recover pending adopted participation | Unverified | Ext-doc | Persist-before-submit crash window and rerun cases have targeted tests (`src/adopt.rs:384-434`); live provider behavior is unverified. |
| Persist identity needed for conditional recovery | Working | Ext-doc | Full optional `agent_session` and Herdr socket/session identity round-trip. Identity may legitimately remain null; provider-neutral resume is outside the feasible boundary. |
| Restart a completed/failed agent session | Absent | Ext-doc | Conscious deferral pending a reliable provider-specific `resume_command` or upstream targeted resume (`docs/spec.md:291-296`). |
| Adopt into a live mesh without stale peer contracts | Absent | Self | Explicitly rejected to preserve immutable protocols; append-only amendments are deferred until demanded (`docs/spec.md:532-534`). |

### 7. End participation safely

| Capability | Verdict | Constraint | Evidence and divergence |
|---|---|---|---|
| Detect and preserve dirty worktrees | Working | None | Uses real `git status --porcelain --untracked-files=normal` (`src/status_kill.rs:586-610`); tests create real repositories and preserve untracked evidence. |
| Apply owned-close versus adopted-release policy | Working | None | Teardown selects only owned workspace IDs and sends release notices only to adopted panes (`src/status_kill.rs:383-420`, `524-560`); deterministic backend tests cover whole-run and single-worker policy. |
| Close/release real external participation | Unverified | Ext-doc | Actual workspace close, worktree removal, and pane notification depend on Herdr and provider state; no fresh teardown was performed. |
| Keep durable teardown truth aligned with external reality | Partial | Self | Close/remove/release-notice failures are logged and ignored, then workers/runs are persisted Ended or Released (`src/status_kill.rs:392-427`, `453-488`). The run board can therefore claim external participation ended when teardown was not confirmed. |

## Verdict distribution

Across 37 concrete capabilities:

- **Working: 16**
- **Partial: 5**
- **Unverified: 12**
- **Absent: 4**

The four Absent capabilities are not equivalent failures: task dependencies,
provider restart, and mesh adoption are conscious deferrals; end-to-end target
acknowledgment was never part of ADR-0008’s actual submission contract, but the
current “delivery” vocabulary obscures that boundary.

## Cross-cutting assessment

### Correctness

Owned pure/local behavior is generally coherent and heavily tested. The main
correctness failures are semantic: incomplete report files can satisfy waits;
idle/done can generate “Team complete”; task metadata is dropped; board
acknowledgment does not acknowledge durable attention.

### Durability and recovery

This is the strongest axis. Run state, hook metadata, outboxes, read marks,
checkpoints, and identities are durable and transactionally updated. Resume and
adoption recovery have unusually focused tests. External recovery remains
conditional on panes, worktrees, identity, and provider behavior the plugin
does not own.

### Observability and evidence

The project correctly separates durable inbox truth from transient pointer
notifications in most places, but then weakens that discipline through
`report_present`, `Delivered`, “Team complete,” board `g`, and unconditional
Ended/Released persistence after external failures. The evidence hierarchy is
stronger than the runtime vocabulary.

### Partial-failure safety

Spawn checkpoints, atomic updates, queue retention, dirty-worktree salvage, and
best-effort teardown are strong. The residual problem is honest state after
best-effort teardown: preserving progress is good, but marking the external
effect complete after failure hides recovery work.

### Portability

Config-driven launchers and conservative unknown-agent policy provide leverage,
but only Claude and Codex have historical validation. Sandboxes, approvals,
network access, hooks, and provider session identity remain external. The
manifest supports Linux/macOS; the socket is Unix-only by implementation and
the broader provider/platform matrix is unverified.

## Divergence register

| Divergence | Class | Consequence |
|---|---|---|
| Generated repository `AGENTS.md` became immutable per-run worker protocols | Conscious revision | Preserves authored instructions and makes shared-cwd workers safe while retaining the intended brief/report contract |
| Dashboard deferral became a human control deck | Conscious revision | Avoids duplicating Herdr’s generic status UI and concentrates team-specific actions |
| Raw Herdr messaging became plugin `msg` plus outbox | Defect-driven revision | Fixes unsubmitted text and provides policy/queue durability |
| Hook expanded from status only to lifecycle reconciliation | Defect-driven revision | Prevents stale IDs and zombie run truth after resource movement/removal |
| `team wait` uses report existence as completion | Implementation gap | Can release the coordinator before a report is finalized |
| Aggregate completion uses idle/done | Defect | Can announce completion for stopped-not-done workers |
| Task metadata is always omitted | Implementation gap | Sidebar/title cannot expose accepted task facts |
| Board acknowledgment only sends a word | Implementation gap | Attention remains durable even though UI claims acknowledgment |
| Submission is called delivery | Documentation/domain drift | Users and event consumers may infer target receipt that was never established |
| Teardown persists completion after external failure | Implementation gap | Durable run truth can conceal still-live or unreleased external participation |
| Restart, task dependencies, mesh adoption, history, extra providers | Conscious deferral | Correctly remains beyond the accepted destination until evidence of need/feasibility exists |

## Candidates for focused diagnosis

The audit does not diagnose causes. These findings should enter
`/diagnosing-bugs` only after the named red-capable loop exists:

1. **Premature result readiness** — a fast test that creates a report path,
   leaves content unfinished, and asserts `wait report:<worker>` must not reach.
2. **False aggregate completion** — a pure reconciliation test with all workers
   `idle|done` and no ready reports that asserts no team-complete notification.
3. **Dropped task metadata** — a hook test with `worker.task = Some(...)` and a
   title-capable schema that asserts the task reaches `pane report-metadata`.
4. **Board acknowledgment semantics** — first choose the correct seam for
   clearing attention, then assert that the board action changes durable
   attention state rather than merely submitting a string.
5. **Teardown truth after close/notice failure** — a backend-failure test that
   asserts the persisted lifecycle exposes incomplete teardown or retry need.

Each can be deterministic and agent-runnable in seconds. None was converted into
a fix during this planning audit.

## Product-level decision

The plugin’s realistic product identity is a durable team-domain and
coordination layer over Herdr. It should promise durable membership, policy,
instructions, reports, and aggregate coordination truth while describing pane
submission, provider state, and resource teardown at exactly the confidence the
external platform permits. The implementation review should prioritize places
where external effects cross into durable domain truth, because that is where
the current product most often overclaims certainty.
