use std::path::PathBuf;

use anyhow::Result;
use clap::{ArgAction, ArgGroup, Parser, Subcommand};

use crate::config::AgentSelection;

#[derive(Parser)]
#[command(
    name = "aem",
    about = "Agentic environment manager: install skills and MCP servers across AI coding agents",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

/// Requires exactly one way of choosing agents, so `install` and `uninstall`
/// can never run without an explicit target.
fn agent_selection_group() -> ArgGroup {
    ArgGroup::new("agent_selection")
        .args(["agents", "all_agents"])
        .required(true)
}

/// The two capability domains — skills and MCP servers — plus running as an MCP
/// server ourselves.
#[derive(Subcommand)]
pub enum Command {
    /// Install, remove, and list agent skills
    #[command(alias = "skills")]
    Skill {
        #[command(subcommand)]
        action: SkillCommand,
    },
    /// Add or remove an MCP server in each coding agent's own config file
    Mcp {
        #[command(subcommand)]
        action: McpCommand,
    },
    /// Run as an MCP server (stdio transport)
    Serve,
}

/// The `skill` operations. Skills are copied to `~/.agents/skills` and symlinked
/// into each agent's directory, as configured in `~/.agents/config.yaml`.
#[derive(Subcommand)]
pub enum SkillCommand {
    /// Install a skill from a local path, a GitHub URL, or a saved source
    #[command(group = agent_selection_group())]
    Install {
        /// Local path, GitHub URL, or <source>:<skill> reference into a saved
        /// source (e.g. ./my-skill,
        /// https://github.com/owner/repo/tree/branch/path/to/skill, or
        /// anthropic:pdf — see `aem skill source`)
        source: String,
        /// Coding agent to install for, as named in ~/.agents/config.yaml
        /// (e.g. kiro). Repeat to install for several agents at once.
        #[arg(
            long = "agent",
            short = 'a',
            alias = "coding-agent",
            action = ArgAction::Append,
            value_name = "NAME"
        )]
        agents: Vec<String>,
        /// Install for every coding agent in ~/.agents/config.yaml, skipping
        /// those whose directory does not exist. Not allowed with --workspace.
        #[arg(long, action = ArgAction::SetTrue)]
        all_agents: bool,
        /// Install at project level: the skill is copied into
        /// <workspace>/<agent dir> and no symlink is created.
        /// Defaults to a user-level install in your home directory.
        #[arg(long, short = 'w', value_name = "PATH")]
        workspace: Option<PathBuf>,
    },
    /// Uninstall a skill by name
    #[command(group = agent_selection_group())]
    Uninstall {
        /// Name of the skill to uninstall, as it appears in `list`. Where the
        /// skill was installed from does not matter.
        name: String,
        /// Coding agent to uninstall from, as named in ~/.agents/config.yaml
        /// (e.g. kiro). Repeat to uninstall from several agents at once. A
        /// skill that is not installed there is a no-op.
        #[arg(
            long = "agent",
            short = 'a',
            alias = "coding-agent",
            action = ArgAction::Append,
            value_name = "NAME"
        )]
        agents: Vec<String>,
        /// Uninstall from every coding agent in ~/.agents/config.yaml.
        #[arg(long, action = ArgAction::SetTrue)]
        all_agents: bool,
        /// Uninstall from this project directory instead of your home directory.
        #[arg(long, short = 'w', value_name = "PATH")]
        workspace: Option<PathBuf>,
    },
    /// List installed skills, broken down per coding agent
    List {
        /// Include frontmatter from skill.md files
        #[arg(long, default_value_t = false)]
        frontmatter: bool,
        /// Only list skills installed for this coding agent, as named in
        /// ~/.agents/config.yaml (e.g. kiro). Repeat to list several agents.
        /// Defaults to every configured agent.
        #[arg(
            long = "agent",
            short = 'a',
            alias = "coding-agent",
            action = ArgAction::Append,
            value_name = "NAME"
        )]
        agents: Vec<String>,
        /// List every coding agent in ~/.agents/config.yaml. This is the
        /// default, so the flag only exists for symmetry with the other commands.
        #[arg(long, action = ArgAction::SetTrue)]
        all_agents: bool,
        /// List skills installed in this project directory instead of your
        /// home directory.
        #[arg(long, short = 'w', value_name = "PATH")]
        workspace: Option<PathBuf>,
    },
    /// Manage saved skill sources: GitHub directories that contain skills
    #[command(alias = "src")]
    Source {
        #[command(subcommand)]
        action: SourceCommand,
    },
}

