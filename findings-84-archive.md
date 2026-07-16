# Findings — Issue #84

## Live herdr CLI verification (2026-07-16, herdr 0.7.4 installed at ~/.local/bin/herdr)

`docs/herdr-api-schema.snapshot.json` was stale relative to the installed
0.7.4 binary — it had no `tokens` field at all on `PaneReportMetadataParams`.
Re-fetched via `herdr api schema --json` and re-snapshotted.

### `herdr pane --help` (live, verbatim relevant line)

```
herdr pane report-metadata <pane_id> --source ID [--agent LABEL] [--applies-to-source ID] [--title TEXT|--clear-title] [--display-agent TEXT|--clear-display-agent] [--state-label STATUS=TEXT] [--clear-state-labels] [--token NAME=VALUE] [--clear-token NAME] [--seq N] [--ttl-ms N]
```

Confirms: `--token NAME=VALUE` is a genuine, distinct flag from
`--state-label STATUS=TEXT` — CONTEXT.md's "Sidebar token" definition
(`--token name=value`, rendered as `$name`) is accurate and live. `--token`
repeats for multiple tokens (same shape as `--state-label`, which the
existing legacy code already treats as repeatable-in-spirit via one call
per label).

### Live schema: `PaneReportMetadataParams.tokens`

```json
"tokens": {
  "additionalProperties": { "type": ["string", "null"] },
  "maxProperties": 16,
  "propertyNames": { "pattern": "^[A-Za-z0-9_-]{1,32}$" },
  "type": "object"
}
```

- `maxProperties: 16` — matches BRIEF's "≤16 tokens/report" exactly; my
  `tokens::MAX_TOKENS_PER_REPORT = 16` (step 4) is correct.
- Name pattern `^[A-Za-z0-9_-]{1,32}$` — my token names (`task`, `status`,
  `model`) all satisfy this.
- Value type nullable (`string | null`) — null presumably pairs with
  `--clear-token NAME` to clear one token without `--clear-*` wiping all.
  Not implementing clear semantics in step 5 (BRIEF doesn't ask for it).
- No `maxLength` on token values in the schema — the 80-char truncation
  (BRIEF) is a client-side (our) convention, not schema-enforced. Kept as
  designed in step 4.

Old snapshot also had `custom_status`/`clear_custom_status` on this params
object; the live 0.7.4 schema no longer has them. Not investigated further
— out of scope (legacy `metadata.rs` schema-gate tests use inline fixture
JSON, unaffected either way; `MetadataCapabilities::custom_status` will
just always resolve false against a live 0.7.4 schema fetch, degrading
gracefully as designed).

### Live `herdr agent list` (confirms pane→session resolution path)

```json
{
  "agent": "claude",
  "agent_session": {
    "agent": "claude", "kind": "id", "source": "herdr:claude",
    "value": "2f2c1767-64d9-4fc5-9083-0441be443cb8"
  },
  "pane_id": "w1A:pX",
  "tokens": { "step": "logging wrapper + toy team", "task": "recon: tmux verb inventory" },
  ...
}
```

`agent_session.value` is a Claude Code session UUID in the exact same
format as `TeamConfig.lead_session_id` (parsed from `config.json`'s
`leadSessionId`, verified against a real `~/.claude/teams/*/config.json`
sample: `"leadSessionId": "620e0f77-a2a2-4360-b8e0-2b79ced4d59e"`). This is
the resolution mechanism: match `agent_session.value == lead_session_id` in
`agent_list()` output to find the herdr `pane_id` hosting the team lead.

Also notable: this live output already shows a `tokens` object in use on a
real pane (from an earlier prototype/spike session) — independent
confirmation the mechanism works end-to-end today, pre-shim.

### Live end-to-end smoke test (2026-07-16, `target/release/herdmates pump-board`)

Ran the built binary against the real `~/.claude/teams/` directory (15
teams on disk). 3 resolved to live herdr panes via session-id match
(`session-2f2c1767` → `w1A:pX`, `session-5c63de32` → `w1A:p12`,
`session-b065bc0e` → `w1A:pY`, the pane running this very session). Exit
0, no errors.

Verified real leads carry no `prompt` field in `config.json` (confirmed
directly on `session-b065bc0e`'s lead member — no `prompt` key at all), so
`teammate_tokens` correctly emits only `status=idle` for them (no `task`
token — matches the design, non-empty-task-only rule from step 4).

`herdr pane get w1A:pY` afterward showed a merged `tokens` map containing
`task`/`step` values NOT written by this pump (pre-existing values from a
separate, already-running publisher on this pane — unrelated to
`herdmates-board`) alongside no visible conflict. Confirms the schema's
per-source token model is additive/non-destructive: my `--source
herdmates-board --token status=idle` call coexisted cleanly with whatever
else already reports tokens on that pane. Good real-world evidence the
pump is safe to wire into a live event handler later (step 6) without
clobbering other integrations.

### Resolution scope decision

Team members other than the lead have no independently recorded session ID
anywhere in `config.json` — only `agentId`/`name`/`tmuxPaneId` (a Claude
Code–internal tmux pane reference meaningless to herdr pre-shim). So:

- **Lead** (`is_lead == true`, backed by `TeamConfig.lead_session_id`):
  resolvable via `agent_list()` session-id match → real herdr pane.
- **Everyone else**: unconditionally unresolvable pre-shim → skip, never
  error (ADR-0012's explicit degrade policy).

## Sidebar-rows syntax (step 7, ground truth from live `~/.config/herdr/config.toml`)

Found a live `[ui.sidebar.agents]` table already on this machine, tagged
"herdmates D1 prototype (remove freely)" — evidently left by the earlier
prototype spike referenced in ADR-0012. Used its exact syntax rather than
guessing:

```toml
[ui.sidebar.agents]
rows = [
  ["state_icon", "agent", "state_text"],
  ["$task"],
  ["$step"],
]
```

Confirms: `rows` is a list of rows, each row a list of cells; builtin
tokens are bare names (`state_icon`, `agent`, `state_text` — the
coordinator's supplied finding that the builtin is `state_text` not
`state_label` checks out directly against this live config); custom
tokens (from `pane report-metadata --token`) use a `$name` prefix. One
single global `[ui.sidebar.agents]` table, not an array of per-agent-id
tables — "global" in the coordinator's finding means exactly this: the
same row template applies to every detected agent pane regardless of
which agent (claude/codex/etc.) is running there.

Did not independently reproduce the "invalid token → reload-config
`partial` + silent stale UI" claim: `herdr config check` takes no path
argument (always validates the live `~/.config/herdr/config.toml`), so
reproducing it would have meant swapping out the coordinator's live,
in-use sidebar config mid-session — an unnecessary risk for a claim
already given as coordinator-verified ground truth (also posted to issue
#84). Took it as authoritative per the coordinator's explicit framing.

## Sample team file fixtures (captured 2026-07-16 from live `~/.claude/teams/`)

Used to build `tests/fixtures/teamfiles/*` in step 3. Config wire format:
camelCase (`leadSessionId`, `agentId`, `agentType`, `tmuxPaneId`,
`backendType`, `isActive`). Lead member: `agentType: "team-lead"`,
`tmuxPaneId: "leader"`, `backendType: "in-process"`. Split-pane teammate:
`tmuxPaneId: "%1"`, `backendType: "tmux"`, `isActive: true`. Inbox files
are plain JSON arrays (`[]` when empty).
