//! First inbox WRITE in this crate (issue #99 stage 4, ADR-0013 §93,
//! #92 resolution's "suggested nudge" affordance). Discipline per
//! `docs/research/hook-companion-surface-2026-07-16.md` §5 (decompiled
//! `w6r`): acquire a sidecar `{inboxPath}.lock` (create-exclusive, short
//! retry, abort on contention — never write unlocked), read-modify-write
//! back via an atomic rename (never in-place truncate/append), and match
//! Claude Code's live-verified entry schema exactly — malformed entries
//! are silently pruned on next read, so a schema mistake here is a lost
//! message, not a visible error.
//!
//! This module never invents an inbox file: a missing `inboxes/{agent}.json`
//! (ENOENT) means the teammate has no inbox (e.g. in-process backend, per
//! the hook-companion doc's live-file finding) and is surfaced as
//! [`InboxWriteError::NoInbox`] — the caller's job to show honestly in
//! the TUI, never silently created.

use crate::gather::{GatherPaths, TaskDisplay};
use serde::Serialize;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum InboxWriteError {
    #[error("no inbox for {agent}: {path} does not exist")]
    NoInbox { agent: String, path: PathBuf },
    #[error("could not acquire lock {0}: still held after retries")]
    LockContention(PathBuf),
    #[error("inbox I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("existing inbox at {path} is not a JSON array: {source}")]
    Malformed {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

/// Live-verified entry schema (#89 evidence, carried into #98's findings):
/// `{from, text, timestamp, msgV, msg_id, type, read}`. Field names and
/// casing are load-bearing — Claude Code silently strips anything that
/// doesn't match this exact shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InboxEntry {
    pub from: String,
    pub text: String,
    pub timestamp: String,
    #[serde(rename = "msgV")]
    pub msg_v: u32,
    pub msg_id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub read: bool,
}

/// Pre-composed nudge text. Names the selected agent's owned in-progress
/// task when there is one; otherwise a generic status ask. Pure — the
/// caller decides which task (if any) is "the" selected agent's, this
/// only turns that choice into copy.
pub fn compose_nudge(task: Option<&TaskDisplay>) -> String {
    match task.and_then(|task| task.subject.as_deref()) {
        Some(subject) => format!("Status check on \"{subject}\" — how's it going?"),
        None => "Status check — how's it going?".to_owned(),
    }
}

/// Build one entry ready to append. `msg_id` is injected rather than
/// generated here so this stays pure and testable on a fixed id.
pub fn new_entry(from: &str, text: &str, now: SystemTime, msg_id: String) -> InboxEntry {
    InboxEntry {
        from: from.to_owned(),
        text: text.to_owned(),
        timestamp: format_iso8601_utc(now),
        msg_v: 1,
        msg_id,
        kind: "message".to_owned(),
        read: false,
    }
}

/// A v4-shaped UUID string (RFC 4122 version/variant nibbles set) built
/// from two independently-seeded `RandomState` hasher draws plus
/// wall-clock and pid. Not cryptographic — `msg_id` is a dedup key, not a
/// security token — but avoids adding a `uuid` dependency for one string
/// (ponytail rung 3: stdlib does it).
pub fn generate_msg_id() -> String {
    let seed = |salt: u64| -> u64 {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(salt);
        hasher.write_u128(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
        );
        hasher.write_u32(std::process::id());
        hasher.finish()
    };
    let hi = seed(0x9E37_79B9);
    let lo = seed(0x85EB_CA6B);

    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&hi.to_be_bytes());
    bytes[8..].copy_from_slice(&lo.to_be_bytes());
    bytes[6] = (bytes[6] & 0x0F) | 0x40; // version 4
    bytes[8] = (bytes[8] & 0x3F) | 0x80; // variant 10xx

    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5],
        bytes[6], bytes[7],
        bytes[8], bytes[9],
        bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    )
}

/// Inverse of `gather.rs`'s `parse_iso8601_utc`/`days_from_civil` (Howard
/// Hinnant's civil-calendar algorithm) — same family, no `chrono`.
fn format_iso8601_utc(now: SystemTime) -> String {
    let epoch_secs = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let days = (epoch_secs / 86_400) as i64;
    let secs_of_day = epoch_secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = secs_of_day / 3_600;
    let minute = (secs_of_day % 3_600) / 60;
    let second = secs_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

// ─── I/O: lock + read-modify-atomic-rename ──────────────────────────────────

const LOCK_RETRY_ATTEMPTS: u32 = 20;
const LOCK_RETRY_DELAY: Duration = Duration::from_millis(50);

struct LockGuard(PathBuf);

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn acquire_lock(lock_path: &Path) -> Result<LockGuard, InboxWriteError> {
    for attempt in 0..LOCK_RETRY_ATTEMPTS {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(lock_path)
        {
            Ok(_) => return Ok(LockGuard(lock_path.to_owned())),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                if attempt + 1 == LOCK_RETRY_ATTEMPTS {
                    return Err(InboxWriteError::LockContention(lock_path.to_owned()));
                }
                std::thread::sleep(LOCK_RETRY_DELAY);
            }
            Err(source) => {
                return Err(InboxWriteError::Io {
                    path: lock_path.to_owned(),
                    source,
                })
            }
        }
    }
    Err(InboxWriteError::LockContention(lock_path.to_owned()))
}