impl SkillCommand {
    /// The agent selection for this operation, or `None` for `source`, which
    /// does not target agents.
    pub fn agent_selection(&self) -> Result<Option<AgentSelection>> {
        let (agents, all_agents) = match self {
            Self::Install {
                agents, all_agents, ..
            }
            | Self::Uninstall {
                agents, all_agents, ..
            }
            | Self::List {
                agents, all_agents, ..
            } => (agents, all_agents),
            Self::Source { .. } => return Ok(None),
        };

        AgentSelection::parse(agents, *all_agents).map(Some)
    }
}

/// The `mcp` operations. These write to each agent's native MCP config file
/// (`~/.kiro/settings/mcp.json`, `~/.codex/config.toml`, …) rather than to
/// anything this tool owns, so one generic definition is translated per agent.
#[derive(Subcommand)]
pub enum McpCommand {
    /// Install an MCP server into the named agents' config files
    #[command(group = agent_selection_group())]
    Install {
        /// The server definition: inline JSON, a path to a .json file, or `-`
        /// to read stdin. Accepts a `{"mcpServers": {...}}` wrapper, a bare
        /// `{"<name>": {...}}` map, or a single server object with --name.
        definition: String,
        /// Name for the server when the definition is a single unnamed object.
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
        /// Coding agent to install for (e.g. kiro). Repeat for several. See
        /// `aem mcp agents` for the supported names.
        #[arg(
            long = "agent",
            short = 'a',
            action = ArgAction::Append,
            value_name = "NAME"
        )]
        agents: Vec<String>,
        /// Install for every agent that supports MCP configuration.
        #[arg(long, action = ArgAction::SetTrue)]
        all_agents: bool,
        /// Write the project-level config file instead of the user-level one.
        #[arg(long, short = 'w', value_name = "PATH")]
        workspace: Option<PathBuf>,
        /// Print what would be written for each agent without touching any file.
        #[arg(long, action = ArgAction::SetTrue)]
        dry_run: bool,
    },
    /// Remove an MCP server from the named agents' config files
    #[command(group = agent_selection_group())]
    Uninstall {
        /// Name of the server to remove, as it appears in `mcp list`.
        name: String,
        /// Coding agent to remove it from. Repeat for several.
        #[arg(
            long = "agent",
            short = 'a',
            action = ArgAction::Append,
            value_name = "NAME"
        )]
        agents: Vec<String>,
        /// Remove from every agent that supports MCP configuration.
        #[arg(long, action = ArgAction::SetTrue)]
        all_agents: bool,
        /// Remove from the project-level config file instead of the user-level one.
        #[arg(long, short = 'w', value_name = "PATH")]
        workspace: Option<PathBuf>,
    },
    /// List the MCP servers configured for each agent
    List {
        /// Only list this agent. Repeat for several. Defaults to all of them.
        #[arg(
            long = "agent",
            short = 'a',
            action = ArgAction::Append,
            value_name = "NAME"
        )]
        agents: Vec<String>,
        /// List every agent. This is the default.
        #[arg(long, action = ArgAction::SetTrue)]
        all_agents: bool,
        /// Read project-level config files instead of the user-level ones.
        #[arg(long, short = 'w', value_name = "PATH")]
        workspace: Option<PathBuf>,
    },
    /// Show which agents can be configured, and the file each one uses
    Agents,
}

impl McpCommand {
    /// The agent selection for this operation, or `None` for `agents`, which
    /// targets nothing.
    pub fn agent_selection(&self) -> Result<Option<AgentSelection>> {
        let (agents, all_agents) = match self {
            Self::Install {
                agents, all_agents, ..
            }
            | Self::Uninstall {
                agents, all_agents, ..
            }
            | Self::List {
                agents, all_agents, ..
            } => (agents, all_agents),
            Self::Agents => return Ok(None),
        };

        AgentSelection::parse(agents, *all_agents).map(Some)
    }
}

