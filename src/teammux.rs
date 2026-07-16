//! `teammux` dispatcher core (issue #85 commit 3, extended commits 4-5).
//!
//! One reusable dispatch entry point, per cmux's `__tmux-compat` pattern
//! (`docs/research/cmux-comparative-2026-07-16/REPORT.md`, correction c):
//! [`dispatch`] takes a parsed call plus its dependencies (a [`HerdrApi`]
//! implementor and a loaded [`IdMap`]) and returns a [`DispatchOutcome`],
//! with no process-boundary I/O of its own — real herdr calls and idmap
//! file reads happen through the injected dependencies, so tests can supply
//! `crate::herdr::test_support::FakeHerdr` (the same recording-fake process
//! trait pattern the rest of the codebase already uses) and a temp-file-
//! backed `IdMap`. [`run`] is the only code that touches real stdio, a real
//! `HerdrClient`, and the real `TEAMMUX_STATE_PATH` file.
//!
//! Commit 3 wired the three static probes from
//! `docs/research/spike-tmux-verbs-2026-07-16/REPORT.md` §3 (`show -Av
//! mouse`, `show -gv focus-events`, `display-message -p #{client_termtype}`).
//! Commit 4 adds the structural reads: `list-panes -F #{pane_id}` (via
//! `herdr pane list` filtered client-side by tab id — live-verified: `herdr
//! tab get` returns tab metadata, not a pane roster, so `pane list` is the
//! only way to enumerate a tab's panes) and `display-message -p
//! #{window_id}` (via `herdr pane get`'s `tab_id` field). Both translate a
//! herdr id back to its tmux `%N`/`@N` id through [`IdMap::reverse_lookup`]
//! before printing — output must always speak in tmux's id space, never
//! herdr's. A herdr pane found in the target tab with no idmap registration
//! is a loud error (inconsistent state), not a silently-dropped line: by the
//! time teammux runs, the launcher has already registered every pane it
//! created, so an orphan means something is wrong, not that it's optional.
//!
//! Commit 5 adds `split-window`: `herdr pane split` targeting the resolved
//! herdr pane, then [`IdMap::allocate`] registers the new pane under a
//! freshly minted `%N` (ids are allocated freely — no real tmux session to
//! shadow, cmux comparative research correction a) before printing it.
//!
//! Commit 6 adds the lifecycle verbs: `respawn-pane -k` (launch the real
//! teammate process via `herdr pane run` — herdr has no separate "respawn"
//! primitive), `kill-pane` (`herdr pane close` + [`IdMap::remove`], so a
//! torn-down pane stops resolving), `select-pane -T` (`herdr pane rename`),
//! and `resize-pane -x` (`herdr pane resize` — a documented, one-way
//! mapping from tmux's absolute-size target onto herdr's directional
//! border-move model, see `resize_pane`'s doc comment).
//!
//! Every other successfully-*parsed* verb (styling — commit 7) is still a
//! deliberate, labeled "not yet implemented" placeholder, not a
//! translate-don't-emulate failure: it is recognized by `tmuxargs`, just
//! not yet handled here.

use crate::herdr::HerdrApi;
use crate::idmap::IdMap;
use crate::tmuxargs::{self, DisplayField, ParseError, TmuxId, Verb};
use std::process::ExitCode;

/// The result of dispatching one parsed tmux call, before any process I/O.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchOutcome {
    /// Print `stdout` to the real stdout and exit 0.
    Ok { stdout: String },
    /// Print `message` to stderr and exit nonzero.
    Error { message: String },
}

/// Dispatch one parsed call against real (or faked) herdr + idmap state.
pub fn dispatch<H: HerdrApi>(
    herdr: &H,
    idmap: &IdMap,
    call: tmuxargs::ParsedCall,
) -> DispatchOutcome {
    match call.verb {
        Verb::ShowMouse => DispatchOutcome::Ok {
            stdout: "off\n".to_owned(),
        },
        Verb::ShowFocusEvents => DispatchOutcome::Ok {
            stdout: "0\n".to_owned(),
        },
        Verb::DisplayMessage {
            field: DisplayField::ClientTermtype,
            ..
        } => DispatchOutcome::Ok {
            stdout: "xterm-256color\n".to_owned(),
        },
        Verb::DisplayMessage {
            target: Some(pane),
            field: DisplayField::WindowId,
        } => display_window_id(herdr, idmap, &pane),
        Verb::ListPaneIds { window } => list_pane_ids(herdr, idmap, &window),
        Verb::SplitWindow {
            target,
            direction,
            size,
            ..
        } => split_window(herdr, idmap, &target, direction, size.as_deref()),
        Verb::RespawnPane { pane, command } => respawn_pane(herdr, idmap, &pane, &command),
        Verb::KillPane { pane } => kill_pane(herdr, idmap, &pane),
        Verb::SelectPaneTitle { pane, title } => select_pane_title(herdr, idmap, &pane, &title),
        Verb::ResizePane { pane, amount } => resize_pane(herdr, idmap, &pane, &amount),
        other => DispatchOutcome::Error {
            message: format!("teammux: {other:?} not yet implemented (issue #85 commit 7 pending)"),
        },
    }
}

