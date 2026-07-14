//! Data-driven coding-agent launcher table from `docs/spec.md` section 3.

use crate::types::AgentsMdMode;
use crate::types::{LauncherEntry, LauncherTable};
use std::{fs, io, path::Path};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LauncherError {
    #[error("failed to read launcher config {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("failed to parse launcher config {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: toml::de::Error,
    },
    #[error("unknown agent kind `{agent}`; available kinds: {available}")]
    UnknownAgent { agent: String, available: String },
}

pub fn default_launcher_table() -> LauncherTable {
    [
        (
            "claude".to_owned(),
            LauncherEntry {
                command: vec!["claude".to_owned()],
                submit: vec!["Enter".to_owned()],
                submit_verify: true,
                reads_agents_md: AgentsMdMode::Pointer,
            },
        ),
        (
            "codex".to_owned(),
            LauncherEntry {
                command: vec!["codex".to_owned()],
                submit: vec!["Enter".to_owned()],
                submit_verify: true,
                reads_agents_md: AgentsMdMode::Native,
            },
        ),
    ]
    .into_iter()
    .collect()
}

pub fn load_launcher_table(config_dir: &Path) -> Result<LauncherTable, LauncherError> {
    let config_path = config_dir.join("agents.toml");
    let contents = match fs::read_to_string(&config_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(default_launcher_table())
        }
        Err(source) => {
            return Err(LauncherError::Read {
                path: config_path.display().to_string(),
                source,
            });
        }
    };

    let overrides: LauncherTable =
        toml::from_str(&contents).map_err(|source| LauncherError::Parse {
            path: config_path.display().to_string(),
            source,
        })?;
    let mut table = default_launcher_table();
    table.extend(overrides);
    Ok(table)
}

pub fn launcher_entry<'a>(
    table: &'a LauncherTable,
    agent: &str,
) -> Result<&'a LauncherEntry, LauncherError> {
    table.get(agent).ok_or_else(|| LauncherError::UnknownAgent {
        agent: agent.to_owned(),
        available: table.keys().cloned().collect::<Vec<_>>().join(", "),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT_TEMP_DIR: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let suffix = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "herdr-agent-team-launcher-{}-{suffix}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("create launcher test directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).expect("remove launcher test directory");
        }
    }

    #[test]
    fn default_table_matches_live_verified_launchers() {
        let table = default_launcher_table();

        assert_eq!(table.len(), 2);
        assert_eq!(
            table["claude"],
            LauncherEntry {
                command: vec!["claude".to_owned()],
                submit: vec!["Enter".to_owned()],
                submit_verify: true,
                reads_agents_md: AgentsMdMode::Pointer,
            }
        );
        assert_eq!(
            table["codex"],
            LauncherEntry {
                command: vec!["codex".to_owned()],
                submit: vec!["Enter".to_owned()],
                submit_verify: true,
                reads_agents_md: AgentsMdMode::Native,
            }
        );
    }

    #[test]
    fn config_entries_replace_defaults_wholesale_and_extend_the_table() {
        let config_dir = TempDir::new();
        fs::write(
            config_dir.path().join("agents.toml"),
            r#"
[claude]
command = ["claude", "--model", "opus"]
submit = ["C-m"]
submit_verify = false
reads_agents_md = "native"

[opencode]
command = ["opencode"]
submit = ["Enter"]
submit_verify = true
reads_agents_md = "pointer"
"#,
        )
        .expect("write launcher config");

        let table = load_launcher_table(config_dir.path()).expect("load launcher config");

        assert_eq!(table.len(), 3);
        assert_eq!(table["claude"].command, ["claude", "--model", "opus"]);
        assert_eq!(table["claude"].submit, ["C-m"]);
        assert!(!table["claude"].submit_verify);
        assert_eq!(table["claude"].reads_agents_md, AgentsMdMode::Native);
        assert_eq!(table["codex"], default_launcher_table()["codex"]);
        assert_eq!(table["opencode"].command, ["opencode"]);
    }

    #[test]
    fn unknown_agent_error_names_the_available_kinds() {
        let table = default_launcher_table();

        let error = launcher_entry(&table, "gemini").expect_err("unknown kind must fail");

        assert_eq!(
            error.to_string(),
            "unknown agent kind `gemini`; available kinds: claude, codex"
        );
    }
}
