#!/usr/bin/env bash
set -u
set -o pipefail

ROOT=/home/caio/Projects/herdr-agent-team
EVIDENCE=/home/caio/Projects/herdr-agent-team/docs/reviews/evidence/stage0
BIN=/home/caio/Projects/herdr-agent-team/target/debug/herdr-agent-team
HERDR=/home/caio/.local/bin/herdr
STATE=/home/caio/.local/state/herdr/plugins/caioniehues.agent-team
CONFIG_BASE=/home/caio/Projects/herdr-agent-team/docs/reviews/evidence/stage0/config

if [[ $# -ne 1 || ! $1 =~ ^run[12]$ ]]; then
  printf 'usage: %s run1|run2\n' "$0" >&2
  exit 64
fi
RUN_NAME=$1
OUT="$EVIDENCE/$RUN_NAME"
TEAM="stage0-${RUN_NAME}-$(date +%s)"
SPEC="$OUT/team.toml"
BRIEF_CLAUDE="$OUT/claude-brief.md"
BRIEF_CODEX="$OUT/codex-brief.md"
ADOPT_BRIEF="$OUT/adopt-brief.md"
TRANSCRIPT="$OUT/transcript.log"
RUN_DIR=
ADOPT_WORKSPACE=
ADOPT_PANE=
CREATED_WORKSPACES=()
WORKTREE_PATHS=()

mkdir -p "$OUT" "$CONFIG_BASE"
: > "$TRANSCRIPT"

export HERDR_ENV=1
export HERDR_BIN_PATH="$HERDR"
export HERDR_PLUGIN_STATE_DIR="$STATE"
export HERDR_PLUGIN_CONFIG_DIR="$CONFIG_BASE"

record() {
  printf '\n$ %q' "$1" | tee -a "$TRANSCRIPT"
  shift
  printf ' %q' "$@" | tee -a "$TRANSCRIPT"
  printf '\n' | tee -a "$TRANSCRIPT"
  "$@" 2>&1 | tee -a "$TRANSCRIPT"
  return "${PIPESTATUS[0]}"
}

run_logged() {
  printf '\n$' | tee -a "$TRANSCRIPT"
  printf ' %q' "$@" | tee -a "$TRANSCRIPT"
  printf '\n' | tee -a "$TRANSCRIPT"
  "$@" 2>&1 | tee -a "$TRANSCRIPT"
  return "${PIPESTATUS[0]}"
}

ping() {
  printf '%s %s: %s\n' "$(date --iso-8601=seconds)" "$RUN_NAME" "$1" >> "$EVIDENCE/STATUS.log"
}

capture_created_resources() {
  [[ -n "$RUN_DIR" && -f "$RUN_DIR/run.toml" ]] || return 0
  mapfile -t CREATED_WORKSPACES < <(sed -n 's/^workspace_id = "\([^"]*\)"/\1/p' "$RUN_DIR/run.toml")
  mapfile -t WORKTREE_PATHS < <(sed -n 's/^worktree_path = "\([^"]*\)"/\1/p' "$RUN_DIR/run.toml")
}

cleanup() {
  set +e
  capture_created_resources
  if [[ -n "$RUN_DIR" && -f "$RUN_DIR/run.toml" ]]; then
    "$BIN" kill "$RUN_DIR" >> "$OUT/cleanup.log" 2>&1
  fi
  if [[ -n "$ADOPT_WORKSPACE" ]]; then
    "$HERDR" workspace close "$ADOPT_WORKSPACE" >> "$OUT/cleanup.log" 2>&1
  fi
  for workspace in "${CREATED_WORKSPACES[@]}"; do
    "$HERDR" workspace close "$workspace" >> "$OUT/cleanup.log" 2>&1
  done
  for worktree in "${WORKTREE_PATHS[@]}"; do
    while IFS= read -r workspace; do
      [[ -n "$workspace" ]] && "$HERDR" workspace close "$workspace" >> "$OUT/cleanup.log" 2>&1
    done < <("$HERDR" workspace list | jq -r --arg path "$worktree" '.result.workspaces[] | select(.worktree.checkout_path == $path) | .workspace_id')
    if [[ -d "$worktree" ]]; then
      git -C "$ROOT" worktree remove --force "$worktree" >> "$OUT/cleanup.log" 2>&1
    fi
  done
  git -C "$ROOT" branch --list "stage0/${TEAM}-*" | sed 's/^[*+ ]*//' | while IFS= read -r branch; do
    [[ -n "$branch" ]] && git -C "$ROOT" branch -D "$branch" >> "$OUT/cleanup.log" 2>&1
  done
}
trap cleanup EXIT

cat > "$CONFIG_BASE/agents.toml" <<'CONFIG'
[claude]
command = ["claude"]
submit_verify = true
reads_agents_md = "pointer"
queues_midturn = true

[codex]
command = ["codex", "--dangerously-bypass-approvals-and-sandbox"]
submit_verify = true
reads_agents_md = "native"
queues_midturn = false
CONFIG

cat > "$BRIEF_CLAUDE" <<EOF
# Stage 0 Claude live worker
Follow the generated worker protocol exactly. First send the god an attention message saying "${RUN_NAME} claude started" using the protocol's msg verb. Stay working for at least 25 seconds so the god can submit a live mid-turn instruction. Then write the required durable report containing the exact tokens "${RUN_NAME} claude report" and "immediate message observed" plus any received instruction, send the god "${RUN_NAME} claude report finalized", and emit the required completion sentinel. Do not use git.
EOF

cat > "$BRIEF_CODEX" <<EOF
# Stage 0 Codex live worker
Follow the generated worker protocol exactly. First send the god an attention message saying "${RUN_NAME} codex started" using the protocol's msg verb. Stay working for at least 35 seconds so the god can create an outbox entry while you are working. Then write the required durable report containing the exact tokens "${RUN_NAME} codex report" and "queued message observed" plus any received instruction, send the god "${RUN_NAME} codex report finalized", and emit the required completion sentinel. Do not use git.
EOF

cat > "$ADOPT_BRIEF" <<EOF
# Stage 0 adopted worker
Read the generated protocol, write the required report containing "${RUN_NAME} adopted report", send the god "${RUN_NAME} adopted worker ready", and emit the completion sentinel. Do not use git.
EOF

cat > "$SPEC" <<EOF
name = "$TEAM"
topology = "star"
cwd = "$ROOT"
setup = ["test -f Cargo.toml"]

[god]
target = "self"

[[workers]]
name = "claude-worker"
agent = "claude"
role = "live e2e worker"
task = "prove immediate pane submission and report push"
worktree = true
branch = "stage0/${TEAM}-claude"
brief = "$BRIEF_CLAUDE"

[[workers]]
name = "codex-worker"
agent = "codex"
role = "live e2e worker"
task = "prove queued outbox drain and report push"
worktree = true
branch = "stage0/${TEAM}-codex"
brief = "$BRIEF_CODEX"
EOF

ping "preflight and version capture"
{
  printf 'date='; date --iso-8601=seconds
  printf 'plugin_head='; git -C "$ROOT" rev-parse HEAD
  printf 'plugin_manifest_version='; sed -n 's/^version = "\([^"]*\)"/\1/p' "$ROOT/herdr-plugin.toml" | head -1
  printf '%s\n' 'git_status_short_begin'; git -C "$ROOT" status --short; printf '%s\n' 'git_status_short_end'
  "$HERDR" --version
  claude --version
  codex --version
  uname -a
  printf 'HERDR_BIN_PATH=%s\n' "$HERDR_BIN_PATH"
  printf 'HERDR_PANE_ID=%s\n' "${HERDR_PANE_ID:-unset}"
  printf 'HERDR_SOCKET_PATH=%s\n' "${HERDR_SOCKET_PATH:-default}"
} > "$OUT/environment.txt" 2>&1

run_logged "$HERDR" workspace list > "$OUT/workspaces-before.json"
run_logged "$HERDR" pane current --current > "$OUT/god-pane-before.json"

ping "spawn file spec with worktrees and both providers"
set +e
"$BIN" spawn "$SPEC" > "$OUT/spawn.log" 2>&1
SPAWN_RC=$?
set -e
cat "$OUT/spawn.log" >> "$TRANSCRIPT"
printf 'spawn_rc=%s\n' "$SPAWN_RC" | tee -a "$TRANSCRIPT"
RUN_DIR=$(sed -n 's/^team run created: //p' "$OUT/spawn.log" | tail -1)
if [[ -z "$RUN_DIR" ]]; then
  RUN_DIR=$(ls -1dt "$STATE/runs/${TEAM}-"* 2>/dev/null | head -1 || true)
fi
printf 'run_dir=%s\n' "$RUN_DIR" | tee -a "$TRANSCRIPT"
if [[ -z "$RUN_DIR" || ! -f "$RUN_DIR/run.toml" ]]; then
  printf 'BLOCKER: spawn did not produce a run directory; remaining run-scoped steps cannot execute\n' | tee -a "$TRANSCRIPT"
  exit 0
fi
capture_created_resources
cp "$RUN_DIR/run.toml" "$OUT/run-after-spawn.toml"
cp -a "$RUN_DIR/protocols" "$OUT/protocols-after-spawn"
run_logged "$BIN" status "$RUN_DIR" --json | tee "$OUT/status-after-spawn.json"

ping "resume idempotency probe"
run_logged "$BIN" spawn --resume "$RUN_DIR" | tee "$OUT/resume.log" || true

ping "msg immediate and queued drain"
run_logged "$BIN" msg claude-worker "${RUN_NAME} immediate message from god" --run "$RUN_DIR" | tee "$OUT/msg-immediate.log" || true
run_logged "$BIN" msg codex-worker "${RUN_NAME} queued message from god" --run "$RUN_DIR" | tee "$OUT/msg-queued.log" || true
mkdir -p "$OUT/outbox-before-drain"
cp -a "$RUN_DIR/outbox/." "$OUT/outbox-before-drain/" 2>/dev/null || true
run_logged "$HERDR" pane read "${CREATED_WORKSPACES[0]}:p1" --source recent-unwrapped --lines 160 > "$OUT/claude-pane-midturn.txt" || true
run_logged "$HERDR" pane read "${CREATED_WORKSPACES[1]}:p1" --source recent-unwrapped --lines 160 > "$OUT/codex-pane-midturn.txt" || true

ping "worker reports pointer push and team wait"
run_logged "$BIN" wait --until all-reports --run "$RUN_DIR" --timeout 180 --json | tee "$OUT/wait-all-reports.json" || true
sleep 3
cp "$RUN_DIR/run.toml" "$OUT/run-after-reports.toml"
cp -a "$RUN_DIR/inbox" "$OUT/inbox-after-reports"
mkdir -p "$OUT/outbox-after-drain"
cp -a "$RUN_DIR/outbox/." "$OUT/outbox-after-drain/" 2>/dev/null || true
run_logged "$HERDR" pane read "${CREATED_WORKSPACES[0]}:p1" --source recent-unwrapped --lines 240 > "$OUT/claude-pane-final.txt" || true
run_logged "$HERDR" pane read "${CREATED_WORKSPACES[1]}:p1" --source recent-unwrapped --lines 240 > "$OUT/codex-pane-final.txt" || true
run_logged "$HERDR" pane read "${HERDR_PANE_ID}" --source recent-unwrapped --lines 240 > "$OUT/god-pane-after-reports.txt" || true
run_logged "$BIN" inbox --run "$RUN_DIR" --json | tee "$OUT/inbox.json" || true

ping "adopt release probe"
ADOPT_CREATE=$($HERDR workspace create --cwd "$ROOT" --label "${TEAM}-adopt" --no-focus)
printf '%s\n' "$ADOPT_CREATE" > "$OUT/adopt-workspace-create.json"
ADOPT_WORKSPACE=$(printf '%s\n' "$ADOPT_CREATE" | jq -r '.result.workspace.workspace_id')
ADOPT_PANE=$(printf '%s\n' "$ADOPT_CREATE" | jq -r '.result.workspace.active_pane_id // .result.workspace.pane_id // empty')
if [[ -z "$ADOPT_PANE" ]]; then
  ADOPT_PANE=$($HERDR pane list --workspace "$ADOPT_WORKSPACE" | jq -r '.result.panes[0].pane_id')
fi
run_logged "$HERDR" pane run "$ADOPT_PANE" "codex --dangerously-bypass-approvals-and-sandbox" | tee "$OUT/adopt-launch.log" || true
run_logged "$HERDR" wait agent-status "$ADOPT_PANE" --status idle --timeout 90000 | tee "$OUT/adopt-wait-idle.log" || true
run_logged "$BIN" adopt "$ADOPT_PANE" --name adopted-worker --role "live adopted worker" --brief "$ADOPT_BRIEF" --run "$RUN_DIR" | tee "$OUT/adopt-first.log" || true
run_logged "$BIN" adopt "$ADOPT_PANE" --name adopted-worker --role "live adopted worker" --brief "$ADOPT_BRIEF" --run "$RUN_DIR" | tee "$OUT/adopt-second.log" || true
sleep 8
cp "$RUN_DIR/run.toml" "$OUT/run-after-adopt.toml"
run_logged "$BIN" kill "$RUN_DIR" --worker adopted-worker | tee "$OUT/release.log" || true
cp "$RUN_DIR/run.toml" "$OUT/run-after-release.toml"
run_logged "$HERDR" workspace get "$ADOPT_WORKSPACE" > "$OUT/adopt-workspace-after-release.json" || true
run_logged "$HERDR" pane read "$ADOPT_PANE" --source recent-unwrapped --lines 200 > "$OUT/adopt-pane-after-release.txt" || true

ping "dirty worktree kill and cleanup"
DIRTY_PATH=${WORKTREE_PATHS[0]}
printf '%s\n' "stage0 deliberate dirty marker $RUN_NAME" > "$DIRTY_PATH/stage0-dirty-marker.txt"
git -C "$DIRTY_PATH" status --short > "$OUT/dirty-worktree-status.txt"
set +e
"$BIN" kill "$RUN_DIR" --remove-worktrees > "$OUT/kill-dirty-refusal.log" 2>&1
DIRTY_KILL_RC=$?
set -e
printf 'dirty_kill_rc=%s\n' "$DIRTY_KILL_RC" >> "$OUT/kill-dirty-refusal.log"
run_logged "$BIN" kill "$RUN_DIR" | tee "$OUT/kill-final.log" || true
cp "$RUN_DIR/run.toml" "$OUT/run-after-kill.toml"
run_logged "$HERDR" workspace list > "$OUT/workspaces-after-kill.json"
run_logged "$HERDR" pane list --workspace "$HERDR_WORKSPACE_ID" > "$OUT/god-workspace-panes-after-kill.json"

ping "run complete"
printf '%s\n' "$RUN_DIR" > "$OUT/RUN_DIR"
printf 'E2E_RUN_COMPLETE %s\n' "$RUN_NAME" | tee -a "$TRANSCRIPT"
