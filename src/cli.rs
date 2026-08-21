use std::path::PathBuf;

use anyhow::Result;
use clap::{ArgAction, ArgGroup, Parser, Subcommand};

use crate::config::AgentSelection;

#[derive(Parser)]
#[command(name = "skill-installer", about = "Install agent skills")]
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

#[derive(Subcommand)]
pub enum Command {
    /// Install a skill from a local path or GitHub URL
    #[command(group = agent_selection_group())]
    Install {
        /// Local path, GitHub URL, or <source>:<skill> reference into a saved
        /// source (e.g. ./my-skill,
        /// https://github.com/owner/repo/tree/branch/path/to/skill, or
        /// anthropic:pdf — see `skill-installer source`)
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
    /// Run as an MCP server (stdio transport)
    Serve,
    /// Manage saved skill sources: GitHub directories that contain skills
    Source {
        #[command(subcommand)]
        action: SourceCommand,
    },
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
}

impl Command {
    /// The agent selection for this command, or `None` for commands that do not
    /// target agents.
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
            Self::Serve | Self::Source { .. } => return Ok(None),
        };

        AgentSelection::parse(agents, *all_agents).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        let mut argv = vec!["skill-installer"];
        argv.extend_from_slice(args);
        Cli::try_parse_from(argv)
    }

    /// The selection a successfully parsed command resolves to.
    fn selection(args: &[&str]) -> AgentSelection {
        parse(args)
            .expect("arguments should parse")
            .command
            .agent_selection()
            .expect("selection should resolve")
            .expect("command targets agents")
    }

    #[test]
    fn short_agent_flag_selects_one_agent() {
        assert_eq!(
            selection(&["install", "./my-skill", "-a", "kiro"]),
            AgentSelection::one("kiro")
        );
    }

    #[test]
    fn long_agent_flag_selects_one_agent() {
        assert_eq!(
            selection(&["install", "./my-skill", "--agent", "kiro"]),
            AgentSelection::one("kiro")
        );
    }

    #[test]
    fn repeating_the_agent_flag_selects_several_agents() {
        assert_eq!(
            selection(&["install", "./my-skill", "-a", "kiro", "-a", "claude"]),
            AgentSelection::Named(vec!["kiro".to_string(), "claude".to_string()])
        );
    }

    #[test]
    fn all_agents_flag_selects_every_agent() {
        assert_eq!(
            selection(&["install", "./my-skill", "--all-agents"]),
            AgentSelection::All
        );
    }

    #[test]
    fn legacy_coding_agent_flag_still_works() {
        assert_eq!(
            selection(&["install", "./my-skill", "--coding-agent", "kiro"]),
            AgentSelection::one("kiro")
        );
        assert_eq!(
            selection(&["uninstall", "my-skill", "--coding-agent", "all"]),
            AgentSelection::All
        );
    }

    #[test]
    fn install_and_uninstall_require_an_agent_selection() {
        assert!(parse(&["install", "./my-skill"]).is_err());
        assert!(parse(&["uninstall", "my-skill"]).is_err());
    }

    #[test]
    fn all_agents_and_named_agents_cannot_be_combined() {
        // clap rejects the two flags together before we ever parse the values.
        let err = parse(&["install", "./my-skill", "-a", "kiro", "--all-agents"])
            .err()
            .expect("clap should reject the combination");
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn legacy_all_value_combined_with_a_name_is_rejected() {
        // Both arrive through the same arg, so the conflict surfaces on parse.
        let cli = parse(&["install", "./my-skill", "-a", "all", "-a", "kiro"]).unwrap();
        let err = cli.command.agent_selection().unwrap_err();
        assert!(
            format!("{err}").contains("cannot combine every agent with a named one"),
            "{err}"
        );
    }

    #[test]
    fn list_defaults_to_every_agent() {
        assert_eq!(selection(&["list"]), AgentSelection::All);
    }

    #[test]
    fn list_can_be_narrowed_to_named_agents() {
        assert_eq!(
            selection(&["list", "-a", "kiro"]),
            AgentSelection::one("kiro")
        );
    }

    #[test]
    fn workspace_accepts_short_and_long_forms() {
        for args in [
            ["install", "./my-skill", "-a", "kiro", "-w", "."],
            ["install", "./my-skill", "-a", "kiro", "--workspace", "."],
        ] {
            let cli = parse(&args).expect("arguments should parse");
            let Command::Install { workspace, .. } = cli.command else {
                panic!("expected an install command");
            };
            assert_eq!(workspace, Some(PathBuf::from(".")));
        }
    }

    #[test]
    fn the_removed_ws_alias_is_gone() {
        assert!(parse(&["install", "./my-skill", "-a", "kiro", "--ws", "."]).is_err());
    }

    #[test]
    fn serve_targets_no_agents() {
        let cli = parse(&["serve"]).expect("arguments should parse");
        assert!(cli.command.agent_selection().unwrap().is_none());
    }

    const URL: &str = "https://github.com/anthropics/skills/tree/main/skills";

    #[test]
    fn source_add_takes_a_name_and_a_url() {
        let cli = parse(&["source", "add", "anthropic", URL]).expect("arguments should parse");
        let Command::Source {
            action: SourceCommand::Add { name, url, force },
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
        let cli =
            parse(&["source", "add", "anthropic", URL, "--force"]).expect("arguments should parse");
        let Command::Source {
            action: SourceCommand::Add { force, .. },
        } = cli.command
        else {
            panic!("expected a source add command");
        };

        assert!(force);
    }

    #[test]
    fn source_list_and_remove_have_short_aliases() {
        assert!(matches!(
            parse(&["source", "ls"]).unwrap().command,
            Command::Source {
                action: SourceCommand::List
            }
        ));

        let Command::Source {
            action: SourceCommand::Remove { name },
        } = parse(&["source", "rm", "anthropic"]).unwrap().command
        else {
            panic!("expected a source remove command");
        };
        assert_eq!(name, "anthropic");
    }

    #[test]
    fn source_subcommands_require_their_arguments() {
        assert!(parse(&["source"]).is_err());
        assert!(parse(&["source", "add", "anthropic"]).is_err());
        assert!(parse(&["source", "remove"]).is_err());
        assert!(parse(&["source", "show"]).is_err());
    }

    #[test]
    fn source_targets_no_agents() {
        let cli = parse(&["source", "list"]).expect("arguments should parse");
        assert!(cli.command.agent_selection().unwrap().is_none());
    }
}