/// Append `entry` to `{team}/inboxes/{agent}.json` under lock, via
/// read-modify-atomic-rename. Existing entries are round-tripped as raw
/// `serde_json::Value` so unknown/future fields on entries this module
/// didn't write are preserved verbatim (never re-derived, never dropped).
pub fn append_entry(
    paths: &GatherPaths,
    team: &str,
    agent: &str,
    entry: &InboxEntry,
) -> Result<(), InboxWriteError> {
    let inbox_path = paths
        .teams_root
        .join(team)
        .join("inboxes")
        .join(format!("{agent}.json"));
    if !inbox_path.is_file() {
        return Err(InboxWriteError::NoInbox {
            agent: agent.to_owned(),
            path: inbox_path,
        });
    }

    let lock_path = PathBuf::from(format!("{}.lock", inbox_path.display()));
    let _guard = acquire_lock(&lock_path)?;

    let content = std::fs::read_to_string(&inbox_path).map_err(|source| InboxWriteError::Io {
        path: inbox_path.clone(),
        source,
    })?;
    let mut entries: Vec<serde_json::Value> =
        serde_json::from_str(&content).map_err(|source| InboxWriteError::Malformed {
            path: inbox_path.clone(),
            source,
        })?;
    entries.push(serde_json::to_value(entry).expect("InboxEntry always serializes"));
    let serialized = serde_json::to_string_pretty(&entries).expect("Vec<Value> always serializes");

    let tmp_path = PathBuf::from(format!("{}.tmp", inbox_path.display()));
    std::fs::write(&tmp_path, serialized).map_err(|source| InboxWriteError::Io {
        path: tmp_path.clone(),
        source,
    })?;
    std::fs::rename(&tmp_path, &inbox_path).map_err(|source| InboxWriteError::Io {
        path: inbox_path.clone(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gather::GatherPaths;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let n = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("inbox-write-test-{}-{n}", std::process::id()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn gather_paths_for(dir: &Path) -> GatherPaths {
        GatherPaths {
            teams_root: dir.join("teams"),
            tasks_root: dir.join("tasks"),
            projects_root: dir.join("projects"),
        }
    }

    fn task(subject: Option<&str>) -> TaskDisplay {
        TaskDisplay {
            id: "1".to_owned(),
            subject: subject.map(str::to_owned),
            status: "in_progress".to_owned(),
            owner: Some("alpha".to_owned()),
            seconds_since_modified: Some(5),
        }
    }

    // ── compose_nudge ────────────────────────────────────────────────────────

    #[test]
    fn compose_nudge_names_the_owned_task_subject() {
        let text = compose_nudge(Some(&task(Some("Ship the board"))));
        assert_eq!(text, "Status check on \"Ship the board\" — how's it going?");
    }

    #[test]
    fn compose_nudge_falls_back_generically_with_no_task() {
        assert_eq!(compose_nudge(None), "Status check — how's it going?");
    }

    #[test]
    fn compose_nudge_falls_back_generically_when_task_has_no_subject() {
        assert_eq!(
            compose_nudge(Some(&task(None))),
            "Status check — how's it going?"
        );
    }

    // ── entry schema ─────────────────────────────────────────────────────────

    #[test]
    fn new_entry_matches_the_exact_live_verified_schema() {
        let now = UNIX_EPOCH + Duration::from_secs(1_752_768_000);
        let entry = new_entry("team-lead", "go", now, "fixed-id".to_owned());
        let json = serde_json::to_string(&entry).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["from"], "team-lead");
        assert_eq!(value["text"], "go");
        assert_eq!(value["msgV"], 1);
        assert_eq!(value["msg_id"], "fixed-id");
        assert_eq!(value["type"], "message");
        assert_eq!(value["read"], false);
        assert!(value.get("timestamp").is_some());
        // exact key set — no extra, no missing (a stray field is a lost
        // message, per the module doc's silent-prune warning)
        let mut keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "from",
                "msgV",
                "msg_id",
                "read",
                "text",
                "timestamp",
                "type"
            ]
        );
    }

    #[test]
    fn generate_msg_id_looks_like_a_v4_uuid_and_is_not_constant() {
        let a = generate_msg_id();
        let b = generate_msg_id();
        assert_ne!(a, b);
        let parts: Vec<&str> = a.split('-').collect();
        assert_eq!(
            parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
            [8, 4, 4, 4, 12]
        );
        assert_eq!(parts[2].chars().next().unwrap(), '4');
        assert!(matches!(
            parts[3].chars().next().unwrap(),
            '8' | '9' | 'a' | 'b'
        ));
    }

    #[test]
    fn format_iso8601_utc_round_trips_through_the_existing_parser() {
        let now = UNIX_EPOCH + Duration::from_secs(1_752_768_045);
        let entry = new_entry("x", "y", now, "id".to_owned());
        assert_eq!(
            crate::gather::parse_iso8601_utc(&entry.timestamp),
            Some(1_752_768_045)
        );
    }

    // ── append_entry: NoInbox honesty ────────────────────────────────────────

    #[test]
    fn append_entry_errors_honestly_when_no_inbox_file_exists() {
        let dir = TempDir::new();
        let paths = gather_paths_for(&dir.0);
        std::fs::create_dir_all(paths.teams_root.join("team-x").join("inboxes")).unwrap();
        let entry = new_entry("team-lead", "go", SystemTime::now(), "id".to_owned());

        let result = append_entry(&paths, "team-x", "alpha", &entry);
        assert!(matches!(result, Err(InboxWriteError::NoInbox { .. })));
    }

    #[test]
    fn append_entry_never_creates_the_inbox_file() {
        let dir = TempDir::new();
        let paths = gather_paths_for(&dir.0);
        std::fs::create_dir_all(paths.teams_root.join("team-x").join("inboxes")).unwrap();
        let entry = new_entry("team-lead", "go", SystemTime::now(), "id".to_owned());

        let _ = append_entry(&paths, "team-x", "alpha", &entry);
        assert!(!paths
            .teams_root
            .join("team-x")
            .join("inboxes")
            .join("alpha.json")
            .exists());
    }

    // ── append_entry: lock + atomic rename, integration ──────────────────────

    #[test]
    fn append_entry_appends_via_lock_and_atomic_rename() {
        let dir = TempDir::new();
        let paths = gather_paths_for(&dir.0);
        let inbox_dir = paths.teams_root.join("team-x").join("inboxes");
        std::fs::create_dir_all(&inbox_dir).unwrap();
        let inbox_path = inbox_dir.join("alpha.json");
        std::fs::write(
            &inbox_path,
            r#"[{"from":"someone","text":"hi","timestamp":"2026-07-17T00:00:00Z","msgV":1,"msg_id":"old","type":"message","read":true}]"#,
        )
        .unwrap();

        let entry = new_entry(
            "team-lead",
            "status?",
            SystemTime::now(),
            "new-id".to_owned(),
        );
        append_entry(&paths, "team-x", "alpha", &entry).unwrap();

        let content = std::fs::read_to_string(&inbox_path).unwrap();
        let entries: Vec<serde_json::Value> = serde_json::from_str(&content).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["msg_id"], "old");
        assert_eq!(entries[1]["msg_id"], "new-id");
        assert_eq!(entries[1]["text"], "status?");

        // no leftover lock or tmp file
        assert!(!PathBuf::from(format!("{}.lock", inbox_path.display())).exists());
        assert!(!PathBuf::from(format!("{}.tmp", inbox_path.display())).exists());
    }

    #[test]
    fn append_entry_rejects_a_malformed_existing_file_without_touching_it() {
        let dir = TempDir::new();
        let paths = gather_paths_for(&dir.0);
        let inbox_dir = paths.teams_root.join("team-x").join("inboxes");
        std::fs::create_dir_all(&inbox_dir).unwrap();
        let inbox_path = inbox_dir.join("alpha.json");
        std::fs::write(&inbox_path, "not json").unwrap();

        let entry = new_entry("team-lead", "go", SystemTime::now(), "id".to_owned());
        let result = append_entry(&paths, "team-x", "alpha", &entry);
        assert!(matches!(result, Err(InboxWriteError::Malformed { .. })));
        assert_eq!(std::fs::read_to_string(&inbox_path).unwrap(), "not json");
    }

    #[test]
    fn acquire_lock_times_out_when_the_lock_file_is_already_held() {
        let dir = TempDir::new();
        let lock_path = dir.0.join("held.lock");
        std::fs::write(&lock_path, "").unwrap();

        // shrink the retry budget indirectly by pre-holding the lock —
        // acquire_lock will exhaust LOCK_RETRY_ATTEMPTS and error rather
        // than hang; this test intentionally accepts the ~1s retry cost.
        let result = acquire_lock(&lock_path);
        assert!(matches!(result, Err(InboxWriteError::LockContention(_))));
        std::fs::remove_file(&lock_path).unwrap();
    }
}
