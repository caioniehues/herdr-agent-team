# Feature reality review — historical-intent evidence ledger

Date: 2026-07-15  
Wayfinder ticket: [Establish the historical-intent evidence ledger](https://github.com/caioniehues/herdr-agent-team/issues/40)

## Purpose

This ledger defines which artifacts may answer “what was this plugin intended to
be?” and how much authority each artifact carries. It prevents the feature
reality audit from treating brainstorms as commitments, implementation plans as
product decisions, or current documentation as a pristine historical record.

## Review terms

- **Brainstormed intent** — a possibility discussed before a commitment. It is
  evidence of the design space, not an unmet requirement.
- **Accepted scope** — behavior or a constraint explicitly committed through a
  contemporaneous specification, accepted ADR, or later recorded decision.
- **Superseded direction** — formerly accepted scope that a later explicit
  decision replaced or cancelled.
- **Implementation plan** — a proposed decomposition of accepted scope into
  work. It can clarify interpretation but cannot expand product scope by itself.
- **Shipped claim** — documentation or a work report asserting that behavior
  exists. It is a claim to verify, not proof of behavior.
- **Observed behavior** — behavior established by current source, an automated
  test, or a live run, with the evidence type stated.
- **Reality constraint** — an observed upstream, agent-CLI, sandbox, or
  operational condition that limits or reshapes feasible behavior.

These are review terms, not product-domain concepts, so they do not belong in
`CONTEXT.md`.

## Authority order for intended behavior

Use the narrowest applicable source. Later sources override earlier sources only
when they explicitly amend, supersede, cancel, or accept new scope.

1. **Accepted ADR at the relevant historical point.** Highest authority for the
   decision it owns and its stated trade-off.
2. **Contemporaneous buildable specification.** Authority for committed behavior
   not owned more specifically by an ADR.
3. **Recorded later decision.** An accepted ADR, decision-bearing issue/comment,
   or explicit specification amendment can add or replace scope after the
   initial cut.
4. **Issue/brief implementation contract.** Authority for how accepted scope was
   decomposed, but not independent authority to invent product behavior.
5. **Planning notes and handoffs.** Contextual evidence for alternatives,
   constraints, and chronology; never sufficient alone to establish accepted
   scope.
6. **Research and brainstorm artifacts.** Evidence of options considered. An idea
   becomes accepted scope only through one of levels 1–3.

Commit timestamps establish chronology, not authority. A later README sentence
does not silently supersede an ADR.

## Historical source ledger

| Artifact | Historical role | Authority and use | Important limitation |
|---|---|---|---|
| `docs/spec.md` at commit `2416889` | Earliest repository snapshot of the v1 buildable scope, committed 2026-07-14 | Primary baseline for the original accepted feature cut and Definition of Done | It says it was distilled from a grilling session in the limux repo; the underlying interview transcript is not present here, so pre-commit brainstorms cannot be reconstructed completely |
| `docs/adr/0001`–`0007` at commit `2416889` | Contemporaneous accepted product and architecture decisions | Primary authority for plugin placement, god-led reporting, topology, worktrees, Rust/repo shape, launcher configuration, and the thin-v1 cut | Later amendments must be read from their own historical revisions; current text is not automatically the original wording |
| Git history beginning at `2416889` | Chronology and immutable versions of living documents | Required for comparing the original spec/ADRs with later amendments; use `git show <commit>:<path>` rather than current files when asking what was scoped at a past point | Commit messages summarize work but are not sufficient behavioral proof |
| `.scratch/team-v1/spec.md` | Pointer and orchestration contract for the original 18-ticket build | Useful for build sequencing, ownership rules, and the coordinator’s interpretation of the spec | It points back to `docs/spec.md`; it is not an independent feature specification |
| `.scratch/team-v1/issues/01`–`18` and `briefs/` | Concrete implementation decomposition and later corrective tickets | Clarifies what workers were asked to build and which gaps were recognized during dogfooding | A ticket can reflect an implementation tactic or bug fix rather than original product intent |
| `.scratch/team-v1/reports/` | Worker-reported implementation outcomes | Candidate shipped claims and pointers to files/tests worth verifying | Self-reports are not independent proof and may predate integration review or later fixes |
| `docs/adr/0008` and specification section 11 | Accepted 2026-07-15 expansion for the `msg` verb and outbox | Primary authority for the later messaging commitment | Postdates the initial v1 cut and must be reported as evolved scope, not original scope |
| `docs/adr/0009` and specification section 12 | Accepted 2026-07-15 expansion for adoption/release semantics | Primary authority for the later adoption commitment | Postdates the initial v1 cut |
| `docs/adr/0010` | Accepted evidence hierarchy for upstream Herdr claims | Governs authority tags and how current constraints should be verified | It governs evidence; it does not itself prove each upstream claim remains current |
| `docs/adr/0011` and related issue/commits | Accepted experimental direct-socket direction | Primary authority for backend selection, fallback, validation, and bounded lifecycle expectations | Its implementation status evolved across several commits; behavior must be checked at the released revision |
| GitHub issues and resolution comments created after publication | Decision and defect history for post-v1 work | Decision-bearing comments and accepted issue text can establish later scope; closed issues identify claimed completion | Closure alone is not behavioral proof; comments may revise the issue body |
| `docs/research/*2026-07-15*.md` and `.planning/2026-07-15-upstream-integration-research/` | Research wave that exposed upstream capabilities, overlap, and constraints | Primary-input research for explaining why roadmap choices changed | Research proposes opportunities; only explicit later decisions turn them into scope. Drift-prone claims require current verification |
| `docs/spec.md` section 8 at the released revision | Living post-v1 roadmap with shipped/cancelled annotations | Index of later direction and candidate links to decision sources | Mixes roadmap, shipped status, bug history, cancellation, and optional ideas; each entry must be decomposed before comparison |
| `docs/learnings/wave5`–`wave8` | Retrospective dogfooding and release observations | Strong contextual evidence for constraints, review residue, and claimed live wins | Retrospective narrative is not a substitute for retained runtime artifacts |
| `.planning/**`, `.scratch/*handoff*.md`, and PR-review notes | Operational history and contemporaneous reasoning | Useful secondary evidence when a divergence needs explanation | Mutable, sometimes superseded, and often written for coordination rather than product definition |
| Current `README.md`, examples, manifest, and shipped skills | Current public product promise | Defines claims users are presently invited to rely on and therefore must be audited | Describes the product after multiple scope expansions; not evidence of original intent or proof that claims work |

## Non-intent evidence reserved for the comparison

The following sources answer what exists, not what was intended:

- released source at tag `v1.0.0` and the interfaces exercised by callers;
- automated tests and fixtures, with their seam and untested assumptions noted;
- `herdr-plugin.toml`, examples, and generated protocol golden files;
- retained run state, reports, logs, and release smoke-test artifacts;
- fresh live runs against the currently installed Herdr and supported agent CLIs;
- current upstream source/schema and official CLI behavior.

The later proof-standard decision will determine how these evidence types map to
verdicts such as working, partial, constrained, unverified, or absent.

## Required extraction procedure

1. Freeze the comparison revision at tag `v1.0.0` (`aa0c0e0`) unless a later
   review decision chooses another point.
2. Reconstruct the initial accepted v1 cut from `git show 2416889:docs/spec.md`
   and ADRs 0001–0007 at that commit.
3. Build a chronological amendment register from later accepted ADRs,
   decision-bearing issue comments, and explicit spec changes.
4. Record brainstorm/research ideas separately; never score them as missing
   unless the amendment register shows acceptance.
5. Treat current README/spec “shipped” language and worker reports as claims to
   verify against implementation evidence.
6. For every divergence, cite both sides: the artifact establishing intent and
   the artifact establishing observed reality or constraint.

## Decision

The feature audit will use the initial committed spec plus contemporaneous ADRs
as its original accepted-scope baseline, then apply an explicit chronological
amendment register. No surviving artifact is assumed to be the missing
brainstorm transcript. Current documentation and reports supply claims, while
source, tests, live observations, and current upstream evidence establish
reality at separately stated confidence levels.