/// `respawn-pane -k -t %N -- CMD`: launch the real teammate process in the
/// already-split pane, via `herdr pane run` (closest herdr match — herdr has
/// no separate "respawn the pane's process" primitive; `pane run` submits
/// `CMD` to the pane the same way a human typing it would).
fn respawn_pane<H: HerdrApi>(
    herdr: &H,
    idmap: &IdMap,
    pane: &TmuxId,
    command: &str,
) -> DispatchOutcome {
    let herdr_pane_id = match idmap.lookup(pane.as_str()) {
        Some(id) => id.to_owned(),
        None => return unknown_tmux_id("respawn-pane", pane.as_str()),
    };
    match herdr.pane_run(&herdr_pane_id, command) {
        Ok(()) => DispatchOutcome::Ok {
            stdout: String::new(),
        },
        Err(error) => DispatchOutcome::Error {
            message: format!("teammux: respawn-pane: herdr pane run failed: {error}"),
        },
    }
}

/// `kill-pane -t %N`: close the herdr pane and drop its idmap registration —
/// a torn-down pane must not be a resolvable tmux id afterwards.
fn kill_pane<H: HerdrApi>(herdr: &H, idmap: &IdMap, pane: &TmuxId) -> DispatchOutcome {
    let herdr_pane_id = match idmap.lookup(pane.as_str()) {
        Some(id) => id.to_owned(),
        None => return unknown_tmux_id("kill-pane", pane.as_str()),
    };
    if let Err(error) = herdr.pane_close(&herdr_pane_id) {
        return DispatchOutcome::Error {
            message: format!("teammux: kill-pane: herdr pane close failed: {error}"),
        };
    }
    match IdMap::remove(idmap.path(), pane.as_str()) {
        Ok(()) => DispatchOutcome::Ok {
            stdout: String::new(),
        },
        Err(error) => DispatchOutcome::Error {
            message: format!("teammux: kill-pane: failed to deregister pane: {error}"),
        },
    }
}

/// `select-pane -t %N -T TITLE`: rename the herdr pane.
fn select_pane_title<H: HerdrApi>(
    herdr: &H,
    idmap: &IdMap,
    pane: &TmuxId,
    title: &str,
) -> DispatchOutcome {
    let herdr_pane_id = match idmap.lookup(pane.as_str()) {
        Some(id) => id.to_owned(),
        None => return unknown_tmux_id("select-pane", pane.as_str()),
    };
    match herdr.pane_rename(&herdr_pane_id, title) {
        Ok(()) => DispatchOutcome::Ok {
            stdout: String::new(),
        },
        Err(error) => DispatchOutcome::Error {
            message: format!("teammux: select-pane: herdr pane rename failed: {error}"),
        },
    }
}

/// `resize-pane -t %N -x AMOUNT`: herdr models resize as a directional
/// border move (`--direction left|right|up|down [--amount FLOAT]`), not
/// tmux's absolute-size-target `-x`. Documented assumption (findings.md,
/// live `herdr pane --help`): `-x` (horizontal) maps onto a fixed `right`
/// direction, with the percentage converted to the same 0-1 ratio
/// `split-window` uses — there is no tmux-side direction to carry over,
/// only every observed spike call moving one lead pane. `-y` is not in the
/// verb inventory and is not parsed.
fn resize_pane<H: HerdrApi>(
    herdr: &H,
    idmap: &IdMap,
    pane: &TmuxId,
    amount: &str,
) -> DispatchOutcome {
    let herdr_pane_id = match idmap.lookup(pane.as_str()) {
        Some(id) => id.to_owned(),
        None => return unknown_tmux_id("resize-pane", pane.as_str()),
    };
    let ratio = match parse_percentage(amount) {
        Some(ratio) => ratio,
        None => {
            return DispatchOutcome::Error {
                message: format!(
                    "teammux: resize-pane: unsupported amount `{amount}` (expected a percentage like `30%`)"
                ),
            }
        }
    };
    match herdr.pane_resize(&herdr_pane_id, "right", Some(ratio)) {
        Ok(()) => DispatchOutcome::Ok {
            stdout: String::new(),
        },
        Err(error) => DispatchOutcome::Error {
            message: format!("teammux: resize-pane: herdr pane resize failed: {error}"),
        },
    }
}

