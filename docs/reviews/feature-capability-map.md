# Feature reality review — capability map

Date: 2026-07-15  
Wayfinder ticket: [Define the capability map and review vocabulary](https://github.com/caioniehues/herdr-agent-team/issues/41)

## Organizing rule

The feature audit is organized around outcomes the god or human needs, not
around commands, source modules, or UI surfaces. A command, board action,
manifest action, hook, skill, or adapter is an **interface or mechanism beneath
an outcome**, not a separately counted capability.

This avoids two common distortions:

1. treating a compound command such as `team spawn` as one indivisible feature;
2. counting the same outcome several times because CLI, board, and skill
   interfaces expose it differently.

## Outcome families

### 1. Describe a team

Express who should participate and how the intended collaboration should be
shaped: workers, roles, topology, tasks, worktree choices, launchers, setup, and
other run policy.

The audit asks whether the model is expressive, coherent, validated, and
faithfully preserved into the run—not merely whether a spec parser exists.

### 2. Start a team

Turn a description into active, durably identified participation: prepare
resources, launch agents, establish membership, and provide each worker the
right brief and protocol.

Partial stages remain separately visible. “Spawn succeeded” must not hide a
worker that owns a workspace but never received usable instructions.

### 3. Direct collaboration

Give workers direction and, where topology permits, let participants exchange
coordination messages. This includes immediate submission, durable queueing,
target selection, broadcast intent, and topology enforcement.

The audit distinguishes **Queued**, **Submitted**, and **Acknowledged**. A
successful pane submission is not described as end-to-end delivery.

### 4. Understand team state

Let the god or human form an accurate picture of membership, identity,
lifecycle, task state, attention, and aggregate progress without treating
provider presentation state as stronger evidence than it is.

CLI, board, metadata, notifications, and skills are assessed as interfaces onto
this shared outcome.

### 5. Collect durable results

Make worker output durably available, discoverable, readable, and waitable.
Pointer notifications and status flips may attract attention, but **Result
ready** is the completion truth: the report is finalized and safe to consume.
The completion sentinel follows readiness but is not durable truth by itself.

The audit must explicitly compare this intended semantic with any implementation
that treats report-path existence as completion.

### 6. Change or recover a team

Adapt participation after the initial start: adopt an existing pane, resume an
interrupted start, reconcile externally changed resources, preserve valid
identity where possible, and degrade honestly when recovery is unavailable.

This family does not imply provider-neutral agent-session resume. External
identity and provider capabilities constrain what recovery can promise.

### 7. End participation safely

Stop owned participation, release adopted participation, preserve unsalvaged
work, and leave team/run truth internally consistent even when resources have
already disappeared.

Ending one worker and ending an entire team are related outcomes whose effects
must remain distinguishable.

## Cross-cutting evaluation axes

Every outcome family is reviewed on the same axes:

| Axis | Question |
|---|---|
| Correctness | Does the observed outcome match the accepted intent, including edge cases and negative guarantees? |
| Durability and recovery | Which facts survive process interruption, restart, or unavailable live resources, and can operation resume safely? |
| Observability and evidence | Can the god distinguish durable truth, live presentation, notification, request acceptance, and explicit acknowledgment? |
| Partial-failure safety | Does failure preserve recoverable work and expose honest state without duplicating, orphaning, or silently losing participation? |
| Portability | Which guarantees hold across supported agent providers, Herdr versions/backends, operating systems, and permission profiles? |

These are axes, not standalone feature families. For example, stale-pane
reconciliation is assessed wherever stale identity affects starting,
understanding, directing, recovering, or ending participation.

## Layers recorded beneath each outcome

For each capability row, the final comparison records:

1. **Accepted intent** — original scope plus explicit chronological amendments.
2. **Public interfaces** — CLI, board, manifest action, skill, config, or file
   protocol through which a caller attempts the outcome.
3. **Implementation mechanisms** — modules and adapters that realize it. These
   identify later code-review seams but do not define the product taxonomy.
4. **Observed reality** — source, test, retained runtime, or fresh live evidence
   under the proof standard chosen by the map.
5. **Constraints** — upstream, provider, sandbox, permission, platform, and
   operational limits.
6. **Verdict, constraint, and divergence class** — a working, partial,
   unverified, or absent behavior verdict; a separate constraint annotation;
   plus abandoned idea, conscious revision, implementation gap, defect,
   documentation drift, or unavoidable constraint.

## Codebase-design consequences

The outcome families are candidates for high-level module interfaces and test
surfaces, not a demand for seven source packages. During implementation review:

- assess whether callers get leverage through small interfaces or must learn
  orchestration details themselves;
- place seams where behavior genuinely varies, especially Herdr CLI/socket and
  agent-provider adapters;
- seek locality where one outcome currently requires shotgun changes across
  unrelated callers;
- test observable outcomes through the interface rather than past it;
- do not introduce hypothetical seams when only one adapter exists.

The deletion test applies: if deleting a module spreads complexity across many
callers, it earns its depth; if complexity disappears, it may be a shallow
pass-through.

## Canonical vocabulary decisions

- A product capability is a user/coordinator outcome, independent of the
  interface used to request it.
- **Result ready** means a finalized report is safe to consume. File existence
  alone does not establish it.
- A **Completion sentinel** follows readiness and attracts attention; it is not
  durable completion truth alone.
- An instruction progresses from **Queued** to **Submitted** and only becomes
  **Acknowledged** with explicit evidence from the target.
- “Delivered” is avoided unless the evidence establishes the exact stronger
  meaning intended at that call site.
