mod cli;
mod config;
pub mod github;
mod installer;
mod mcp_server;
mod skill;

use anyhow::Result;
use clap::Parser;
use rmcp::{transport::stdio, ServiceExt};
use tracing_subscriber::EnvFilter;

use cli::{Cli, Command};
use config::AgentSelection;
use mcp_server::SkillInstallerMcpServer;

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
