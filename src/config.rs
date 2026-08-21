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
# Maps a --coding-agent value to the directory where the skill symlink is
# created. Paths may be relative to your home directory (".kiro/skills"),
# start with "~/", or be absolute.
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
    /// The `--coding-agent` value this target came from.
    pub name: String,
    /// Absolute directory in which the skill symlink is created.
    pub dir: PathBuf,
}

/// The `--coding-agent` value that selects every configured agent.
pub const ALL_AGENTS: &str = "all";

/// Which of the configured coding agents an operation targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentSelection {
    /// Every agent in config.yaml, requested explicitly as `all`.
    All,
    /// A single named agent.
    One(String),
}

impl AgentSelection {
    /// True when every configured agent is targeted.
    pub fn is_all(&self) -> bool {
        matches!(self, Self::All)
    }
}

impl From<&str> for AgentSelection {
    fn from(value: &str) -> Self {
        if value.eq_ignore_ascii_case(ALL_AGENTS) {
            Self::All
        } else {
            Self::One(value.to_string())
        }
    }
}

impl fmt::Display for AgentSelection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::All => f.write_str(ALL_AGENTS),
            Self::One(name) => f.write_str(name),
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

    /// Resolves the single workspace target for a named coding agent. Errors if
    /// the agent is not configured.
    pub fn resolve_one_in_workspace(&self, coding_agent: &str, workspace: &Path) -> Result<Target> {
        let dir = self
            .coding_agents
            .get(coding_agent)
            .ok_or_else(|| self.unknown_agent_error(coding_agent))?;

        self.target(coding_agent, dir, Some(workspace))
    }

    fn resolve_with(
        &self,
        selection: &AgentSelection,
        workspace: Option<&Path>,
    ) -> Result<Vec<Target>> {
        match selection {
            AgentSelection::One(name) => {
                let dir = self
                    .coding_agents
                    .get(name)
                    .ok_or_else(|| self.unknown_agent_error(name))?;
                Ok(vec![self.target(name, dir, workspace)?])
            }
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
             Use '{ALL_AGENTS}' to target every configured agent, or add an entry \
             under `coding_agents:` in that file to support a new one.",
            self.path.display(),
            self.configured_agents()
        )
    }

    /// Error for a project-level install that asked for every agent at once.
    pub fn all_agents_in_workspace_error(&self) -> anyhow::Error {
        anyhow::anyhow!(
            "--coding-agent {ALL_AGENTS} is not supported with --workspace; \
             a project-level install targets a single coding agent\n\n\
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
        AgentSelection::One(name.to_string())
    }

    #[test]
    fn resolves_a_known_agent_to_a_single_target() {
        let targets = config(YAML).resolve(&one("kiro")).unwrap();

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].name, "kiro");
        assert_eq!(targets[0].dir, home_dir().unwrap().join(".kiro/skills"));
    }

    #[test]
    fn resolves_every_agent_for_all() {
        let targets = config(YAML).resolve(&AgentSelection::All).unwrap();

        let names: Vec<_> = targets.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["claude", "kiro"]);
    }

    #[test]
    fn all_is_parsed_case_insensitively_and_never_treated_as_an_agent_name() {
        assert_eq!(AgentSelection::from("all"), AgentSelection::All);
        assert_eq!(AgentSelection::from("ALL"), AgentSelection::All);
        assert_eq!(AgentSelection::from("kiro"), one("kiro"));
        assert!(AgentSelection::All.is_all());
        assert!(!one("kiro").is_all());
        assert_eq!(AgentSelection::All.to_string(), "all");
        assert_eq!(one("kiro").to_string(), "kiro");
    }

    #[test]
    fn unknown_agent_error_lists_configured_agents_and_mentions_all() {
        let err = config(YAML).resolve(&one("cursor")).unwrap_err();
        let message = format!("{err}");

        assert!(message.contains("unknown coding agent 'cursor'"), "{message}");
        assert!(message.contains("kiro -> .kiro/skills"), "{message}");
        assert!(message.contains("/tmp/config.yaml"), "{message}");
        assert!(message.contains("Use 'all'"), "{message}");
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
        let target = config(YAML)
            .resolve_one_in_workspace("kiro", Path::new("/work/project"))
            .unwrap();

        assert_eq!(target.dir, PathBuf::from("/work/project/.kiro/skills"));
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
            .resolve_one_in_workspace("cursor", Path::new("/work/project"))
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
