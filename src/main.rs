mod cli;
pub mod github;
mod installer;
mod mcp_server;
mod skill;

use anyhow::Result;
use clap::Parser;
use rmcp::{transport::stdio, ServiceExt};
use tracing_subscriber::EnvFilter;

use cli::{Cli, Command};
use mcp_server::SkillInstallerMcpServer;

#[tokio::main]
async fn main() -> Result<()> {
    let args = Cli::parse();

    match args.command {
        Command::Install { source } => {
            let skill_name = installer::install_from_source(&source).await?;
            println!("Skill '{}' installed successfully.", skill_name);
        }
        Command::Uninstall { name } => {
            installer::uninstall(&name)?;
            println!("Skill uninstalled successfully.");
        }
        Command::List { frontmatter } => {
            let skills = installer::list_skills(frontmatter)?;
            if skills.is_empty() {
                println!("No skills installed.");
            } else {
                for skill in skills {
                    println!("- {}", skill.name);
                    if let Some(fm) = skill.frontmatter {
                        println!("  Frontmatter:");
                        for line in fm.lines() {
                            println!("    {}", line);
                        }
                    }
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