/// `split-window -h/-v [-l SIZE] -P -F #{pane_id} -- cat`: split `target`
/// via `herdr pane split`, register the new pane under a freshly allocated
/// `%N` (no real tmux session to shadow — cmux comparative research
/// correction a — so allocation only has to be internally consistent with
/// this table), and print that new id. The `-- cat` placeholder is not run:
/// herdr's own default pane process serves the same "keep the pane alive
/// and idle" role tmux's `cat` placeholder does; the real teammate process
/// is launched later, by `respawn-pane -k` (commit 6).
fn split_window<H: HerdrApi>(
    herdr: &H,
    idmap: &IdMap,
    target: &TmuxId,
    direction: tmuxargs::SplitDirection,
    size: Option<&str>,
) -> DispatchOutcome {
    let herdr_target = match idmap.lookup(target.as_str()) {
        Some(id) => id.to_owned(),
        None => return unknown_tmux_id("split-window", target.as_str()),
    };
    let herdr_direction = match direction {
        tmuxargs::SplitDirection::Horizontal => "right",
        tmuxargs::SplitDirection::Vertical => "down",
    };
    let ratio = match size {
        None => None,
        Some(size) => match parse_percentage(size) {
            Some(ratio) => Some(ratio),
            None => {
                return DispatchOutcome::Error {
                    message: format!(
                        "teammux: split-window: unsupported size `{size}` (expected a percentage like `70%`)"
                    ),
                }
            }
        },
    };
    let info = match herdr.pane_split_pane(&herdr_target, herdr_direction, ratio) {
        Ok(info) => info,
        Err(error) => {
            return DispatchOutcome::Error {
                message: format!("teammux: split-window: herdr pane split failed: {error}"),
            }
        }
    };
    match IdMap::allocate(idmap.path(), '%', info.pane_id) {
        Ok(new_tmux_id) => DispatchOutcome::Ok {
            stdout: format!("{new_tmux_id}\n"),
        },
        Err(error) => DispatchOutcome::Error {
            message: format!("teammux: split-window: failed to register new pane: {error}"),
        },
    }
}

/// Parse tmux's `-l SIZE` percentage shape (e.g. `"70%"`) into a 0–1 ratio —
/// the only shape herdr's `--ratio` float accepts (confirmed against
/// `docs/herdr-api-schema.snapshot.json`'s `PaneSplitParams.ratio`, a plain
/// nullable float, not a percentage string).
fn parse_percentage(size: &str) -> Option<f64> {
    let digits = size.strip_suffix('%')?;
    let percent: f64 = digits.parse().ok()?;
    Some(percent / 100.0)
}

/// `display-message -t %N -p #{window_id}`: resolve the tmux window id that
/// owns `pane`, via herdr's own `tab_id` field on the pane.
fn display_window_id<H: HerdrApi>(herdr: &H, idmap: &IdMap, pane: &TmuxId) -> DispatchOutcome {
    let herdr_pane_id = match idmap.lookup(pane.as_str()) {
        Some(id) => id.to_owned(),
        None => return unknown_tmux_id("display-message", pane.as_str()),
    };
    let info = match herdr.pane_get(&herdr_pane_id) {
        Ok(info) => info,
        Err(error) => {
            return DispatchOutcome::Error {
                message: format!("teammux: display-message: herdr pane get failed: {error}"),
            }
        }
    };
    let Some(herdr_tab_id) = info.tab_id else {
        return DispatchOutcome::Error {
            message: format!(
                "teammux: display-message: herdr pane `{herdr_pane_id}` has no tab_id"
            ),
        };
    };
    match idmap.reverse_lookup(&herdr_tab_id) {
        Some(tmux_window_id) => DispatchOutcome::Ok {
            stdout: format!("{tmux_window_id}\n"),
        },
        None => DispatchOutcome::Error {
            message: format!(
                "teammux: display-message: herdr tab `{herdr_tab_id}` has no tmux window id registered in idmap"
            ),
        },
    }
}

