//! Pure-logic parsing/serialization of the focus-file contract (D3, issue
//! #86, ADR-0012 §3): `~/.local/share/herdmates/focus.md`. Format and
//! ownership rules documented in `docs/focus-file.md` (commit 2).
//!
//! Anything may write this file — human, agent, the atomizer skill — and
//! the focus pane only ever renders it, never owns it (BRIEF binding
//! decision). Parsing is therefore tolerant by construction: malformed or
//! unrecognized lines are silently skipped, never fatal, matching the
//! degrade policy already established in `teamfiles`/`pump` (#84).
//!
//! Decision-queue entry ids are not authored by hand — deriving a stable
//! id from an explicit syntax would be one more thing a human editor has
//! to get right. Instead each entry's id is a deterministic hash of its
//! trimmed text (FNV-1a, hand-rolled to avoid depending on std's
//! `DefaultHasher`, whose algorithm is explicitly unspecified and free to
//! change across Rust releases — this id must stay stable across binary
//! rebuilds so an audit log entry written today still matches on a
//! future run). Duplicate text within one file gets a `-2`, `-3`, ...
//! suffix so ids stay unique.

use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FocusFileError {
    #[error("failed to read {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// The parsed focus-file contract.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FocusFile {
    pub task: Option<String>,
    pub next_action: Option<String>,
    pub decisions: Vec<DecisionEntry>,
}

/// One entry from the `## Decisions` checkbox list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionEntry {
    /// Stable, content-derived id (see module docs) — not authored by hand.
    pub id: String,
    pub text: String,
    pub resolved: bool,
}

/// Read the focus file at `path`. A missing file is not an error — it
/// means no focus has been recorded yet — and resolves to an empty
/// [`FocusFile`], matching the launcher-table precedent (`load_launcher_table`).
pub fn read_focus_file(path: &Path) -> Result<FocusFile, FocusFileError> {
    match std::fs::read_to_string(path) {
        Ok(contents) => Ok(parse_focus_file_str(&contents)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(FocusFile::default()),
        Err(source) => Err(FocusFileError::Read {
            path: path.display().to_string(),
            source,
        }),
    }
}

pub fn write_focus_file(path: &Path, focus: &FocusFile) -> Result<(), FocusFileError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| FocusFileError::Write {
            path: path.display().to_string(),
            source,
        })?;
    }
    std::fs::write(path, render_focus_file(focus)).map_err(|source| FocusFileError::Write {
        path: path.display().to_string(),
        source,
    })
}

#[derive(PartialEq, Eq)]
enum Section {
    None,
    Task,
    NextAction,
    Decisions,
}

/// Pure parser: never errors, tolerant of any input (BRIEF: "tolerant of
/// hand-edits"). Unrecognized headings and non-checkbox lines inside
/// `## Decisions` are silently skipped.
pub fn parse_focus_file_str(contents: &str) -> FocusFile {
    let mut section = Section::None;
    let mut task_lines: Vec<&str> = Vec::new();
    let mut next_action_lines: Vec<&str> = Vec::new();
    let mut decisions = Vec::new();
    let mut seen_ids = std::collections::BTreeMap::<String, u32>::new();

    for line in contents.lines() {
        let trimmed = line.trim();
        if let Some(heading) = trimmed.strip_prefix("## ") {
            section = match heading.trim().to_ascii_lowercase().as_str() {
                "task" => Section::Task,
                "next action" => Section::NextAction,
                "decisions" => Section::Decisions,
                _ => Section::None,
            };
            continue;
        }
        if trimmed.starts_with("# ") {
            // Top-level title line (e.g. "# Focus") — not a section body.
            continue;
        }
        match section {
            Section::Task => task_lines.push(line),
            Section::NextAction => next_action_lines.push(line),
            Section::Decisions => {
                if let Some(entry) = parse_decision_line(trimmed, &mut seen_ids) {
                    decisions.push(entry);
                }
            }
            Section::None => {}
        }
    }

    FocusFile {
        task: joined_or_none(&task_lines),
        next_action: joined_or_none(&next_action_lines),
        decisions,
    }
}

fn joined_or_none(lines: &[&str]) -> Option<String> {
    let joined = lines.join("\n").trim().to_owned();
    (!joined.is_empty()).then_some(joined)
}

fn parse_decision_line(
    trimmed: &str,
    seen_ids: &mut std::collections::BTreeMap<String, u32>,
) -> Option<DecisionEntry> {
    let rest = trimmed.strip_prefix("- [")?;
    let (marker, text) = rest.split_once(']')?;
    let resolved = match marker {
        " " => false,
        "x" | "X" => true,
        _ => return None,
    };
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let id = unique_id(text, seen_ids);
    Some(DecisionEntry {
        id,
        text: text.to_owned(),
        resolved,
    })
}

fn unique_id(text: &str, seen_ids: &mut std::collections::BTreeMap<String, u32>) -> String {
    let base = format!("{:016x}", fnv1a_hash(text));
    let count = seen_ids.entry(base.clone()).or_insert(0);
    *count += 1;
    if *count == 1 {
        base
    } else {
        format!("{base}-{count}")
    }
}

/// FNV-1a, 64-bit. Deterministic by construction — see module docs for why
/// this can't be `std::collections::hash_map::DefaultHasher`.
fn fnv1a_hash(text: &str) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    text.bytes().fold(OFFSET_BASIS, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(PRIME)
    })
}

