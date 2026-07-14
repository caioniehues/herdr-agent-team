//! Team-spec parsing, resolution, validation, and dry-run output (spec section 2).

use crate::launcher::default_launcher_table;
use crate::types::{GodSpec, LauncherTable, TeamSpec, Topology, WorkerSpec};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

const DEFAULT_SPEC_PATH: &str = "herdr-team.toml";
const SHORTHAND_TEAM_NAME: &str = "adhoc";
const SHORTHAND_ROLE: &str = "worker";

#[derive(Debug, Error)]
pub enum SpecError {
    #[error("cannot read team spec {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid team spec TOML: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("invalid team spec: {0}")]
    Validation(String),
    #[error("invalid spawn arguments: {0}")]
    Cli(String),
    #[error("cannot determine the current directory: {0}")]
    CurrentDirectory(#[source] std::io::Error),
    #[error("team spawn is not implemented (ticket 07); use --dry-run")]
    SpawnNotImplemented,
}

#[derive(Debug, Deserialize)]
struct RawTeamSpec {
    name: String,
    topology: Option<Topology>,
    cwd: Option<PathBuf>,
    #[serde(default)]
    setup: Vec<String>,
    god: Option<RawGodSpec>,
    workers: Vec<RawWorkerSpec>,
}

#[derive(Debug, Deserialize)]
struct RawGodSpec {
    target: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawWorkerSpec {
    name: String,
    agent: String,
    role: String,
    worktree: Option<bool>,
    branch: Option<String>,
    brief: PathBuf,
}

#[derive(Debug, PartialEq, Eq)]
struct SpawnOptions {
    dry_run: bool,
    source: SpecSource,
}

#[derive(Debug, PartialEq, Eq)]
enum SpecSource {
    File(PathBuf),
    Agents(String),
}

pub fn parse_team_spec(source: &str) -> Result<TeamSpec, SpecError> {
    let raw = toml::from_str::<RawTeamSpec>(source)?;
    Ok(resolve_raw_spec(raw))
}

pub fn load_team_spec(path: &Path, launchers: &LauncherTable) -> Result<TeamSpec, SpecError> {
    let source = fs::read_to_string(path).map_err(|source| SpecError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let spec = parse_team_spec(&source)?;
    validate_team_spec(&spec, launchers)?;
    Ok(spec)
}

pub fn team_spec_from_agents(
    agents: &str,
    cwd: &Path,
    launchers: &LauncherTable,
) -> Result<TeamSpec, SpecError> {
    let agent_kinds = agents
        .split(',')
        .map(str::trim)
        .filter(|agent| !agent.is_empty())
        .collect::<Vec<_>>();

    if agent_kinds.is_empty() {
        return Err(SpecError::Cli(
            "--agents requires at least one comma-separated agent kind".to_owned(),
        ));
    }

    let workers = agent_kinds
        .into_iter()
        .enumerate()
        .map(|(index, agent)| {
            let name = format!("{agent}-{}", index + 1);
            WorkerSpec {
                brief: PathBuf::from("briefs").join(format!("{name}.md")),
                name,
                agent: agent.to_owned(),
                role: SHORTHAND_ROLE.to_owned(),
                worktree: false,
                branch: None,
            }
        })
        .collect();

    let spec = TeamSpec {
        name: SHORTHAND_TEAM_NAME.to_owned(),
        topology: Topology::Star,
        cwd: cwd.to_path_buf(),
        setup: Vec::new(),
        god: GodSpec::default(),
        workers,
    };
    validate_team_spec(&spec, launchers)?;
    Ok(spec)
}

pub fn validate_team_spec(spec: &TeamSpec, launchers: &LauncherTable) -> Result<(), SpecError> {
    let mut names = BTreeSet::new();

    for worker in &spec.workers {
        if !names.insert(worker.name.as_str()) {
            return Err(validation_error(
                worker,
                format!("duplicate worker name '{}'", worker.name),
            ));
        }

        if !launchers.contains_key(&worker.agent) {
            let available = if launchers.is_empty() {
                "none".to_owned()
            } else {
                launchers.keys().cloned().collect::<Vec<_>>().join(", ")
            };
            return Err(validation_error(
                worker,
                format!(
                    "unknown agent '{}'; available agent kinds: {available}",
                    worker.agent
                ),
            ));
        }

        match (worker.worktree, worker.branch.as_deref()) {
            (true, None | Some("")) => {
                return Err(validation_error(
                    worker,
                    "branch is required when worktree = true",
                ));
            }
            (false, Some(_)) => {
                return Err(validation_error(
                    worker,
                    "branch must be omitted when worktree = false",
                ));
            }
            _ => {}
        }
    }

    Ok(())
}

pub fn render_spawn_plan(spec: &TeamSpec) -> String {
    let mut plan = String::new();
    writeln!(plan, "team: {}", spec.name).expect("writing to a String cannot fail");
    writeln!(
        plan,
        "topology: {}",
        match spec.topology {
            Topology::Star => "star",
            Topology::Mesh => "mesh",
        }
    )
    .expect("writing to a String cannot fail");
    writeln!(plan, "cwd: {}", spec.cwd.display()).expect("writing to a String cannot fail");
    writeln!(plan, "god: {}", spec.god.target).expect("writing to a String cannot fail");
    if spec.setup.is_empty() {
        writeln!(plan, "setup: none").expect("writing to a String cannot fail");
    } else {
        writeln!(plan, "setup: {}", spec.setup.join(" ")).expect("writing to a String cannot fail");
    }
    writeln!(plan, "workers:").expect("writing to a String cannot fail");

    for worker in &spec.workers {
        writeln!(
            plan,
            "  - {}: agent={} role={} worktree={} branch={} brief={}",
            worker.name,
            worker.agent,
            worker.role,
            worker.worktree,
            worker.branch.as_deref().unwrap_or("none"),
            worker.brief.display()
        )
        .expect("writing to a String cannot fail");
    }

    plan
}

pub fn spawn_command(args: &[String]) -> Result<(), SpecError> {
    let options = parse_spawn_args(args)?;
    if !options.dry_run {
        return Err(SpecError::SpawnNotImplemented);
    }

    let launchers = default_launcher_table();
    let current_dir = std::env::current_dir().map_err(SpecError::CurrentDirectory)?;
    let spec = resolve_source(&options.source, &current_dir, &launchers)?;
    print!("{}", render_spawn_plan(&spec));
    Ok(())
}

fn resolve_raw_spec(raw: RawTeamSpec) -> TeamSpec {
    TeamSpec {
        name: raw.name,
        topology: raw.topology.unwrap_or_default(),
        cwd: raw.cwd.unwrap_or_else(|| PathBuf::from(".")),
        setup: raw.setup,
        god: GodSpec {
            target: raw
                .god
                .and_then(|god| god.target)
                .unwrap_or_else(|| "self".to_owned()),
        },
        workers: raw
            .workers
            .into_iter()
            .map(|worker| WorkerSpec {
                name: worker.name,
                agent: worker.agent,
                worktree: worker.worktree.unwrap_or_else(|| worker.role == "builder"),
                role: worker.role,
                branch: worker.branch,
                brief: worker.brief,
            })
            .collect(),
    }
}

fn validation_error(worker: &WorkerSpec, detail: impl Into<String>) -> SpecError {
    SpecError::Validation(format!("worker '{}': {}", worker.name, detail.into()))
}

fn parse_spawn_args(args: &[String]) -> Result<SpawnOptions, SpecError> {
    let mut dry_run = false;
    let mut agents = None;
    let mut spec_path = None;
    let mut index = 0;

    while index < args.len() {
        let argument = &args[index];
        match argument.as_str() {
            "--dry-run" => dry_run = true,
            "--agents" => {
                index += 1;
                let value = args.get(index).ok_or_else(|| {
                    SpecError::Cli("--agents requires a comma-separated value".to_owned())
                })?;
                set_agents(&mut agents, value)?;
            }
            value if value.starts_with("--agents=") => {
                set_agents(&mut agents, &value["--agents=".len()..])?;
            }
            value if value.starts_with('-') => {
                return Err(SpecError::Cli(format!("unknown option '{value}'")));
            }
            value => {
                if spec_path.replace(PathBuf::from(value)).is_some() {
                    return Err(SpecError::Cli(
                        "only one team spec path may be supplied".to_owned(),
                    ));
                }
            }
        }
        index += 1;
    }

    if agents.is_some() && spec_path.is_some() {
        return Err(SpecError::Cli(
            "--agents and a team spec path are mutually exclusive".to_owned(),
        ));
    }

    let source = match agents {
        Some(agents) => SpecSource::Agents(agents),
        None => SpecSource::File(spec_path.unwrap_or_else(|| PathBuf::from(DEFAULT_SPEC_PATH))),
    };
    Ok(SpawnOptions { dry_run, source })
}

fn set_agents(slot: &mut Option<String>, value: &str) -> Result<(), SpecError> {
    if slot.replace(value.to_owned()).is_some() {
        return Err(SpecError::Cli(
            "--agents may only be supplied once".to_owned(),
        ));
    }
    Ok(())
}

fn resolve_source(
    source: &SpecSource,
    cwd: &Path,
    launchers: &LauncherTable,
) -> Result<TeamSpec, SpecError> {
    match source {
        SpecSource::File(path) => load_team_spec(path, launchers),
        SpecSource::Agents(agents) => team_spec_from_agents(agents, cwd, launchers),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AgentsMdMode, LauncherEntry};

    const EXAMPLE: &str = r#"
name = "limux-wave3"
topology = "star"
cwd = "."
setup = ["./scripts/worktree-setup.sh"]

[god]
target = "self"

[[workers]]
name = "builder-1"
agent = "claude"
role = "builder"
worktree = true
branch = "feat/wave3-builder-1"
brief = "briefs/builder-1.md"

[[workers]]
name = "reviewer-1"
agent = "codex"
role = "reviewer"
worktree = false
brief = "briefs/reviewer-1.md"
"#;

    fn launchers() -> LauncherTable {
        ["claude", "codex"]
            .into_iter()
            .map(|agent| {
                (
                    agent.to_owned(),
                    LauncherEntry {
                        command: vec![agent.to_owned()],
                        submit: vec!["Enter".to_owned()],
                        submit_verify: true,
                        reads_agents_md: if agent == "claude" {
                            AgentsMdMode::Pointer
                        } else {
                            AgentsMdMode::Native
                        },
                    },
                )
            })
            .collect()
    }

    fn worker(name: &str, agent: &str) -> WorkerSpec {
        WorkerSpec {
            name: name.to_owned(),
            agent: agent.to_owned(),
            role: "reviewer".to_owned(),
            worktree: false,
            branch: None,
            brief: PathBuf::from(format!("briefs/{name}.md")),
        }
    }

    fn team(workers: Vec<WorkerSpec>) -> TeamSpec {
        TeamSpec {
            name: "test-team".to_owned(),
            topology: Topology::Star,
            cwd: PathBuf::from("."),
            setup: Vec::new(),
            god: GodSpec::default(),
            workers,
        }
    }

    #[test]
    fn parses_every_example_field() {
        let spec = parse_team_spec(EXAMPLE).expect("example should parse");

        assert_eq!(spec.name, "limux-wave3");
        assert_eq!(spec.topology, Topology::Star);
        assert_eq!(spec.cwd, PathBuf::from("."));
        assert_eq!(spec.setup, ["./scripts/worktree-setup.sh"]);
        assert_eq!(spec.god.target, "self");
        assert_eq!(spec.workers.len(), 2);
        assert_eq!(
            spec.workers[0],
            WorkerSpec {
                name: "builder-1".to_owned(),
                agent: "claude".to_owned(),
                role: "builder".to_owned(),
                worktree: true,
                branch: Some("feat/wave3-builder-1".to_owned()),
                brief: PathBuf::from("briefs/builder-1.md"),
            }
        );
        assert_eq!(
            spec.workers[1],
            WorkerSpec {
                name: "reviewer-1".to_owned(),
                agent: "codex".to_owned(),
                role: "reviewer".to_owned(),
                worktree: false,
                branch: None,
                brief: PathBuf::from("briefs/reviewer-1.md"),
            }
        );
    }

    #[test]
    fn resolves_all_documented_defaults() {
        let spec = parse_team_spec(
            r#"
name = "defaults"

[[workers]]
name = "builder-1"
agent = "claude"
role = "builder"
branch = "feat/default-builder"
brief = "briefs/builder-1.md"

[[workers]]
name = "reviewer-1"
agent = "codex"
role = "reviewer"
brief = "briefs/reviewer-1.md"
"#,
        )
        .expect("minimal spec should parse");

        assert_eq!(spec.topology, Topology::Star);
        assert_eq!(spec.cwd, PathBuf::from("."));
        assert!(spec.setup.is_empty());
        assert_eq!(spec.god.target, "self");
        assert!(spec.workers[0].worktree);
        assert!(!spec.workers[1].worktree);
    }

    #[test]
    fn rejects_duplicate_worker_names_and_names_the_worker() {
        let error = validate_team_spec(
            &team(vec![worker("same", "claude"), worker("same", "codex")]),
            &launchers(),
        )
        .expect_err("duplicate names must fail")
        .to_string();

        assert!(error.contains("worker 'same'"));
        assert!(error.contains("duplicate worker name"));
    }

    #[test]
    fn rejects_unknown_agent_and_names_the_worker() {
        let error = validate_team_spec(&team(vec![worker("mystery", "gemini")]), &launchers())
            .expect_err("unknown agent must fail")
            .to_string();

        assert!(error.contains("worker 'mystery'"));
        assert!(error.contains("unknown agent 'gemini'"));
        assert!(error.contains("claude, codex"));
    }

    #[test]
    fn rejects_worktree_without_branch_and_names_the_worker() {
        let mut builder = worker("builder-1", "claude");
        builder.role = "builder".to_owned();
        builder.worktree = true;

        let error = validate_team_spec(&team(vec![builder]), &launchers())
            .expect_err("worktree without branch must fail")
            .to_string();

        assert!(error.contains("worker 'builder-1'"));
        assert!(error.contains("branch is required"));
    }

    #[test]
    fn rejects_branch_when_worktree_is_false() {
        let mut reviewer = worker("reviewer-1", "codex");
        reviewer.branch = Some("unused-branch".to_owned());

        let error = validate_team_spec(&team(vec![reviewer]), &launchers())
            .expect_err("branch without worktree must fail")
            .to_string();

        assert!(error.contains("worker 'reviewer-1'"));
        assert!(error.contains("branch must be omitted"));
    }

    #[test]
    fn agents_shorthand_builds_a_valid_throwaway_spec() {
        let spec = team_spec_from_agents("claude,codex", Path::new("/project"), &launchers())
            .expect("shorthand should resolve");

        assert_eq!(spec.name, "adhoc");
        assert_eq!(spec.topology, Topology::Star);
        assert_eq!(spec.cwd, PathBuf::from("/project"));
        assert_eq!(spec.god.target, "self");
        assert_eq!(spec.workers.len(), 2);
        assert_eq!(spec.workers[0].name, "claude-1");
        assert_eq!(spec.workers[0].brief, PathBuf::from("briefs/claude-1.md"));
        assert_eq!(spec.workers[1].name, "codex-2");
        assert!(spec.workers.iter().all(|worker| !worker.worktree));
        assert!(spec.workers.iter().all(|worker| worker.branch.is_none()));
        validate_team_spec(&spec, &launchers()).expect("shorthand spec should remain valid");
    }

    #[test]
    fn agents_shorthand_rejects_an_empty_list() {
        let error = team_spec_from_agents(" , ", Path::new("/project"), &launchers())
            .expect_err("empty shorthand must fail")
            .to_string();

        assert!(error.contains("at least one"));
    }

    #[test]
    fn dry_run_plan_contains_every_resolved_worker_field() {
        let spec = parse_team_spec(EXAMPLE).expect("example should parse");
        let plan = render_spawn_plan(&spec);

        assert!(plan.contains("team: limux-wave3"));
        assert!(plan.contains("topology: star"));
        assert!(plan.contains("god: self"));
        assert!(plan.contains("setup: ./scripts/worktree-setup.sh"));
        assert!(plan.contains(
            "builder-1: agent=claude role=builder worktree=true branch=feat/wave3-builder-1 brief=briefs/builder-1.md"
        ));
        assert!(plan.contains(
            "reviewer-1: agent=codex role=reviewer worktree=false branch=none brief=briefs/reviewer-1.md"
        ));
    }

    #[test]
    fn parses_dry_run_file_and_agents_cli_forms() {
        assert_eq!(
            parse_spawn_args(&["--dry-run".to_owned(), "team.toml".to_owned()])
                .expect("file form should parse"),
            SpawnOptions {
                dry_run: true,
                source: SpecSource::File(PathBuf::from("team.toml")),
            }
        );
        assert_eq!(
            parse_spawn_args(&["--agents=claude,codex".to_owned(), "--dry-run".to_owned()])
                .expect("agents form should parse"),
            SpawnOptions {
                dry_run: true,
                source: SpecSource::Agents("claude,codex".to_owned()),
            }
        );
    }

    #[test]
    fn cli_rejects_conflicting_sources_and_unknown_options() {
        let conflict = parse_spawn_args(&[
            "--agents".to_owned(),
            "claude".to_owned(),
            "team.toml".to_owned(),
        ])
        .expect_err("conflicting sources must fail")
        .to_string();
        assert!(conflict.contains("mutually exclusive"));

        let unknown = parse_spawn_args(&["--surprise".to_owned()])
            .expect_err("unknown option must fail")
            .to_string();
        assert!(unknown.contains("unknown option '--surprise'"));
    }

    #[test]
    fn non_dry_run_defers_to_ticket_07_without_loading_launchers() {
        let error = spawn_command(&[])
            .expect_err("real spawn must remain deferred")
            .to_string();

        assert!(error.contains("not implemented (ticket 07)"));
    }
}
