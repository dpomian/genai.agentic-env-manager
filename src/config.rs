use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

/// Environment variable used to point at an alternate config file.
pub(crate) const CONFIG_ENV_VAR: &str = "SKILL_INSTALLER_CONFIG";

/// Config file location relative to the home directory.
const CONFIG_RELATIVE_PATH: &str = ".agents/config.yaml";

/// Written on first run when no config file exists yet.
const DEFAULT_CONFIG_YAML: &str = r#"# skill-installer configuration
#
# Maps an --agent value to the directory where the skill symlink is created.
# Paths may be relative to your home directory (".kiro/skills"), start with
# "~/", or be absolute.
#
# To support a new coding agent / IDE, add an entry here. No rebuild needed.
coding_agents:
  kiro: .kiro/skills
  windsurf: .codeium/windsurf/skills
  copilot: .copilot/skills
  claude: .claude/skills
"#;

/// A resolved symlink destination for one coding agent.
#[derive(Debug, Clone)]
pub struct Target {
    /// The `--agent` value this target came from.
    pub name: String,
    /// Absolute directory in which the skill symlink is created.
    pub dir: PathBuf,
}

/// Legacy `--agent` value that selects every configured agent. Superseded by
/// `--all-agents`, but still accepted so existing scripts keep working.
pub const ALL_AGENTS: &str = "all";

/// Which of the configured coding agents an operation targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentSelection {
    /// Every agent in config.yaml, requested as `--all-agents` (or the legacy
    /// `--agent all`).
    All,
    /// One or more explicitly named agents. Never empty, and deduplicated in
    /// the order the names were given.
    Named(Vec<String>),
}

impl AgentSelection {
    /// Builds a selection from the `--agent` values and the `--all-agents` flag.
    ///
    /// `all` is also accepted as an `--agent` value for backwards compatibility.
    /// No names and no flag means every agent, which read-only commands such as
    /// `list` rely on for their default; `install` and `uninstall` require a
    /// selection at the argument-parsing level so they never reach that case.
    pub fn parse(names: &[String], all: bool) -> Result<Self> {
        let (all_values, named): (Vec<&String>, Vec<&String>) = names
            .iter()
            .partition(|name| name.eq_ignore_ascii_case(ALL_AGENTS));

        let wants_all = all || !all_values.is_empty();

        if wants_all && !named.is_empty() {
            bail!(
                "cannot combine every agent with a named one: drop either \
                 --all-agents / --agent {ALL_AGENTS} or --agent {}",
                named
                    .iter()
                    .map(|n| n.as_str())
                    .collect::<Vec<_>>()
                    .join(" --agent ")
            );
        }

        if wants_all || named.is_empty() {
            return Ok(Self::All);
        }

        Ok(Self::named(named.into_iter().cloned()))
    }

    /// A selection of explicitly named agents, deduplicated in input order.
    /// Falls back to [`Self::All`] when `names` is empty.
    pub fn named<I: IntoIterator<Item = String>>(names: I) -> Self {
        let mut unique: Vec<String> = Vec::new();
        for name in names {
            if !unique.contains(&name) {
                unique.push(name);
            }
        }

        if unique.is_empty() {
            return Self::All;
        }

        Self::Named(unique)
    }

    /// A selection of exactly one named agent. Production code builds its
    /// selection from parsed arguments, so this exists for tests.
    #[cfg(test)]
    pub fn one(name: impl Into<String>) -> Self {
        Self::Named(vec![name.into()])
    }

    /// True when every configured agent is targeted.
    pub fn is_all(&self) -> bool {
        matches!(self, Self::All)
    }
}

impl fmt::Display for AgentSelection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::All => f.write_str(ALL_AGENTS),
            Self::Named(names) => f.write_str(&names.join(", ")),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// Maps coding agent name -> symlink directory.
    pub coding_agents: BTreeMap<String, String>,
    /// Where this config was read from. Not part of the YAML.
    #[serde(skip)]
    pub path: PathBuf,
}

