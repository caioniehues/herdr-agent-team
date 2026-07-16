//! `%N`/`@N` (tmux pane/window) ↔ herdr id table for the teammux shim
//! (issue #85 commit 1).
//!
//! Each tmux-argv invocation the shim handles is a fresh process (issue #85
//! decision doc), so the table is entirely file-backed at the path named by
//! [`STATE_PATH_ENV`] and every mutation is a self-contained load-mutate-save
//! transaction guarded by a sibling lock file — the same pattern
//! `run::update_run_with_hook` uses for `run.toml`. A single table holds both
//! `%N` pane ids and `@N` window ids side by side: the key string carries its
//! own `%`/`@` prefix, so lookups for either kind are indistinguishable from
//! the table's perspective, matching how the spike log targets `@N` in
//! `list-panes`/`set-option -w`/`select-layout` exactly like it targets `%N`
//! in pane verbs.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::env;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use thiserror::Error;

use fs4::FileExt;

/// Env var the launcher sets to the per-session idmap state file path.
pub const STATE_PATH_ENV: &str = "TEAMMUX_STATE_PATH";

#[derive(Debug, Error)]
pub enum IdMapError {
    #[error("{STATE_PATH_ENV} is not set")]
    StatePathUnset,
    #[error("failed to read {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to write {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to lock {path}: {source}")]
    Lock {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to serialize idmap for {path}: {source}")]
    Serialize {
        path: String,
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct IdMapWire {
    #[serde(default)]
    entries: BTreeMap<String, String>,
}

/// A loaded snapshot of the `%N`/`@N` ↔ herdr-id table.
///
/// [`IdMap::load`] takes an unlocked read for [`IdMap::lookup`] snapshots.
/// [`IdMap::insert`] and [`IdMap::remove`] are associated functions that take
/// the state path directly rather than `&mut self`: each re-reads the table
/// fresh under a lock before mutating and persisting, so a snapshot held by
/// one process can never overwrite a concurrent writer's update (the lost-
/// update race a fresh-process-per-invocation model would otherwise invite).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdMap {
    entries: BTreeMap<String, String>,
}

impl IdMap {
    /// Resolve the state path from [`STATE_PATH_ENV`] and load the table.
    pub fn load_from_env() -> Result<Self, IdMapError> {
        Self::load(&state_path_from_env()?)
    }

    /// Load the table at `path`, treating a missing file as empty (first
    /// invocation of a session).
    pub fn load(path: &Path) -> Result<Self, IdMapError> {
        Ok(Self {
            entries: read_entries(path)?,
        })
    }

    /// Look up the herdr id for a tmux `%N`/`@N` id.
    pub fn lookup(&self, tmux_id: &str) -> Option<&str> {
        self.entries.get(tmux_id).map(String::as_str)
    }

    /// Insert (or overwrite) `tmux_id` → `herdr_id`, persisting the change
    /// under a fresh locked load-mutate-save transaction.
    pub fn insert(
        path: &Path,
        tmux_id: impl Into<String>,
        herdr_id: impl Into<String>,
    ) -> Result<(), IdMapError> {
        let tmux_id = tmux_id.into();
        let herdr_id = herdr_id.into();
        transact(path, move |entries| {
            entries.insert(tmux_id, herdr_id);
        })
    }

    /// Remove `tmux_id` from the table, persisting the change under a fresh
    /// locked load-mutate-save transaction. A no-op if absent.
    pub fn remove(path: &Path, tmux_id: &str) -> Result<(), IdMapError> {
        transact(path, |entries| {
            entries.remove(tmux_id);
        })
    }
}

fn state_path_from_env() -> Result<PathBuf, IdMapError> {
    env::var_os(STATE_PATH_ENV)
        .map(PathBuf::from)
        .ok_or(IdMapError::StatePathUnset)
}

fn lock_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_default();
    name.push(".lock");
    path.with_file_name(name)
}

fn read_entries(path: &Path) -> Result<BTreeMap<String, String>, IdMapError> {
    match fs::read_to_string(path) {
        Ok(raw) => {
            let wire: IdMapWire =
                serde_json::from_str(&raw).map_err(|source| IdMapError::Parse {
                    path: path.display().to_string(),
                    source,
                })?;
            Ok(wire.entries)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(source) => Err(IdMapError::Read {
            path: path.display().to_string(),
            source,
        }),
    }
}

fn write_entries(path: &Path, entries: &BTreeMap<String, String>) -> Result<(), IdMapError> {
    let wire = IdMapWire {
        entries: entries.clone(),
    };
    let json = serde_json::to_string_pretty(&wire).map_err(|source| IdMapError::Serialize {
        path: path.display().to_string(),
        source,
    })?;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|source| IdMapError::Write {
                path: path.display().to_string(),
                source,
            })?;
        }
    }
    let temporary = path.with_file_name(format!(
        ".{}.{}.tmp",
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "idmap".to_owned()),
        std::process::id()
    ));
    fs::write(&temporary, json).map_err(|source| IdMapError::Write {
        path: temporary.display().to_string(),
        source,
    })?;
    fs::rename(&temporary, path).map_err(|source| IdMapError::Write {
        path: path.display().to_string(),
        source,
    })
}

