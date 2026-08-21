mod cli;
mod config;
pub mod github;
mod installer;
mod mcp_server;
mod skill;
mod sources;

use anyhow::Result;
use clap::Parser;
use rmcp::{transport::stdio, ServiceExt};
use tracing_subscriber::EnvFilter;

use cli::{Cli, Command, SourceCommand};
use config::AgentSelection;
use github::GitHubPath;
use mcp_server::SkillInstallerMcpServer;
use sources::Sources;

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        // `{:#}` keeps the message readable (single line per cause) instead of
        // anyhow's debug rendering.
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let args = Cli::parse();

    // Every agent-targeting command builds its selection the same way, so the
    // `--agent` / `--all-agents` rules live in one place.
    let selection = args.command.agent_selection()?;

    match args.command {
        Command::Install {
            source, workspace, ..
        } => {
            let selection = selection.expect("install targets agents");
            let skill_name =
                installer::install_from_source(&source, &selection, workspace.as_deref()).await?;
            println!("Skill '{}' installed successfully.", skill_name);
        }
        Command::Uninstall {
            name, workspace, ..
        } => {
            let selection = selection.expect("uninstall targets agents");
            let report = installer::uninstall(&name, &selection, workspace.as_deref())?;

            if report.is_noop() {
                match &workspace {
                    Some(ws) => println!(
                        "Skill '{}' is not installed for {} in {}; nothing to do.",
                        name,
                        selection,
                        ws.display()
                    ),
                    None => println!(
                        "Skill '{}' is not installed for {}; nothing to do.",
                        name, selection
                    ),
                }
            } else {
                println!("Skill '{}' uninstalled successfully.", name);
            }
        }
        Command::List {
            frontmatter,
            workspace,
            ..
        } => {
            let selection = selection.expect("list targets agents");
            let agents =
                installer::list_skills_by_agent(frontmatter, &selection, workspace.as_deref())?;
            let any_skills = agents.iter().any(|a| !a.skills.is_empty());
            if !any_skills {
                match (&selection, &workspace) {
                    (AgentSelection::Named(_), Some(ws)) => println!(
                        "No skills installed for coding agent '{}' in {}.",
                        selection,
                        ws.display()
                    ),
                    (AgentSelection::Named(_), None) => {
                        println!("No skills installed for coding agent '{}'.", selection)
                    }
                    (AgentSelection::All, Some(ws)) => {
                        println!("No skills installed in {}.", ws.display())
                    }
                    (AgentSelection::All, None) => println!("No skills installed."),
                }
            } else {
                for agent in &agents {
                    println!("[{}]", agent.agent_name);
                    if agent.skills.is_empty() {
                        println!("  (none)");
                    } else {
                        for skill in &agent.skills {
                            println!("  - {}", skill.name);
                            if let Some(fm) = &skill.frontmatter {
                                println!("    Frontmatter:");
                                for line in fm.lines() {
                                    println!("      {}", line);
                                }
                            }
                        }
                    }
                    println!();
                }
            }
        }
        Command::Source { action } => run_source(action)?,
        Command::Serve => {
            tracing_subscriber::fmt()
                .with_env_filter(
                    EnvFilter::from_default_env().add_directive(tracing::Level::DEBUG.into()),
                )
                .with_writer(std::io::stderr)
                .with_ansi(false)
                .init();

            tracing::info!("Starting Skill Installer MCP Server");

            let service = SkillInstallerMcpServer::new()
                .serve(stdio())
                .await
                .inspect_err(|e| {
                    tracing::error!("serving error: {:?}", e);
                })?;

            service.waiting().await?;
        }
    }

    Ok(())
}

/// Handles the `source` subcommands. Loads `~/.agents/sources.yaml`, applies the
/// change, and writes it back only for the operations that mutate it.
fn run_source(action: SourceCommand) -> Result<()> {
    match action {
        SourceCommand::Add { name, url, force } => {
            let mut sources = Sources::load()?;
            let previous = sources.add(&name, &url, force)?;
            sources.save()?;

            match previous {
                Some(old) => println!("Updated source '{name}':\n  was: {old}\n  now: {url}"),
                None => println!("Added source '{name}' -> {url}"),
            }
            println!("Saved to {}", sources.path().display());
        }
        SourceCommand::List => {
            let sources = Sources::load()?;

            if sources.is_empty() {
                println!("No sources saved.");
                println!(
                    "\nSave one with:\n  skill-installer source add <name> \
                     https://github.com/owner/repo/tree/main/skills"
                );
            } else {
                for (name, url) in sources.iter() {
                    println!("{name} -> {url}");
                }
            }
        }
        SourceCommand::Remove { name } => {
            let mut sources = Sources::load()?;
            let url = sources.remove(&name)?;
            sources.save()?;

            println!("Removed source '{name}' -> {url}");
        }
        SourceCommand::Show { name } => {
            let sources = Sources::load()?;
            let url = sources.get(&name)?;

            println!("{name}");
            println!("  url:    {url}");

            // A saved URL is validated on add, but a hand-edited file can still
            // hold a broken one, so report it instead of failing the command.
            match GitHubPath::parse(url) {
                Ok(github) => {
                    println!("  owner:  {}", github.owner);
                    println!("  repo:   {}", github.repo);
                    println!("  branch: {}", github.branch);
                    println!(
                        "  path:   {}",
                        if github.path.is_empty() {
                            "(repository root)"
                        } else {
                            &github.path
                        }
                    );
                }
                Err(err) => println!("  error:  not a usable GitHub URL: {err:#}"),
            }
        }
    }

    Ok(())
}