/// The `source` operations. Sources are saved in `~/.agents/sources.yaml`,
/// separately from the hand-edited `config.yaml`.
#[derive(Subcommand)]
pub enum SourceCommand {
    /// Save a GitHub directory of skills under a short name
    Add {
        /// Short name to save the source as (e.g. anthropic)
        name: String,
        /// GitHub directory URL
        /// (e.g. https://github.com/owner/repo/tree/main/skills)
        url: String,
        /// Repoint a name that is already saved instead of failing
        #[arg(long, action = ArgAction::SetTrue)]
        force: bool,
    },
    /// List the saved sources
    #[command(alias = "ls")]
    List,
    /// Forget a saved source
    #[command(alias = "rm")]
    Remove {
        /// Name of the source to remove
        name: String,
    },
    /// Show one saved source and the GitHub location it resolves to
    Show {
        /// Name of the source to show
        name: String,
    },
    /// List the skills available in a source
    #[command(alias = "skills")]
    Browse {
        /// Name of a saved source, or a GitHub directory URL to browse without
        /// saving it first
        name: String,
        /// Include frontmatter from each skill's skill.md
        #[arg(long, default_value_t = false)]
        frontmatter: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        let mut argv = vec!["aem"];
        argv.extend_from_slice(args);
        Cli::try_parse_from(argv)
    }

    /// The selection a successfully parsed command resolves to.
    fn selection(args: &[&str]) -> AgentSelection {
        let Command::Skill { action } = parse(args).expect("arguments should parse").command else {
            panic!("expected a skill command");
        };
        action
            .agent_selection()
            .expect("selection should resolve")
            .expect("command targets agents")
    }

    #[test]
    fn short_agent_flag_selects_one_agent() {
        assert_eq!(
            selection(&["skill", "install", "./my-skill", "-a", "kiro"]),
            AgentSelection::one("kiro")
        );
    }

    #[test]
    fn long_agent_flag_selects_one_agent() {
        assert_eq!(
            selection(&["skill", "install", "./my-skill", "--agent", "kiro"]),
            AgentSelection::one("kiro")
        );
    }

    #[test]
    fn repeating_the_agent_flag_selects_several_agents() {
        assert_eq!(
            selection(&[
                "skill",
                "install",
                "./my-skill",
                "-a",
                "kiro",
                "-a",
                "claude"
            ]),
            AgentSelection::Named(vec!["kiro".to_string(), "claude".to_string()])
        );
    }

    #[test]
    fn all_agents_flag_selects_every_agent() {
        assert_eq!(
            selection(&["skill", "install", "./my-skill", "--all-agents"]),
            AgentSelection::All
        );
    }

    #[test]
    fn legacy_coding_agent_flag_still_works() {
        assert_eq!(
            selection(&["skill", "install", "./my-skill", "--coding-agent", "kiro"]),
            AgentSelection::one("kiro")
        );
        assert_eq!(
            selection(&["skill", "uninstall", "my-skill", "--coding-agent", "all"]),
            AgentSelection::All
        );
    }

    #[test]
    fn install_and_uninstall_require_an_agent_selection() {
        assert!(parse(&["skill", "install", "./my-skill"]).is_err());
        assert!(parse(&["skill", "uninstall", "my-skill"]).is_err());
    }