/// Serialize a fresh load-mutate-save transaction across cooperating shim
/// processes: lock a dedicated sibling file, re-read `path`, apply `mutate`,
/// write the result back atomically, then unlock.
fn transact(
    path: &Path,
    mutate: impl FnOnce(&mut BTreeMap<String, String>),
) -> Result<(), IdMapError> {
    let lock_file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path(path))
        .map_err(|source| IdMapError::Lock {
            path: path.display().to_string(),
            source,
        })?;
    FileExt::lock(&lock_file).map_err(|source| IdMapError::Lock {
        path: path.display().to_string(),
        source,
    })?;

    let result = (|| {
        let mut entries = read_entries(path)?;
        mutate(&mut entries);
        write_entries(path, &entries)
    })();

    let unlock = FileExt::unlock(&lock_file).map_err(|source| IdMapError::Lock {
        path: path.display().to_string(),
        source,
    });
    match (result, unlock) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn temp_state_path() -> PathBuf {
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("test clock should follow Unix epoch")
            .as_nanos();
        let dir = env::temp_dir().join(format!(
            "teammux-idmap-tests-{}-{nanos}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp idmap test dir");
        dir.join("state.json")
    }

    #[test]
    fn load_treats_missing_file_as_empty() {
        let path = temp_state_path();
        let map = IdMap::load(&path).expect("missing file loads as empty");
        assert_eq!(map.lookup("%0"), None);
    }

    #[test]
    fn insert_persists_and_is_visible_to_a_fresh_load() {
        let path = temp_state_path();
        IdMap::insert(&path, "%0", "wG:p6").expect("insert should succeed");

        let map = IdMap::load(&path).expect("load after insert");
        assert_eq!(map.lookup("%0"), Some("wG:p6"));
    }

    #[test]
    fn insert_overwrites_an_existing_key() {
        let path = temp_state_path();
        IdMap::insert(&path, "%0", "wG:p6").unwrap();
        IdMap::insert(&path, "%0", "wG:p7").unwrap();

        let map = IdMap::load(&path).unwrap();
        assert_eq!(map.lookup("%0"), Some("wG:p7"));
    }

    #[test]
    fn tracks_both_pane_and_window_ids_in_one_table() {
        let path = temp_state_path();
        IdMap::insert(&path, "%1", "wG:p6").unwrap();
        IdMap::insert(&path, "@0", "wG:t2").unwrap();

        let map = IdMap::load(&path).unwrap();
        assert_eq!(map.lookup("%1"), Some("wG:p6"));
        assert_eq!(map.lookup("@0"), Some("wG:t2"));
    }

    #[test]
    fn remove_deletes_the_entry_and_persists() {
        let path = temp_state_path();
        IdMap::insert(&path, "%2", "wG:p8").unwrap();
        IdMap::remove(&path, "%2").expect("remove should succeed");

        let map = IdMap::load(&path).unwrap();
        assert_eq!(map.lookup("%2"), None);
    }

    #[test]
    fn remove_of_absent_key_is_a_harmless_no_op() {
        let path = temp_state_path();
        IdMap::remove(&path, "%9").expect("removing an absent key must not error");
        assert_eq!(IdMap::load(&path).unwrap().lookup("%9"), None);
    }

    #[test]
    fn lookup_misses_return_none() {
        let path = temp_state_path();
        IdMap::insert(&path, "%0", "wG:p1").unwrap();
        let map = IdMap::load(&path).unwrap();
        assert_eq!(map.lookup("%does-not-exist"), None);
    }

    #[test]
    fn malformed_state_file_is_a_parse_error() {
        let path = temp_state_path();
        fs::write(&path, "not json").unwrap();
        match IdMap::load(&path) {
            Err(IdMapError::Parse { .. }) => {}
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn load_from_env_reports_a_clear_error_when_unset() {
        // SAFETY-of-intent: single-threaded within this test process's view of
        // the var; other tests never touch STATE_PATH_ENV.
        env::remove_var(STATE_PATH_ENV);
        match IdMap::load_from_env() {
            Err(IdMapError::StatePathUnset) => {}
            other => panic!("expected StatePathUnset, got {other:?}"),
        }
    }

    #[test]
    fn concurrent_inserts_do_not_lose_updates() {
        let path = temp_state_path();
        let handles: Vec<_> = (0..8)
            .map(|index| {
                let path = path.clone();
                thread::spawn(move || {
                    IdMap::insert(&path, format!("%{index}"), format!("wG:p{index}"))
                        .expect("concurrent insert should not error");
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("writer thread should not panic");
        }

        let map = IdMap::load(&path).unwrap();
        for index in 0..8 {
            assert_eq!(
                map.lookup(&format!("%{index}")),
                Some(format!("wG:p{index}").as_str())
            );
        }
    }
}