/// Canonical re-serialization. Does not preserve original formatting,
/// stray notes, or extra headings from a hand-edited source — round-trips
/// the *contract* fields only, which is the documented ownership boundary
/// (`docs/focus-file.md`: the pane renders, never owns, but any writer
/// that reads-modifies-writes goes through this canonical form).
pub fn render_focus_file(focus: &FocusFile) -> String {
    let mut out = String::from("# Focus\n\n## Task\n");
    if let Some(task) = &focus.task {
        out.push_str(task);
        out.push('\n');
    }
    out.push_str("\n## Next Action\n");
    if let Some(next_action) = &focus.next_action {
        out.push_str(next_action);
        out.push('\n');
    }
    out.push_str("\n## Decisions\n");
    for decision in &focus.decisions {
        let checkbox = if decision.resolved { "x" } else { " " };
        out.push_str(&format!("- [{checkbox}] {}\n", decision.text));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/focusfile")
            .join(name)
    }

    #[test]
    fn full_fixture_parses_task_next_action_and_decisions() {
        let focus = read_focus_file(&fixture("full.md")).expect("full fixture");

        assert_eq!(
            focus.task.as_deref(),
            Some("Ship the D3 focus pane foundation")
        );
        assert_eq!(
            focus.next_action.as_deref(),
            Some("Write the focusfile parser")
        );
        assert_eq!(focus.decisions.len(), 3);
        assert!(!focus.decisions[0].resolved);
        assert_eq!(
            focus.decisions[0].text,
            "Ship split or overlay placement first?"
        );
        assert!(focus.decisions[1].resolved);
        assert!(!focus.decisions[2].resolved);
    }

    #[test]
    fn missing_file_resolves_to_default_not_an_error() {
        let focus =
            read_focus_file(Path::new("/nonexistent/focus.md")).expect("missing file is not Err");
        assert_eq!(focus, FocusFile::default());
    }

    #[test]
    fn hand_edited_fixture_ignores_stray_notes_and_unknown_headings() {
        let focus = read_focus_file(&fixture("hand-edited.md")).expect("hand-edited fixture");

        assert!(focus.task.is_some_and(|t| t.contains("flaky CI run")));
        assert_eq!(
            focus.next_action, None,
            "empty Next Action section must be None, not Some(\"\")"
        );
        // 4 decision-section lines: 1 valid unchecked, 1 non-checkbox note
        // (skipped), 1 valid uppercase-X checked, 1 malformed (skipped).
        assert_eq!(focus.decisions.len(), 2);
        assert!(!focus.decisions[0].resolved);
        assert!(focus.decisions[1].resolved);
        assert_eq!(
            focus.decisions[1].text,
            "Uppercase X should still count as checked"
        );
    }

    #[test]
    fn empty_contents_parse_to_default_focus_file() {
        assert_eq!(parse_focus_file_str(""), FocusFile::default());
    }

    #[test]
    fn duplicate_decision_text_gets_disambiguated_ids() {
        let focus = parse_focus_file_str(
            "## Decisions\n- [ ] Same text\n- [ ] Same text\n- [ ] Same text\n",
        );

        let ids = focus
            .decisions
            .iter()
            .map(|d| d.id.clone())
            .collect::<Vec<_>>();
        assert_eq!(ids.len(), 3);
        assert_ne!(ids[0], ids[1]);
        assert_ne!(ids[1], ids[2]);
        assert!(ids[0].len() == 16, "first occurrence keeps the bare hash");
        assert!(ids[1].ends_with("-2"));
        assert!(ids[2].ends_with("-3"));
    }

    #[test]
    fn decision_id_is_stable_across_separate_parses() {
        let first = parse_focus_file_str("## Decisions\n- [ ] Ship it?\n");
        let second = parse_focus_file_str("## Task\nunrelated\n\n## Decisions\n- [ ] Ship it?\n");

        assert_eq!(first.decisions[0].id, second.decisions[0].id);
    }

    #[test]
    fn render_then_parse_round_trips_contract_fields() {
        let original = FocusFile {
            task: Some("Investigate the flaky run".to_owned()),
            next_action: Some("Bisect the commit range".to_owned()),
            decisions: vec![
                DecisionEntry {
                    id: "ignored-on-render".to_owned(),
                    text: "Retry or bisect?".to_owned(),
                    resolved: false,
                },
                DecisionEntry {
                    id: "also-ignored".to_owned(),
                    text: "File a flaky-test ticket?".to_owned(),
                    resolved: true,
                },
            ],
        };

        let rendered = render_focus_file(&original);
        let reparsed = parse_focus_file_str(&rendered);

        assert_eq!(reparsed.task, original.task);
        assert_eq!(reparsed.next_action, original.next_action);
        assert_eq!(reparsed.decisions.len(), 2);
        assert_eq!(reparsed.decisions[0].text, "Retry or bisect?");
        assert!(!reparsed.decisions[0].resolved);
        assert!(reparsed.decisions[1].resolved);
    }

    #[test]
    fn write_then_read_round_trips_through_disk() {
        let sequence = std::sync::atomic::AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("test clock after epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "herdmates-focusfile-tests-{}-{nanos}-{}",
            std::process::id(),
            sequence.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let path = dir.join("focus.md");
        let focus = FocusFile {
            task: Some("Write the module".to_owned()),
            next_action: None,
            decisions: Vec::new(),
        };

        write_focus_file(&path, &focus).expect("write into a not-yet-existing directory");
        let reread = read_focus_file(&path).expect("read back");

        assert_eq!(reread.task, focus.task);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
