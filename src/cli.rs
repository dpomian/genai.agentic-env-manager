use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "skill-installer", about = "Install agent skills")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Install a skill from a local path or GitHub URL
    Install {
        /// Local path or GitHub URL (e.g., ./my-skill or https://github.com/owner/repo/tree/branch/path/to/skill)
        source: String,
        /// Coding agent to install for, as named in ~/.agents/config.yaml
        /// (e.g. kiro), or "all" for every configured agent. Required, and
        /// cannot be "all" together with --workspace.
        #[arg(long, required = true, value_name = "NAME")]
        coding_agent: String,
        /// Install at project level: the skill is copied into
        /// <workspace>/.<coding-agent>/skills and no symlink is created.
        /// Defaults to a user-level install in your home directory.
        #[arg(long, short = 'w', alias = "ws", value_name = "PATH")]
        workspace: Option<PathBuf>,
    },
    /// Uninstall a skill by name
    Uninstall {
        /// Name of the skill to uninstall
        name: String,
        /// Coding agent to uninstall from, as named in ~/.agents/config.yaml
        /// (e.g. kiro), or "all" for every configured agent. Required. A skill
        /// that is not installed there is a no-op.
        #[arg(long, required = true, value_name = "NAME")]
        coding_agent: String,
        /// Uninstall from this project directory instead of your home directory.
        #[arg(long, short = 'w', alias = "ws", value_name = "PATH")]
        workspace: Option<PathBuf>,
    },
    /// List installed skills, broken down per coding agent
    List {
        /// Include frontmatter from skill.md files
        #[arg(long, default_value_t = false)]
        frontmatter: bool,
        /// Only list skills installed for this coding agent, as named in
        /// ~/.agents/config.yaml (e.g. kiro), or "all" for every configured
        /// agent.
        #[arg(long, default_value = "all", value_name = "NAME")]
        coding_agent: String,
        /// List skills installed in this project directory instead of your
        /// home directory.
        #[arg(long, short = 'w', alias = "ws", value_name = "PATH")]
        workspace: Option<PathBuf>,
    },
    /// Run as an MCP server (stdio transport)
    Serve,
}