impl Config {
    /// Loads the config, seeding a default file if none exists yet.
    pub fn load() -> Result<Self> {
        let path = Self::path()?;

        if !path.exists() {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).with_context(|| {
                    format!("failed to create config directory '{}'", parent.display())
                })?;
            }
            fs::write(&path, DEFAULT_CONFIG_YAML)
                .with_context(|| format!("failed to write config '{}'", path.display()))?;
            println!("Created default config at {}", path.display());
        }

        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read config '{}'", path.display()))?;

        Self::from_yaml(&raw, &path)
    }

    /// Parses a config from YAML. `path` is recorded for error messages.
    pub fn from_yaml(raw: &str, path: &Path) -> Result<Self> {
        let mut config: Config = serde_yaml::from_str(raw)
            .with_context(|| format!("failed to parse config '{}'", path.display()))?;
        config.path = path.to_path_buf();

        if config.coding_agents.is_empty() {
            bail!(
                "no coding agents configured in '{}'\n\n\
                 Add at least one entry under `coding_agents:`, for example:\n\
                 \x20 coding_agents:\n\
                 \x20   kiro: .kiro/skills",
                path.display()
            );
        }

        Ok(config)
    }

    /// Resolves which home-level directories to link into.
    pub fn resolve(&self, selection: &AgentSelection) -> Result<Vec<Target>> {
        self.resolve_with(selection, None)
    }

    /// Same selection rules as [`Config::resolve`], but each target directory is
    /// rooted at `workspace` instead of the home directory, for project-level
    /// installs.
    pub fn resolve_in_workspace(
        &self,
        selection: &AgentSelection,
        workspace: &Path,
    ) -> Result<Vec<Target>> {
        self.resolve_with(selection, Some(workspace))
    }

    /// Resolves the workspace targets for a selection that names its agents.
    /// Errors if any of them is not configured.
    pub fn resolve_named_in_workspace(
        &self,
        names: &[String],
        workspace: &Path,
    ) -> Result<Vec<Target>> {
        names
            .iter()
            .map(|name| {
                let dir = self
                    .coding_agents
                    .get(name)
                    .ok_or_else(|| self.unknown_agent_error(name))?;
                self.target(name, dir, Some(workspace))
            })
            .collect()
    }

    fn resolve_with(
        &self,
        selection: &AgentSelection,
        workspace: Option<&Path>,
    ) -> Result<Vec<Target>> {
        match selection {
            AgentSelection::Named(names) => names
                .iter()
                .map(|name| {
                    let dir = self
                        .coding_agents
                        .get(name)
                        .ok_or_else(|| self.unknown_agent_error(name))?;
                    self.target(name, dir, workspace)
                })
                .collect(),
            AgentSelection::All => self
                .coding_agents
                .iter()
                .map(|(name, dir)| self.target(name, dir, workspace))
                .collect(),
        }
    }

    fn target(&self, name: &str, dir: &str, workspace: Option<&Path>) -> Result<Target> {
        let dir = match workspace {
            Some(workspace) => self.workspace_dir(name, dir, workspace)?,
            None => self.absolute_dir(dir)?,
        };

        Ok(Target {
            name: name.to_string(),
            dir,
        })
    }

    /// Maps a configured agent directory into a workspace.
    ///
    /// Home-relative entries (".kiro/skills") are reused as-is under the
    /// workspace, and a leading "~/" is stripped. An absolute entry has no
    /// meaningful project-level equivalent, so it falls back to
    /// `<workspace>/.<agent>/skills`.
    fn workspace_dir(&self, name: &str, dir: &str, workspace: &Path) -> Result<PathBuf> {
        let relative = self.relative_dir(dir)?;

        if Path::new(&relative).is_absolute() {
            return Ok(workspace.join(format!(".{name}")).join("skills"));
        }

        Ok(workspace.join(relative))
    }

    /// Expands `~/` and home-relative paths into absolute ones.
    fn absolute_dir(&self, dir: &str) -> Result<PathBuf> {
        let relative = self.relative_dir(dir)?;

        let path = Path::new(&relative);
        if path.is_absolute() {
            return Ok(path.to_path_buf());
        }

        Ok(home_dir()?.join(relative))
    }

    /// Trims a configured directory and strips a leading `~/`, leaving either a
    /// relative path or an absolute one.
    fn relative_dir(&self, dir: &str) -> Result<String> {
        let dir = dir.trim();
        if dir.is_empty() {
            bail!(
                "empty directory configured in '{}'; every coding agent needs a path",
                self.path.display()
            );
        }

        Ok(dir.strip_prefix("~/").unwrap_or(dir).to_string())
    }

    fn unknown_agent_error(&self, name: &str) -> anyhow::Error {
        anyhow::anyhow!(
            "unknown coding agent '{name}'\n\n\
             Configured coding agents in {}:\n{}\n\n\
             Use --all-agents to target every configured agent, or add an entry \
             under `coding_agents:` in that file to support a new one.",
            self.path.display(),
            self.configured_agents()
        )
    }

    /// Error for a project-level install that asked for every agent at once.
    pub fn all_agents_in_workspace_error(&self) -> anyhow::Error {
        anyhow::anyhow!(
            "--all-agents is not supported with --workspace; name the agents you \
             want so a project only gets directories for the tools it uses\n\n\
             Configured coding agents in {}:\n{}",
            self.path.display(),
            self.configured_agents()
        )
    }

    /// The configured agents, one `name -> dir` per line, for error messages.
    fn configured_agents(&self) -> String {
        self.coding_agents
            .iter()
            .map(|(agent, dir)| format!("  {agent} -> {dir}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn path() -> Result<PathBuf> {
        if let Some(path) = std::env::var_os(CONFIG_ENV_VAR) {
            return Ok(PathBuf::from(path));
        }
        Ok(home_dir()?.join(CONFIG_RELATIVE_PATH))
    }
}

pub fn home_dir() -> Result<PathBuf> {
    dirs::home_dir().context("could not determine home directory")
}

#[cfg(test)]
mod tests {
    use super::*;

    const YAML: &str = "coding_agents:\n  kiro: .kiro/skills\n  claude: .claude/skills\n";

    fn config(raw: &str) -> Config {
        Config::from_yaml(raw, Path::new("/tmp/config.yaml")).expect("config should parse")
    }

    fn one(name: &str) -> AgentSelection {
        AgentSelection::one(name)
    }

    fn values(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn resolves_a_known_agent_to_a_single_target() {
        let targets = config(YAML).resolve(&one("kiro")).unwrap();

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].name, "kiro");
        assert_eq!(targets[0].dir, home_dir().unwrap().join(".kiro/skills"));
    }

    #[test]
    fn resolves_several_named_agents_in_the_order_given() {
        let selection = AgentSelection::parse(&values(&["kiro", "claude"]), false).unwrap();
        let targets = config(YAML).resolve(&selection).unwrap();

        let names: Vec<_> = targets.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["kiro", "claude"]);
    }

    #[test]
    fn resolves_every_agent_for_all() {
        let targets = config(YAML).resolve(&AgentSelection::All).unwrap();

        let names: Vec<_> = targets.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["claude", "kiro"]);
    }

    #[test]
    fn all_agents_flag_selects_every_agent() {
        assert_eq!(
            AgentSelection::parse(&[], true).unwrap(),
            AgentSelection::All
        );
    }

    #[test]
    fn legacy_all_value_is_still_accepted_case_insensitively() {
        assert_eq!(
            AgentSelection::parse(&values(&["all"]), false).unwrap(),
            AgentSelection::All
        );
        assert_eq!(
            AgentSelection::parse(&values(&["ALL"]), false).unwrap(),
            AgentSelection::All
        );
    }

    #[test]
    fn no_selection_at_all_means_every_agent() {
        assert_eq!(
            AgentSelection::parse(&[], false).unwrap(),
            AgentSelection::All
        );
    }

    #[test]
    fn named_agents_are_deduplicated_in_input_order() {
        let selection = AgentSelection::parse(&values(&["kiro", "claude", "kiro"]), false).unwrap();

        assert_eq!(
            selection,
            AgentSelection::Named(vec!["kiro".to_string(), "claude".to_string()])
        );
    }

    #[test]
    fn mixing_every_agent_with_a_named_one_is_rejected() {
        let err = AgentSelection::parse(&values(&["kiro"]), true).unwrap_err();
        assert!(
            format!("{err}").contains("cannot combine every agent with a named one"),
            "{err}"
        );

        let err = AgentSelection::parse(&values(&["all", "kiro"]), false).unwrap_err();
        assert!(
            format!("{err}").contains("cannot combine every agent with a named one"),
            "{err}"
        );
    }

    #[test]
    fn selection_reports_and_renders_itself() {
        assert!(AgentSelection::All.is_all());
        assert!(!one("kiro").is_all());
        assert_eq!(AgentSelection::All.to_string(), "all");
        assert_eq!(one("kiro").to_string(), "kiro");
        assert_eq!(
            AgentSelection::named(values(&["kiro", "claude"])).to_string(),
            "kiro, claude"
        );
    }

    #[test]
    fn unknown_agent_error_lists_configured_agents_and_mentions_all() {
        let err = config(YAML).resolve(&one("cursor")).unwrap_err();
        let message = format!("{err}");

        assert!(message.contains("unknown coding agent 'cursor'"), "{message}");
        assert!(message.contains("kiro -> .kiro/skills"), "{message}");
        assert!(message.contains("/tmp/config.yaml"), "{message}");
        assert!(message.contains("--all-agents"), "{message}");
    }

    #[test]
    fn absolute_and_tilde_paths_are_honoured() {
        let config = config("coding_agents:\n  abs: /opt/skills\n  tilde: ~/nested/skills\n");

        let targets = config.resolve(&AgentSelection::All).unwrap();
        assert_eq!(targets[0].dir, PathBuf::from("/opt/skills"));
        assert_eq!(
            targets[1].dir,
            home_dir().unwrap().join("nested/skills")
        );
    }

    #[test]
    fn empty_mapping_is_rejected() {
        let err = Config::from_yaml("coding_agents: {}\n", Path::new("/tmp/config.yaml"))
            .unwrap_err();
        assert!(format!("{err}").contains("no coding agents configured"));
    }

    #[test]
    fn workspace_targets_are_rooted_at_the_workspace() {
        let targets = config(YAML)
            .resolve_named_in_workspace(&values(&["kiro"]), Path::new("/work/project"))
            .unwrap();

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].dir, PathBuf::from("/work/project/.kiro/skills"));
    }

    #[test]
    fn workspace_resolves_several_named_agents() {
        let targets = config(YAML)
            .resolve_named_in_workspace(&values(&["kiro", "claude"]), Path::new("/work/project"))
            .unwrap();

        let dirs: Vec<_> = targets.iter().map(|t| t.dir.clone()).collect();
        assert_eq!(
            dirs,
            vec![
                PathBuf::from("/work/project/.kiro/skills"),
                PathBuf::from("/work/project/.claude/skills"),
            ]
        );
    }

    #[test]
    fn workspace_resolves_every_agent_for_all() {
        let targets = config(YAML)
            .resolve_in_workspace(&AgentSelection::All, Path::new("/work/project"))
            .unwrap();

        let dirs: Vec<_> = targets.iter().map(|t| t.dir.clone()).collect();
        assert_eq!(
            dirs,
            vec![
                PathBuf::from("/work/project/.claude/skills"),
                PathBuf::from("/work/project/.kiro/skills"),
            ]
        );
    }

    #[test]
    fn workspace_strips_tilde_and_falls_back_for_absolute_entries() {
        let config = config("coding_agents:\n  abs: /opt/skills\n  tilde: ~/nested/skills\n");

        let targets = config
            .resolve_in_workspace(&AgentSelection::All, Path::new("/work/project"))
            .unwrap();

        // An absolute config entry has no project-level equivalent.
        assert_eq!(targets[0].dir, PathBuf::from("/work/project/.abs/skills"));
        assert_eq!(
            targets[1].dir,
            PathBuf::from("/work/project/nested/skills")
        );
    }

    #[test]
    fn workspace_rejects_an_unknown_agent() {
        let err = config(YAML)
            .resolve_named_in_workspace(&values(&["cursor"]), Path::new("/work/project"))
            .unwrap_err();
        assert!(format!("{err}").contains("unknown coding agent 'cursor'"));
    }

    #[test]
    fn default_config_is_valid_and_covers_the_known_agents() {
        let config = config(DEFAULT_CONFIG_YAML);
        let names: Vec<_> = config.coding_agents.keys().cloned().collect();

        assert_eq!(names, vec!["claude", "copilot", "kiro", "windsurf"]);
        assert!(config.resolve(&one("windsurf")).is_ok());
    }
}
