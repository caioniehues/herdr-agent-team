//! herdmates — Herdr plugin binary.
//!
//! Legacy subcommands (`adopt`, `spawn`, `status`, `kill`, `msg`, `on-agent-status`)
//! are frozen at v1.1.0 (ADR-0012); new surfaces are `pump-board` (D1) and
//! focus-pane (D3).

use std::fmt::Display;
use std::process::ExitCode;

use herdmates::*;

fn main() -> ExitCode {
    paths::hydrate_environment();
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_default();
    let args = args.collect::<Vec<_>>();

    match command.as_str() {
        "adopt" => exit(adopt::adopt_command(&args)),
        "board" => exit(board::board_command(&args)),
        "open-report" => exit(board::open_report_command(&args)),
        "spawn" => exit(spawn::spawn_command(&args)),
        "status" => exit(status_kill::status_command(&args)),
        "kill" => exit(status_kill::kill_command(&args)),
        "inbox" => exit(god_cli::inbox_command(&args)),
        "report" => exit(god_cli::report_command(&args)),
        "wait" => match god_cli::wait_command(&args) {
            Ok(verdict) => ExitCode::from(verdict.exit_code()),
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        },
        "msg" => exit(msg::msg_command(&args)),
        "on-agent-status" => exit(hook::hook_command()),
        "pump-board" => exit(pump::pump_board_command(&args)),
        "teammux-launch" => exit(teammux_launch::teammux_launch_command(&args)),
        "jump" => exit(jump::jump_command(&args)),
        "focus" => exit(focus_pane::focus_pane_command(&args)),
        // Issue #97 stage 2 (ADR-0013 §93 stage 2, docs/spec.md §4): minimal
        // recorder — polls the gather + signal-engine pipeline and appends
        // classified-observation deltas to an append-only JSONL log.
        "record" => exit(recorder::record_command(&args)),
        // Issue #98 stage 3 (ADR-0013 §93 stage 3, docs/spec.md §4):
        // read-only full-screen TUI plugin pane over the gather +
        // signal-engine pipeline. Distinct from the legacy `board`
        // subcommand (frozen v1.1.0, different data model).
        "pane-board" => exit(pane_board::pane_board_command(&args)),
        "" | "help" | "--help" | "-h" => {
            eprintln!(
                "herdmates <adopt|board|spawn|status|kill|inbox|report|wait|msg|open-report|on-agent-status|pump-board|teammux-launch|jump|focus|record|pane-board>"
            );
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("unknown subcommand: {other}");
            ExitCode::FAILURE
        }
    }
}

fn exit(result: Result<(), impl Display>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