/// `list-panes -t @N -F #{pane_id}`: enumerate the panes herdr reports for
/// the tab `window` maps to, translating each back to its tmux `%N` id.
fn list_pane_ids<H: HerdrApi>(herdr: &H, idmap: &IdMap, window: &TmuxId) -> DispatchOutcome {
    let herdr_tab_id = match idmap.lookup(window.as_str()) {
        Some(id) => id.to_owned(),
        None => return unknown_tmux_id("list-panes", window.as_str()),
    };
    let panes = match herdr.pane_list(None) {
        Ok(panes) => panes,
        Err(error) => {
            return DispatchOutcome::Error {
                message: format!("teammux: list-panes: herdr pane list failed: {error}"),
            }
        }
    };

    let mut tmux_ids = Vec::new();
    for pane in panes {
        if pane.tab_id.as_deref() != Some(herdr_tab_id.as_str()) {
            continue;
        }
        match idmap.reverse_lookup(&pane.pane_id) {
            Some(tmux_id) => tmux_ids.push(tmux_id.to_owned()),
            None => {
                return DispatchOutcome::Error {
                    message: format!(
                        "teammux: list-panes: herdr pane `{}` in tab `{herdr_tab_id}` has no tmux id registered in idmap",
                        pane.pane_id
                    ),
                }
            }
        }
    }
    tmux_ids.sort_by_key(|id| pane_sort_key(id));

    let mut stdout = String::new();
    for id in tmux_ids {
        stdout.push_str(&id);
        stdout.push('\n');
    }
    DispatchOutcome::Ok { stdout }
}

/// Sort key for tmux `%N`/`@N` ids: numeric by `N`, falling back to the raw
/// string for anything that doesn't parse (never observed, but sorting must
/// not panic on it).
fn pane_sort_key(tmux_id: &str) -> i64 {
    tmux_id
        .trim_start_matches(['%', '@'])
        .parse()
        .unwrap_or(i64::MAX)
}

fn unknown_tmux_id(verb: &str, tmux_id: &str) -> DispatchOutcome {
    DispatchOutcome::Error {
        message: format!("teammux: {verb}: unknown tmux id `{tmux_id}` (not in idmap)"),
    }
}

/// Parse `argv` and dispatch it against real (or faked) herdr + idmap state.
pub fn execute<H: HerdrApi>(herdr: &H, idmap: &IdMap, argv: &[String]) -> DispatchOutcome {
    match tmuxargs::parse(argv) {
        Ok(call) => dispatch(herdr, idmap, call),
        Err(error) => DispatchOutcome::Error {
            message: format_parse_error(&error),
        },
    }
}

fn format_parse_error(error: &ParseError) -> String {
    format!("teammux: {error}")
}

