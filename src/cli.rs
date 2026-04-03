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
    },
    /// Uninstall a skill by name
    Uninstall {
        /// Name of the skill to uninstall
        name: String,
    },
    /// List installed skills
    List {
        /// Include frontmatter from skill.md files
        #[arg(long, default_value_t = false)]
        frontmatter: bool,
    },
    /// Run as an MCP server (stdio transport)
    Serve,
}
