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
//!
//! Consumption is TTL-scoped, not permanent (issue #86 review finding 1).
//! `blocked:{pane_id}` and `inbox:{stable_id(from|content)}` ids
//! (`attention.rs`) carry no occurrence/time signal — they're keyed on
//! *what* is blocked or *what* was said, not *when*. Recording a consumed
//! id forever would mean a worker that goes blocked, gets fixed, and later
//! goes blocked again on the same pane produces the identical id and is
//! silently swallowed by the old consumption — exactly the failure this
//! feature exists to prevent (attention silently not surfaced). Rather
//! than fold a timestamp/nonce into the id itself (which would make a
//! *single ongoing* occurrence's id change on every queue rebuild, since
//! nothing here carries a real "first observed" timestamp to anchor a
//! nonce to — that breaks dedup entirely, not just the recurrence case),
//! consumption here expires after `CONSUMED_TTL_MS`: a still-true
//! situation eventually resurfaces and re-prompts the human, which is
//! reasonable "attention" semantics on its own, and a genuinely new
//! occurrence separated by more than the TTL is never mistaken for the
//! old one.

use crate::attention::AttentionItem;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::Path;
use thiserror::Error;

/// How long a consumed id keeps suppressing its attention item. 24h: long
/// enough that jumping to or dismissing something doesn't immediately
/// re-nag within the same working session, short enough that a situation
/// still true a day later gets surfaced again rather than staying silently
/// swallowed forever.
const CONSUMED_TTL_MS: u64 = 24 * 60 * 60 * 1000;

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

/// Read every consumed id ever recorded, mapped to the *most recent*
/// `consumed_at_unix_ms` it was recorded with (the log is append-only, so a
/// re-consumption of the same id — e.g. marked done twice — refreshes when
/// its suppression window started). A missing file is not an error — it
/// means nothing has been consumed yet — and resolves to an empty map,
/// matching `read_focus_file`'s missing-file-is-default precedent.
pub fn read_consumed(path: &Path) -> Result<BTreeMap<String, u64>, AuditLogError> {
    match std::fs::read_to_string(path) {
        Ok(contents) => Ok(parse_audit_lines(&contents)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(source) => Err(AuditLogError::Read {
            path: path.display().to_string(),
            source,
        }),
    }
}

/// Remove still-suppressed items from a freshly built attention queue,
/// preserving the queue's existing order. An item is suppressed only while
/// `now_ms` is within `CONSUMED_TTL_MS` of its most recent consumption —
/// see the module docs for why this is TTL-scoped rather than permanent.
pub fn filter_unconsumed(
    items: Vec<AttentionItem>,
    consumed: &BTreeMap<String, u64>,
    now_ms: u64,
) -> Vec<AttentionItem> {
    items
        .into_iter()
        .filter(|item| match consumed.get(&item.id) {
            Some(consumed_at) => now_ms.saturating_sub(*consumed_at) >= CONSUMED_TTL_MS,
            None => true,
        })
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

/// Pure parser: one JSON object per line, later lines overwrite earlier
/// ones for the same id (append-only file, so "later in the file" means
/// "more recently consumed"). Malformed or blank lines are silently
/// skipped, never fatal — an audit log a human or another tool hand-edited
/// (or a partially-written line from a crash mid-append) shouldn't take
/// down the whole read, matching the degrade policy already established in
/// `teamfiles`/`focusfile`/`pump`.
fn parse_audit_lines(contents: &str) -> BTreeMap<String, u64> {
    let mut consumed = BTreeMap::new();
    for record in contents
        .lines()
        .filter_map(|line| serde_json::from_str::<AuditRecord>(line.trim()).ok())
    {
        consumed.insert(record.id, record.consumed_at_unix_ms);
    }
    consumed
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
    fn missing_file_resolves_to_empty_map_not_an_error() {
        let consumed = read_consumed(Path::new("/nonexistent/audit.jsonl")).expect("not an error");
        assert!(consumed.is_empty());
    }

    #[test]
    fn append_then_read_round_trips_through_disk() {
        let dir = temp_path("roundtrip");
        let path = dir.join("audit.jsonl");

        append_consumed(&path, "decision:abc", 1_000).expect("append into new dir");
        append_consumed(&path, "inbox:def", 2_000).expect("append second line");

        let consumed = read_consumed(&path).expect("read back");
        assert_eq!(
            consumed,
            BTreeMap::from([
                ("decision:abc".to_owned(), 1_000),
                ("inbox:def".to_owned(), 2_000),
            ])
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn appending_the_same_id_twice_keeps_the_most_recent_timestamp() {
        let dir = temp_path("dup");
        let path = dir.join("audit.jsonl");

        append_consumed(&path, "decision:abc", 1_000).expect("first append");
        append_consumed(&path, "decision:abc", 2_000).expect("second append");

        let consumed = read_consumed(&path).expect("read back");
        assert_eq!(
            consumed,
            BTreeMap::from([("decision:abc".to_owned(), 2_000)]),
            "the log is append-only; the later record wins on read"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_and_blank_lines_are_skipped_not_fatal() {
        let consumed = parse_audit_lines(
            "{\"id\":\"decision:abc\",\"consumed_at_unix_ms\":1}\n\nnot json at all\n   \n",
        );
        assert_eq!(consumed, BTreeMap::from([("decision:abc".to_owned(), 1)]));
    }

    #[test]
    fn filter_unconsumed_removes_only_matching_ids_and_preserves_order() {
        let items = vec![item("a"), item("b"), item("c")];
        let consumed = BTreeMap::from([("b".to_owned(), 1_000)]);

        let remaining = filter_unconsumed(items, &consumed, 1_500);

        assert_eq!(
            remaining.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            vec!["a", "c"]
        );
    }

    #[test]
    fn filter_unconsumed_is_a_no_op_on_an_empty_consumed_map() {
        let items = vec![item("a"), item("b")];
        let remaining = filter_unconsumed(items.clone(), &BTreeMap::new(), 0);
        assert_eq!(remaining, items);
    }

    /// The cross-time collision the review flagged (finding 1): a
    /// non-occurrence-unique id (e.g. `blocked:w1A:p1`, recurring on the
    /// same pane) must not be swallowed forever by one old consumption.
    /// Within the TTL it still suppresses (no regression); once the TTL
    /// has elapsed since that consumption, the id — unchanged, same as a
    /// genuinely new occurrence would produce — must resurface.
    #[test]
    fn a_stale_consumption_expires_and_lets_a_recurring_id_resurface() {
        let items = vec![item("blocked:w1A:p1")];
        let consumed = BTreeMap::from([("blocked:w1A:p1".to_owned(), 0)]);

        let still_within_ttl = filter_unconsumed(items.clone(), &consumed, CONSUMED_TTL_MS - 1);
        assert!(
            still_within_ttl.is_empty(),
            "a recent consumption must still suppress (no regression)"
        );

        let past_ttl = filter_unconsumed(items, &consumed, CONSUMED_TTL_MS);
        assert_eq!(
            past_ttl.len(),
            1,
            "a consumption older than the TTL must not permanently swallow a recurring id"
        );
    }
}