/// The process-boundary entry point `src/bin/teammux.rs` calls: loads the
/// real idmap and herdr client from the environment, dispatches, prints the
/// outcome, and returns a faithful exit code.
pub fn run(argv: &[String]) -> ExitCode {
    let idmap = match IdMap::load_from_env() {
        Ok(idmap) => idmap,
        Err(error) => {
            eprintln!("teammux: {error}");
            return ExitCode::FAILURE;
        }
    };
    let herdr = crate::herdr::HerdrClient::from_env();
    match execute(&herdr, &idmap, argv) {
        DispatchOutcome::Ok { stdout } => {
            print!("{stdout}");
            ExitCode::SUCCESS
        }
        DispatchOutcome::Error { message } => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::herdr::test_support::FakeHerdr;
    use crate::herdr::PaneInfo;
    use crate::tmuxargs::GlobalFlags;
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn temp_idmap(entries: &[(&str, &str)]) -> IdMap {
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("test clock should follow Unix epoch")
            .as_nanos();
        let dir = env::temp_dir().join(format!(
            "teammux-dispatch-tests-{}-{nanos}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp idmap test dir");
        let path: PathBuf = dir.join("state.json");
        for (tmux_id, herdr_id) in entries {
            IdMap::insert(&path, *tmux_id, *herdr_id).expect("seed idmap fixture");
        }
        IdMap::load(&path).expect("load seeded idmap fixture")
    }

    fn call(verb: Verb) -> tmuxargs::ParsedCall {
        tmuxargs::ParsedCall {
            globals: GlobalFlags::default(),
            verb,
        }
    }

    fn pane(pane_id: &str, tab_id: Option<&str>) -> PaneInfo {
        PaneInfo {
            pane_id: pane_id.to_owned(),
            workspace_id: "w1A".to_owned(),
            tab_id: tab_id.map(str::to_owned),
            agent: None,
            agent_id: None,
            agent_session: None,
            agent_status: None,
            cwd: None,
        }
    }

    #[test]
    fn show_mouse_probe_returns_static_off() {
        let idmap = temp_idmap(&[]);
        assert_eq!(
            dispatch(&FakeHerdr::default(), &idmap, call(Verb::ShowMouse)),
            DispatchOutcome::Ok {
                stdout: "off\n".to_owned()
            }
        );
    }

    #[test]
    fn show_focus_events_probe_returns_static_zero() {
        let idmap = temp_idmap(&[]);
        assert_eq!(
            dispatch(&FakeHerdr::default(), &idmap, call(Verb::ShowFocusEvents)),
            DispatchOutcome::Ok {
                stdout: "0\n".to_owned()
            }
        );
    }

    #[test]
    fn client_termtype_probe_returns_static_terminal_type() {
        let idmap = temp_idmap(&[]);
        assert_eq!(
            dispatch(
                &FakeHerdr::default(),
                &idmap,
                call(Verb::DisplayMessage {
                    target: None,
                    field: DisplayField::ClientTermtype,
                })
            ),
            DispatchOutcome::Ok {
                stdout: "xterm-256color\n".to_owned()
            }
        );
    }

    #[test]
    fn recognized_but_unhandled_verbs_are_a_labeled_placeholder_not_a_silent_success() {
        let idmap = temp_idmap(&[]);
        let outcome = dispatch(
            &FakeHerdr::default(),
            &idmap,
            call(Verb::SetWindowStyle {
                pane: TmuxId::parse("%0").unwrap(),
                style: "bg=default".to_owned(),
            }),
        );
        match outcome {
            DispatchOutcome::Error { message } => {
                assert!(message.contains("not yet implemented"));
            }
            other => panic!("expected a placeholder Error outcome, got {other:?}"),
        }
    }

    #[test]
    fn execute_surfaces_unrecognized_verbs_as_a_loud_error_not_silent_success() {
        let idmap = temp_idmap(&[]);
        let outcome = execute(
            &FakeHerdr::default(),
            &idmap,
            &["frobnicate-pane".to_owned()],
        );
        match outcome {
            DispatchOutcome::Error { message } => {
                assert!(message.starts_with("teammux: unrecognized verb"));
            }
            other => panic!("expected an Error outcome, got {other:?}"),
        }
    }

    #[test]
    fn display_message_window_id_resolves_the_owning_tmux_window() {
        let idmap = temp_idmap(&[("%1", "w1A:p6"), ("@0", "w1A:t1")]);
        let fake = FakeHerdr::default();
        *fake.pane.borrow_mut() = Some(pane("w1A:p6", Some("w1A:t1")));

        let outcome = dispatch(
            &fake,
            &idmap,
            call(Verb::DisplayMessage {
                target: Some(TmuxId::parse("%1").unwrap()),
                field: DisplayField::WindowId,
            }),
        );
        assert_eq!(
            outcome,
            DispatchOutcome::Ok {
                stdout: "@0\n".to_owned()
            }
        );
    }

    #[test]
    fn display_message_window_id_fails_loudly_for_an_unregistered_pane() {
        let idmap = temp_idmap(&[]);
        let outcome = dispatch(
            &FakeHerdr::default(),
            &idmap,
            call(Verb::DisplayMessage {
                target: Some(TmuxId::parse("%9").unwrap()),
                field: DisplayField::WindowId,
            }),
        );
        match outcome {
            DispatchOutcome::Error { message } => assert!(message.contains("unknown tmux id")),
            other => panic!("expected an Error outcome, got {other:?}"),
        }
    }

    #[test]
    fn display_message_window_id_fails_loudly_when_the_tab_has_no_idmap_entry() {
        let idmap = temp_idmap(&[("%1", "w1A:p6")]);
        let fake = FakeHerdr::default();
        *fake.pane.borrow_mut() = Some(pane("w1A:p6", Some("w1A:t1")));

        let outcome = dispatch(
            &fake,
            &idmap,
            call(Verb::DisplayMessage {
                target: Some(TmuxId::parse("%1").unwrap()),
                field: DisplayField::WindowId,
            }),
        );
        match outcome {
            DispatchOutcome::Error { message } => {
                assert!(message.contains("no tmux window id registered"));
            }
            other => panic!("expected an Error outcome, got {other:?}"),
        }
    }

    #[test]
    fn list_pane_ids_emits_tmux_shaped_ids_sorted_numerically() {
        let idmap = temp_idmap(&[("@0", "w1A:t1"), ("%2", "w1A:p8"), ("%1", "w1A:p6")]);
        let fake = FakeHerdr::default();
        *fake.panes.borrow_mut() = vec![
            pane("w1A:p8", Some("w1A:t1")),
            pane("w1A:p6", Some("w1A:t1")),
            pane("w1A:pOther", Some("w1A:t2")),
        ];

        let outcome = dispatch(
            &fake,
            &idmap,
            call(Verb::ListPaneIds {
                window: TmuxId::parse("@0").unwrap(),
            }),
        );
        assert_eq!(
            outcome,
            DispatchOutcome::Ok {
                stdout: "%1\n%2\n".to_owned()
            }
        );
    }

    #[test]
    fn list_pane_ids_fails_loudly_for_an_unregistered_window() {
        let idmap = temp_idmap(&[]);
        let outcome = dispatch(
            &FakeHerdr::default(),
            &idmap,
            call(Verb::ListPaneIds {
                window: TmuxId::parse("@9").unwrap(),
            }),
        );
        match outcome {
            DispatchOutcome::Error { message } => assert!(message.contains("unknown tmux id")),
            other => panic!("expected an Error outcome, got {other:?}"),
        }
    }

    #[test]
    fn list_pane_ids_fails_loudly_on_an_orphan_pane_missing_from_idmap() {
        let idmap = temp_idmap(&[("@0", "w1A:t1")]);
        let fake = FakeHerdr::default();
        *fake.panes.borrow_mut() = vec![pane("w1A:pOrphan", Some("w1A:t1"))];

        let outcome = dispatch(
            &fake,
            &idmap,
            call(Verb::ListPaneIds {
                window: TmuxId::parse("@0").unwrap(),
            }),
        );
        match outcome {
            DispatchOutcome::Error { message } => {
                assert!(message.contains("no tmux id registered"));
            }
            other => panic!("expected an Error outcome, got {other:?}"),
        }
    }

    #[test]
    fn list_pane_ids_returns_empty_output_when_the_tab_has_no_matching_panes() {
        let idmap = temp_idmap(&[("@0", "w1A:t1")]);
        let fake = FakeHerdr::default();
        *fake.panes.borrow_mut() = vec![pane("w1A:pOther", Some("w1A:t2"))];

        let outcome = dispatch(
            &fake,
            &idmap,
            call(Verb::ListPaneIds {
                window: TmuxId::parse("@0").unwrap(),
            }),
        );
        assert_eq!(
            outcome,
            DispatchOutcome::Ok {
                stdout: String::new()
            }
        );
    }

    #[test]
    fn split_window_horizontal_with_ratio_registers_and_prints_a_new_pane_id() {
        let idmap = temp_idmap(&[("%0", "w1A:p1")]);
        let fake = FakeHerdr::default();
        *fake.split_result.borrow_mut() = Some(pane("w1A:p2", Some("w1A:t1")));

        let outcome = dispatch(
            &fake,
            &idmap,
            call(Verb::SplitWindow {
                target: TmuxId::parse("%0").unwrap(),
                direction: tmuxargs::SplitDirection::Horizontal,
                size: Some("70%".to_owned()),
                command: vec!["cat".to_owned()],
            }),
        );
        assert_eq!(
            outcome,
            DispatchOutcome::Ok {
                stdout: "%1\n".to_owned()
            }
        );
        assert!(fake
            .calls()
            .iter()
            .any(|call| call == "pane_split_pane:w1A:p1:right:Some(0.7)"));

        // The new mapping is persisted, not just returned.
        let reloaded = IdMap::load(idmap.path()).unwrap();
        assert_eq!(reloaded.lookup("%1"), Some("w1A:p2"));
    }

    #[test]
    fn split_window_vertical_without_size_omits_ratio() {
        let idmap = temp_idmap(&[("%0", "w1A:p1")]);
        let fake = FakeHerdr::default();
        *fake.split_result.borrow_mut() = Some(pane("w1A:p3", Some("w1A:t1")));

        let outcome = dispatch(
            &fake,
            &idmap,
            call(Verb::SplitWindow {
                target: TmuxId::parse("%0").unwrap(),
                direction: tmuxargs::SplitDirection::Vertical,
                size: None,
                command: vec!["cat".to_owned()],
            }),
        );
        assert_eq!(
            outcome,
            DispatchOutcome::Ok {
                stdout: "%1\n".to_owned()
            }
        );
        assert!(fake
            .calls()
            .iter()
            .any(|call| call == "pane_split_pane:w1A:p1:down:None"));
    }

    #[test]
    fn split_window_allocates_past_the_highest_existing_pane_number() {
        let idmap = temp_idmap(&[("%0", "w1A:p1"), ("%1", "w1A:p2"), ("@0", "w1A:t1")]);
        let fake = FakeHerdr::default();
        *fake.split_result.borrow_mut() = Some(pane("w1A:p9", Some("w1A:t1")));

        let outcome = dispatch(
            &fake,
            &idmap,
            call(Verb::SplitWindow {
                target: TmuxId::parse("%1").unwrap(),
                direction: tmuxargs::SplitDirection::Vertical,
                size: None,
                command: vec!["cat".to_owned()],
            }),
        );
        assert_eq!(
            outcome,
            DispatchOutcome::Ok {
                stdout: "%2\n".to_owned()
            }
        );
    }

    #[test]
    fn split_window_fails_loudly_for_an_unregistered_target() {
        let idmap = temp_idmap(&[]);
        let outcome = dispatch(
            &FakeHerdr::default(),
            &idmap,
            call(Verb::SplitWindow {
                target: TmuxId::parse("%9").unwrap(),
                direction: tmuxargs::SplitDirection::Horizontal,
                size: None,
                command: vec!["cat".to_owned()],
            }),
        );
        match outcome {
            DispatchOutcome::Error { message } => assert!(message.contains("unknown tmux id")),
            other => panic!("expected an Error outcome, got {other:?}"),
        }
    }

    #[test]
    fn split_window_fails_loudly_on_an_unsupported_size_shape() {
        let idmap = temp_idmap(&[("%0", "w1A:p1")]);
        let outcome = dispatch(
            &FakeHerdr::default(),
            &idmap,
            call(Verb::SplitWindow {
                target: TmuxId::parse("%0").unwrap(),
                direction: tmuxargs::SplitDirection::Horizontal,
                size: Some("40cells".to_owned()),
                command: vec!["cat".to_owned()],
            }),
        );
        match outcome {
            DispatchOutcome::Error { message } => assert!(message.contains("unsupported size")),
            other => panic!("expected an Error outcome, got {other:?}"),
        }
    }

    #[test]
    fn split_window_surfaces_a_herdr_failure_loudly() {
        let idmap = temp_idmap(&[("%0", "w1A:p1")]);
        let fake = FakeHerdr::default();
        fake.fail_split.set(true);

        let outcome = dispatch(
            &fake,
            &idmap,
            call(Verb::SplitWindow {
                target: TmuxId::parse("%0").unwrap(),
                direction: tmuxargs::SplitDirection::Horizontal,
                size: None,
                command: vec!["cat".to_owned()],
            }),
        );
        match outcome {
            DispatchOutcome::Error { message } => {
                assert!(message.contains("herdr pane split failed"))
            }
            other => panic!("expected an Error outcome, got {other:?}"),
        }
    }

    #[test]
    fn respawn_pane_submits_the_command_via_pane_run() {
        let idmap = temp_idmap(&[("%1", "w1A:p6")]);
        let fake = FakeHerdr::default();

        let outcome = dispatch(
            &fake,
            &idmap,
            call(Verb::RespawnPane {
                pane: TmuxId::parse("%1").unwrap(),
                command: "cd /tmp && claude".to_owned(),
            }),
        );
        assert_eq!(
            outcome,
            DispatchOutcome::Ok {
                stdout: String::new()
            }
        );
        assert!(fake
            .calls()
            .iter()
            .any(|call| call == "pane_run:w1A:p6:cd /tmp && claude"));
    }

    #[test]
    fn respawn_pane_fails_loudly_for_an_unregistered_pane() {
        let idmap = temp_idmap(&[]);
        let outcome = dispatch(
            &FakeHerdr::default(),
            &idmap,
            call(Verb::RespawnPane {
                pane: TmuxId::parse("%9").unwrap(),
                command: "claude".to_owned(),
            }),
        );
        match outcome {
            DispatchOutcome::Error { message } => assert!(message.contains("unknown tmux id")),
            other => panic!("expected an Error outcome, got {other:?}"),
        }
    }

    #[test]
    fn kill_pane_closes_the_herdr_pane_and_deregisters_it() {
        let idmap = temp_idmap(&[("%1", "w1A:p6")]);
        let fake = FakeHerdr::default();

        let outcome = dispatch(
            &fake,
            &idmap,
            call(Verb::KillPane {
                pane: TmuxId::parse("%1").unwrap(),
            }),
        );
        assert_eq!(
            outcome,
            DispatchOutcome::Ok {
                stdout: String::new()
            }
        );
        assert!(fake.calls().iter().any(|call| call == "pane_close:w1A:p6"));

        let reloaded = IdMap::load(idmap.path()).unwrap();
        assert_eq!(reloaded.lookup("%1"), None);
    }

    #[test]
    fn kill_pane_fails_loudly_for_an_unregistered_pane() {
        let idmap = temp_idmap(&[]);
        let outcome = dispatch(
            &FakeHerdr::default(),
            &idmap,
            call(Verb::KillPane {
                pane: TmuxId::parse("%9").unwrap(),
            }),
        );
        match outcome {
            DispatchOutcome::Error { message } => assert!(message.contains("unknown tmux id")),
            other => panic!("expected an Error outcome, got {other:?}"),
        }
    }

    #[test]
    fn kill_pane_surfaces_a_herdr_close_failure_loudly_and_keeps_the_idmap_entry() {
        let idmap = temp_idmap(&[("%1", "w1A:p6")]);
        let fake = FakeHerdr::default();
        fake.fail_close.set(true);

        let outcome = dispatch(
            &fake,
            &idmap,
            call(Verb::KillPane {
                pane: TmuxId::parse("%1").unwrap(),
            }),
        );
        match outcome {
            DispatchOutcome::Error { message } => {
                assert!(message.contains("herdr pane close failed"))
            }
            other => panic!("expected an Error outcome, got {other:?}"),
        }
        let reloaded = IdMap::load(idmap.path()).unwrap();
        assert_eq!(reloaded.lookup("%1"), Some("w1A:p6"));
    }

    #[test]
    fn select_pane_title_renames_the_herdr_pane() {
        let idmap = temp_idmap(&[("%1", "w1A:p6")]);
        let fake = FakeHerdr::default();

        let outcome = dispatch(
            &fake,
            &idmap,
            call(Verb::SelectPaneTitle {
                pane: TmuxId::parse("%1").unwrap(),
                title: "alpha".to_owned(),
            }),
        );
        assert_eq!(
            outcome,
            DispatchOutcome::Ok {
                stdout: String::new()
            }
        );
        assert!(fake
            .calls()
            .iter()
            .any(|call| call == "pane_rename:w1A:p6:alpha"));
    }

    #[test]
    fn select_pane_title_fails_loudly_for_an_unregistered_pane() {
        let idmap = temp_idmap(&[]);
        let outcome = dispatch(
            &FakeHerdr::default(),
            &idmap,
            call(Verb::SelectPaneTitle {
                pane: TmuxId::parse("%9").unwrap(),
                title: "alpha".to_owned(),
            }),
        );
        match outcome {
            DispatchOutcome::Error { message } => assert!(message.contains("unknown tmux id")),
            other => panic!("expected an Error outcome, got {other:?}"),
        }
    }

    #[test]
    fn resize_pane_converts_percentage_and_maps_x_to_a_fixed_direction() {
        let idmap = temp_idmap(&[("%0", "w1A:p1")]);
        let fake = FakeHerdr::default();

        let outcome = dispatch(
            &fake,
            &idmap,
            call(Verb::ResizePane {
                pane: TmuxId::parse("%0").unwrap(),
                amount: "30%".to_owned(),
            }),
        );
        assert_eq!(
            outcome,
            DispatchOutcome::Ok {
                stdout: String::new()
            }
        );
        assert!(fake
            .calls()
            .iter()
            .any(|call| call == "pane_resize:w1A:p1:right:Some(0.3)"));
    }

    #[test]
    fn resize_pane_fails_loudly_for_an_unregistered_pane() {
        let idmap = temp_idmap(&[]);
        let outcome = dispatch(
            &FakeHerdr::default(),
            &idmap,
            call(Verb::ResizePane {
                pane: TmuxId::parse("%9").unwrap(),
                amount: "30%".to_owned(),
            }),
        );
        match outcome {
            DispatchOutcome::Error { message } => assert!(message.contains("unknown tmux id")),
            other => panic!("expected an Error outcome, got {other:?}"),
        }
    }

    #[test]
    fn resize_pane_fails_loudly_on_an_unsupported_amount_shape() {
        let idmap = temp_idmap(&[("%0", "w1A:p1")]);
        let outcome = dispatch(
            &FakeHerdr::default(),
            &idmap,
            call(Verb::ResizePane {
                pane: TmuxId::parse("%0").unwrap(),
                amount: "40cells".to_owned(),
            }),
        );
        match outcome {
            DispatchOutcome::Error { message } => assert!(message.contains("unsupported amount")),
            other => panic!("expected an Error outcome, got {other:?}"),
        }
    }

    // `run()` itself (env var + real HerdrClient wiring) is intentionally not
    // unit-tested here: TEAMMUX_STATE_PATH is process-global state, and
    // `cargo test` runs tests in parallel threads within one process —
    // setting/unsetting it from a test would race against
    // `idmap::tests::load_from_env_reports_a_clear_error_when_unset`, which
    // legitimately unsets it. `run()`'s glue is exercised by building and
    // invoking the real binary by hand (see PROGRESS.md) instead.
}