    #[test]
    fn all_agents_and_named_agents_cannot_be_combined() {
        // clap rejects the two flags together before we ever parse the values.
        let err = parse(&[
            "skill",
            "install",
            "./my-skill",
            "-a",
            "kiro",
            "--all-agents",
        ])
        .err()
        .expect("clap should reject the combination");
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn legacy_all_value_combined_with_a_name_is_rejected() {
        // Both arrive through the same arg, so the conflict surfaces on parse.
        let cli = parse(&["skill", "install", "./my-skill", "-a", "all", "-a", "kiro"]).unwrap();
        let Command::Skill { action } = cli.command else {
            panic!("expected a skill command");
        };
        let err = action.agent_selection().unwrap_err();
        assert!(
            format!("{err}").contains("cannot combine every agent with a named one"),
            "{err}"
        );
    }

    #[test]
    fn list_defaults_to_every_agent() {
        assert_eq!(selection(&["skill", "list"]), AgentSelection::All);
    }

    #[test]
    fn list_can_be_narrowed_to_named_agents() {
        assert_eq!(
            selection(&["skill", "list", "-a", "kiro"]),
            AgentSelection::one("kiro")
        );
    }

    #[test]
    fn workspace_accepts_short_and_long_forms() {
        for args in [
            ["skill", "install", "./my-skill", "-a", "kiro", "-w", "."],
            [
                "skill",
                "install",
                "./my-skill",
                "-a",
                "kiro",
                "--workspace",
                ".",
            ],
        ] {
            let cli = parse(&args).expect("arguments should parse");
            let Command::Skill {
                action: SkillCommand::Install { workspace, .. },
            } = cli.command
            else {
                panic!("expected a skill install command");
            };
            assert_eq!(workspace, Some(PathBuf::from(".")));
        }
    }

    #[test]
    fn the_removed_ws_alias_is_gone() {
        assert!(parse(&["skill", "install", "./my-skill", "-a", "kiro", "--ws", "."]).is_err());
    }

    #[test]
    fn serve_targets_no_agents() {
        // `serve` is its own top-level command and takes no agent selection.
        assert!(matches!(
            parse(&["serve"]).expect("arguments should parse").command,
            Command::Serve
        ));
    }

    const URL: &str = "https://github.com/anthropics/skills/tree/main/skills";

    #[test]
    fn source_add_takes_a_name_and_a_url() {
        let cli =
            parse(&["skill", "source", "add", "anthropic", URL]).expect("arguments should parse");
        let Command::Skill {
            action:
                SkillCommand::Source {
                    action: SourceCommand::Add { name, url, force },
                },
        } = cli.command
        else {
            panic!("expected a source add command");
        };

        assert_eq!(name, "anthropic");
        assert_eq!(url, URL);
        assert!(!force);
    }

    #[test]
    fn source_add_accepts_force() {
        let cli = parse(&["skill", "source", "add", "anthropic", URL, "--force"])
            .expect("arguments should parse");
        let Command::Skill {
            action:
                SkillCommand::Source {
                    action: SourceCommand::Add { force, .. },
                },
        } = cli.command
        else {
            panic!("expected a source add command");
        };

        assert!(force);
    }

    #[test]
    fn source_list_and_remove_have_short_aliases() {
        assert!(matches!(
            parse(&["skill", "source", "ls"]).unwrap().command,
            Command::Skill {
                action: SkillCommand::Source {
                    action: SourceCommand::List
                }
            }
        ));

        let Command::Skill {
            action:
                SkillCommand::Source {
                    action: SourceCommand::Remove { name },
                },
        } = parse(&["skill", "source", "rm", "anthropic"])
            .unwrap()
            .command
        else {
            panic!("expected a source remove command");
        };
        assert_eq!(name, "anthropic");
    }

    #[test]
    fn source_subcommands_require_their_arguments() {
        assert!(parse(&["source"]).is_err());
        assert!(parse(&["skill", "source", "add", "anthropic"]).is_err());
        assert!(parse(&["skill", "source", "remove"]).is_err());
        assert!(parse(&["skill", "source", "show"]).is_err());
    }

    #[test]
    fn source_targets_no_agents() {
        let cli = parse(&["skill", "source", "list"]).expect("arguments should parse");
        let Command::Skill { action } = cli.command else {
            panic!("expected a skill command");
        };
        assert!(action.agent_selection().unwrap().is_none());
    }

    #[test]
    fn source_browse_takes_a_name_and_optional_frontmatter() {
        let Command::Skill {
            action:
                SkillCommand::Source {
                    action: SourceCommand::Browse { name, frontmatter },
                },
        } = parse(&["skill", "source", "browse", "anthropic"])
            .unwrap()
            .command
        else {
            panic!("expected a source browse command");
        };
        assert_eq!(name, "anthropic");
        assert!(!frontmatter);

        let Command::Skill {
            action:
                SkillCommand::Source {
                    action: SourceCommand::Browse { frontmatter, .. },
                },
        } = parse(&["skill", "source", "browse", "anthropic", "--frontmatter"])
            .unwrap()
            .command
        else {
            panic!("expected a source browse command");
        };
        assert!(frontmatter);
    }

    #[test]
    fn source_browse_accepts_a_url_and_has_an_alias() {
        let Command::Skill {
            action:
                SkillCommand::Source {
                    action: SourceCommand::Browse { name, .. },
                },
        } = parse(&["skill", "source", "skills", URL]).unwrap().command
        else {
            panic!("expected a source browse command");
        };
        assert_eq!(name, URL);
    }

    #[test]
    fn source_browse_requires_a_name() {
        assert!(parse(&["skill", "source", "browse"]).is_err());
    }
}
