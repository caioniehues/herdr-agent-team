//! JSONL audit log of consumed attention items (D3, issue #86 commit 4).
//!
//! Append-only: one JSON object per line, each recording "this attention
//! item id was consumed" (jumped to via `herdmates jump`, commit 5; marked
//! done in the focus-pane TUI, commit 7). The `attention` module (commit 3)
//! has no read/unread state of its own — every call to
//! `attention::build_attention_queue` returns *every* currently-true
//! candidate, unconditionally. This log is the missing piece: subtracting
//! its ids from a freshly-built queue is what turns "all candidates, every
//! time" into an actual unread/pending view (`filter_unconsumed`).
//!
//! Lives under the plugin's own state dir (`paths::state_dir()`), not the
//! fixed `~/.local/share/herdmates/focus.md` path — this is internal
//! plugin bookkeeping, not a human-editable contract file, so it doesn't
//! need to be addressable outside the plugin (contrast `docs/focus-file.md`'s
//! ownership rule).

use crate::attention::AttentionItem;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::io::Write as _;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuditLogError {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AuditRecord {
    id: String,
    consumed_at_unix_ms: u64,
}

/// Append one "consumed" record. Creates the parent directory and the file
/// itself if either is missing. `consumed_at_unix_ms` is supplied by the
/// caller (not read from the wall clock in here) so this stays testable
/// without a real-time dependency, matching `pump::maybe_pump_at`'s pattern.
pub fn append_consumed(
    path: &Path,
    id: &str,
    consumed_at_unix_ms: u64,
) -> Result<(), AuditLogError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| AuditLogError::Write {
            path: path.display().to_string(),
            source,
        })?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|source| AuditLogError::Write {
            path: path.display().to_string(),
            source,
        })?;
    file.write_all(render_audit_line(id, consumed_at_unix_ms).as_bytes())
        .map_err(|source| AuditLogError::Write {
            path: path.display().to_string(),
            source,
        })
}

/// Read every consumed id ever recorded. A missing file is not an error —
/// it means nothing has been consumed yet — and resolves to an empty set,
/// matching `read_focus_file`'s missing-file-is-default precedent.
pub fn read_consumed_ids(path: &Path) -> Result<BTreeSet<String>, AuditLogError> {
    match std::fs::read_to_string(path) {
        Ok(contents) => Ok(parse_audit_lines(&contents)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeSet::new()),
        Err(source) => Err(AuditLogError::Read {
            path: path.display().to_string(),
            source,
        }),
    }
}

/// Remove already-consumed items from a freshly built attention queue,
/// preserving the queue's existing order.
pub fn filter_unconsumed(
    items: Vec<AttentionItem>,
    consumed: &BTreeSet<String>,
) -> Vec<AttentionItem> {
    items
        .into_iter()
        .filter(|item| !consumed.contains(&item.id))
        .collect()
}

fn render_audit_line(id: &str, consumed_at_unix_ms: u64) -> String {
    let record = AuditRecord {
        id: id.to_owned(),
        consumed_at_unix_ms,
    };
    // A `String`/`u64` pair can't fail to serialize; the log format is our
    // own struct, not user input.
    format!(
        "{}\n",
        serde_json::to_string(&record).expect("AuditRecord always serializes")
    )
}

/// Pure parser: one JSON object per line. Malformed or blank lines are
/// silently skipped, never fatal — an audit log a human or another tool
/// hand-edited (or a partially-written line from a crash mid-append)
/// shouldn't take down the whole read, matching the degrade policy already
/// established in `teamfiles`/`focusfile`/`pump`.
fn parse_audit_lines(contents: &str) -> BTreeSet<String> {
    contents
        .lines()
        .filter_map(|line| serde_json::from_str::<AuditRecord>(line.trim()).ok())
        .map(|record| record.id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attention::AttentionKind;

    fn item(id: &str) -> AttentionItem {
        AttentionItem {
            id: id.to_owned(),
            kind: AttentionKind::Decision,
            summary: id.to_owned(),
            pane_id: None,
        }
    }

    fn temp_path(label: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("test clock after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "herdmates-audit-tests-{label}-{}-{nanos}",
            std::process::id()
        ))
    }

    #[test]
    fn missing_file_resolves_to_empty_set_not_an_error() {
        let ids = read_consumed_ids(Path::new("/nonexistent/audit.jsonl")).expect("not an error");
        assert!(ids.is_empty());
    }

    #[test]
    fn append_then_read_round_trips_through_disk() {
        let dir = temp_path("roundtrip");
        let path = dir.join("audit.jsonl");

        append_consumed(&path, "decision:abc", 1_000).expect("append into new dir");
        append_consumed(&path, "inbox:def", 2_000).expect("append second line");

        let ids = read_consumed_ids(&path).expect("read back");
        assert_eq!(
            ids,
            BTreeSet::from(["decision:abc".to_owned(), "inbox:def".to_owned()])
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn appending_the_same_id_twice_still_yields_one_entry_in_the_set() {
        let dir = temp_path("dup");
        let path = dir.join("audit.jsonl");

        append_consumed(&path, "decision:abc", 1_000).expect("first append");
        append_consumed(&path, "decision:abc", 2_000).expect("second append");

        let ids = read_consumed_ids(&path).expect("read back");
        assert_eq!(
            ids.len(),
            1,
            "the log is append-only; dedup happens on read"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_and_blank_lines_are_skipped_not_fatal() {
        let ids = parse_audit_lines(
            "{\"id\":\"decision:abc\",\"consumed_at_unix_ms\":1}\n\nnot json at all\n   \n",
        );
        assert_eq!(ids, BTreeSet::from(["decision:abc".to_owned()]));
    }

    #[test]
    fn filter_unconsumed_removes_only_matching_ids_and_preserves_order() {
        let items = vec![item("a"), item("b"), item("c")];
        let consumed = BTreeSet::from(["b".to_owned()]);

        let remaining = filter_unconsumed(items, &consumed);

        assert_eq!(
            remaining.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            vec!["a", "c"]
        );
    }

    #[test]
    fn filter_unconsumed_is_a_no_op_on_an_empty_consumed_set() {
        let items = vec![item("a"), item("b")];
        let remaining = filter_unconsumed(items.clone(), &BTreeSet::new());
        assert_eq!(remaining, items);
    }
}
