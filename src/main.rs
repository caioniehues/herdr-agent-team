//! herdr-agent-team — Herdr plugin binary.
//!
//! Subcommand dispatch follows `docs/spec.md` section 1: the CLI half is
//! `spawn`, `status`, and `kill`; the event half is `on-agent-status`.

use std::fmt::Display;
use std::process::ExitCode;

pub mod agents_md;
pub mod herdr;
pub mod hook;
pub mod launcher;
pub mod run;
pub mod spawn;
pub mod spec;
pub mod status_kill;
pub mod types;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_default();
    let args = args.collect::<Vec<_>>();

    match command.as_str() {
        // Ticket 02 owns the dry-run implementation. Ticket 07 will replace
        // this one dispatch edge when real spawning lands.
        "spawn" => exit(spec::spawn_command(&args)),
        "status" => exit(status_kill::status_command(&args)),
        "kill" => exit(status_kill::kill_command(&args)),
        "on-agent-status" => exit(hook::hook_command()),
        "" | "help" | "--help" | "-h" => {
            eprintln!("herdr-agent-team <spawn|status|kill|on-agent-status>");
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
