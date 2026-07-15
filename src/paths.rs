//! Self-resolution of Herdr plugin directories (spec section 13).

use std::env;
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const PLUGIN_ID: &str = "caioniehues.agent-team";

#[derive(Debug, Error)]
pub enum PathError {
    #[error("cannot resolve {name}: set {name} or HOME (XDG overrides are supported)")]
    Unresolved { name: &'static str },
}

pub fn state_dir() -> Result<PathBuf, PathError> {
    resolve_dir(
        "HERDR_PLUGIN_STATE_DIR",
        "XDG_STATE_HOME",
        ".local/state",
        Path::new("herdr/plugins").join(PLUGIN_ID),
    )
}

pub fn config_dir() -> Result<PathBuf, PathError> {
    resolve_dir(
        "HERDR_PLUGIN_CONFIG_DIR",
        "XDG_CONFIG_HOME",
        ".config",
        Path::new("herdr/plugins/config").join(PLUGIN_ID),
    )
}

/// Populate the same variables Herdr injects for direct PATH invocation.
pub fn hydrate_environment() {
    if env::var_os("HERDR_PLUGIN_STATE_DIR").is_none() {
        if let Ok(path) = state_dir() {
            env::set_var("HERDR_PLUGIN_STATE_DIR", path);
        }
    }
    if env::var_os("HERDR_PLUGIN_CONFIG_DIR").is_none() {
        if let Ok(path) = config_dir() {
            env::set_var("HERDR_PLUGIN_CONFIG_DIR", path);
        }
    }
}

fn resolve_dir(
    explicit: &'static str,
    xdg: &str,
    home_suffix: &str,
    herdr_suffix: PathBuf,
) -> Result<PathBuf, PathError> {
    resolve_dir_values(
        explicit,
        env::var_os(explicit),
        env::var_os(xdg),
        env::var_os("HOME"),
        home_suffix,
        herdr_suffix,
    )
}

fn resolve_dir_values(
    explicit_name: &'static str,
    explicit: Option<std::ffi::OsString>,
    xdg: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
    home_suffix: &str,
    herdr_suffix: PathBuf,
) -> Result<PathBuf, PathError> {
    if let Some(path) = explicit {
        return Ok(PathBuf::from(path));
    }
    let base = xdg
        .map(PathBuf::from)
        .or_else(|| home.map(|home| PathBuf::from(home).join(home_suffix)))
        .ok_or(PathError::Unresolved {
            name: explicit_name,
        })?;
    Ok(base.join(herdr_suffix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_environment_wins_and_well_known_layout_is_fallback() {
        assert_eq!(
            resolve_dir_values(
                "HERDR_PLUGIN_STATE_DIR",
                Some("/explicit/state".into()),
                Some("/xdg/state".into()),
                None,
                ".local/state",
                Path::new("herdr/plugins").join(PLUGIN_ID),
            )
            .unwrap(),
            PathBuf::from("/explicit/state")
        );
        assert_eq!(
            resolve_dir_values(
                "HERDR_PLUGIN_STATE_DIR",
                None,
                Some("/xdg/state".into()),
                None,
                ".local/state",
                Path::new("herdr/plugins").join(PLUGIN_ID),
            )
            .unwrap(),
            PathBuf::from("/xdg/state/herdr/plugins").join(PLUGIN_ID)
        );
    }

    #[test]
    fn missing_environment_has_a_clear_error() {
        assert!(resolve_dir_values(
            "HERDR_PLUGIN_CONFIG_DIR",
            None,
            None,
            None,
            ".config",
            Path::new("herdr/plugins/config").join(PLUGIN_ID),
        )
        .unwrap_err()
        .to_string()
        .contains("HERDR_PLUGIN_CONFIG_DIR"));
    }

    #[test]
    fn plugin_id_matches_the_authored_manifest() {
        let manifest: toml::Value = toml::from_str(include_str!("../herdr-plugin.toml")).unwrap();
        assert_eq!(manifest["id"].as_str(), Some(PLUGIN_ID));
    }
}
