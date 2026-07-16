# Feature reality review — proof standard

Date: 2026-07-15  
Wayfinder ticket: [Set the proof standard for shipped behavior](https://github.com/caioniehues/herdr-agent-team/issues/42)

## Purpose

This standard defines what evidence is sufficient to say a capability works.
It extends ADR-0010's authority lanes—live evidence for observable behavior,
pinned upstream source for attribution, and schema-gated preview surface—with
requirements for test seam, freshness, reproducibility, contrary evidence, and
honest uncertainty.

## Unit of judgment

Judge a concrete capability beneath one outcome family, not an entire command,
module, release, or outcome family at once. Each capability record contains:

1. accepted-intent citations;
2. the observable outcome and supported envelope;
3. the proof threshold appropriate to that outcome;
4. evidence citations with revision, environment, and date;
5. one behavior verdict;
6. one constraint annotation;
7. findings across the five cross-cutting axes;
8. contrary evidence and unresolved uncertainty;
9. a divergence class where intent and reality differ.

## Capability-relative proof threshold

Evidence must exercise the real seam at which the claim can fail.

| Capability shape | Minimum evidence for **Working** |
|---|---|
| Pure parsing or state transition | Released source inspection plus deterministic tests at the public behavior seam, covering accepted positive and negative guarantees |
| Filesystem or local-process integration | Source plus integration tests using the real filesystem/process boundary and representative failure cases |
| Herdr CLI, socket, event hook, pane/workspace lifecycle, or cross-process timing | Source/tests plus fresh live validation against the released plugin revision and identified Herdr runtime |
| Agent-TUI behavior, provider semantics, permissions, sandboxing, or mid-turn interaction | Source/tests where available plus fresh live validation with the named agent/provider and permission profile |
| Portability claim | Evidence on every platform/provider/runtime included by the claim, or a narrower explicitly documented support envelope |
| Reliability or race-safety claim | A deterministic stress, replay, fault-injection, or repeated integration loop capable of exposing the claimed failure mode |

Source inspection alone proves that an implementation path exists, not that an
external outcome works. A unit test behind a fake proves only the behavior the
fake models. Historical live evidence supplies chronology and candidate
procedures, but it cannot establish current behavior when the plugin, Herdr,
agent CLI, schema, platform, or permission profile may have changed.

## Evidence lanes

### Accepted-intent evidence

Use the historical-intent ledger: contemporaneous accepted ADRs and the
buildable specification establish the original commitment; only explicit later
decisions amend it. Brainstorms and research do not become unmet requirements
without acceptance.

### Implementation evidence

- **Source** identifies the path, ownership, error handling, and attribution.
- **Automated tests** prove only the observable behavior and seam they actually
  exercise.
- **Static configuration** such as the manifest, examples, generated protocols,
  and skills proves the shipped surface or instruction, not successful runtime
  behavior.
- **Review reports and closed tickets** are claims and pointers to stronger
  evidence, never proof by themselves.

### Runtime evidence

Fresh runtime evidence is decisive for observable behavior in the tested
environment. It must name the plugin revision, Herdr/runtime version, agent
provider/version where relevant, platform, backend, permission profile, date,
procedure, and retained output.

### Upstream and external evidence

Pinned current upstream source is decisive for ownership and available surface.
Official documentation establishes supported contracts but does not override
contrary observation on the tested runtime. Preview or drifting surface remains
schema-gated and cannot support an unconditional working verdict.

## Reproducibility

- A deterministic local check requires one clean retained run.
- A stateful or external integration requires two consecutive successful runs
  before receiving **Working**.
- A failure must be confirmed to exercise the intended path rather than a
  neighboring setup failure.
- Timing-sensitive or intermittent failures require a recorded reproduction
  rate under a fixed loop, seed, load, or timing window.
- Manual-only validation requires a retained checklist and evidence artifact;
  an unrecorded recollection is historical context only.
- Every runtime record includes the exact command where agent-runnable. When a
  human is required, it records each action and expected observation.

## Behavior verdict

Verdicts are mutually exclusive for one concrete capability in one declared
support envelope.

### Working

The accepted observable outcome and its important negative guarantees meet the
capability-relative proof threshold in the declared envelope. Evidence is
current enough for its dependencies, reproducible, and not contradicted by an
unresolved equal-or-higher-authority observation.

### Partial

Some accepted observable behavior is proven, but at least one material
sub-outcome, negative guarantee, interface path, or required environment in the
declared envelope is missing or fails. A known defect normally makes the
affected capability Partial, not Working.

### Unverified

A claim or implementation path exists, but available evidence does not meet the
capability-relative threshold. Unverified is not a polite synonym for Working
and is not evidence of failure.

### Absent

No implementation path provides the accepted outcome at the reviewed revision,
or the product explicitly records it as not implemented. Tests for adjacent
mechanisms do not change an Absent verdict.

## Constraint annotation

Constraints are separate from behavior verdicts because a capability can work
inside a narrow envelope or fail despite a broad one.

- **None identified** — no material narrowing beyond the accepted support
  envelope was found.
- **Documented external constraint** — upstream, provider, platform, sandbox,
  or permission behavior narrows the envelope and the product states that
  limitation accurately.
- **Undocumented external constraint** — an external reality narrows what users
  were promised but the public contract does not state it adequately.
- **Self-imposed implementation constraint** — the implementation narrows a
  capability even though the platform could support the accepted outcome.

Examples:

- Linux-only socket support can be **Working + documented external/platform
  constraint** when the support envelope explicitly excludes other platforms.
- A public cross-platform promise with no Windows transport is **Partial +
  undocumented constraint**.
- Agent messaging with code and fake tests but no current live provider run is
  **Unverified + provider/sandbox constraint**.

## Cross-cutting findings

Do not average correctness, durability/recovery, observability/evidence,
partial-failure safety, and portability into one score. Record findings on each
axis. An outcome-family summary reports:

- the count and names of Working, Partial, Unverified, and Absent capabilities;
- material constraint annotations;
- the weakest important axis and why;
- contradictory evidence or environmental gaps.

This preserves useful statements such as “normal start is Working, interrupted
start recovery is Partial, and provider portability is Unverified” instead of
calling the entire family “mostly working.”

## Contrary evidence and precedence

- Fresh live behavior in the same environment outranks a source-derived
  prediction about what should happen.
- Pinned source outranks narrative documentation for attribution and internal
  ownership.
- A higher-fidelity test at the real seam outranks a fake-backed test of the
  same claim.
- Newer evidence does not automatically outrank older evidence if it tests a
  different revision, environment, provider, or support envelope.
- Any unresolved contrary evidence at equal or greater authority prevents a
  Working verdict.

## Relationship to bug diagnosis

The audit records suspected defects and the evidence that exposed them; it does
not theorize about causes. A finding enters `/diagnosing-bugs` only after a
tight, agent-runnable command exists that can assert the exact symptom. Until
then, mark the capability Partial or Unverified as evidence warrants and record
the missing feedback loop explicitly.

## Decision

The feature audit uses capability-relative proof thresholds, two-dimensional
behavior-plus-constraint reporting, reproducible runtime evidence, separate
cross-cutting findings, and explicit contrary evidence. No entire outcome
family receives a synthetic average verdict.
