use std::path::{Path, PathBuf};

use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, Content, Implementation, InitializeResult, ProtocolVersion, ServerCapabilities,
    ServerInfo,
};
use rmcp::ErrorData as McpError;
use rmcp::{tool, tool_handler, tool_router, ServerHandler};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::config::AgentSelection;
use crate::installer;
use crate::skill;

#[derive(Clone)]
pub struct SkillInstallerMcpServer {
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct InstallSkillParams {
    /// Local path or GitHub URL (e.g., ./my-skill or https://github.com/owner/repo/tree/branch/path/to/skill)
    pub source: String,
    /// Coding agent to install for, as named in ~/.agents/config.yaml (e.g.
    /// kiro), or "all" for every configured agent. Required. "all" cannot be
    /// combined with workspace.
    pub coding_agent: String,
    /// Optional absolute path to a project directory. When set, the skill is
    /// copied into <workspace>/.<coding_agent>/skills and no symlink is created.
    /// Omit for a user-level install in the home directory.
    #[serde(default)]
    pub workspace: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ValidateSkillParams {
    /// Absolute path to the skill folder to validate
    pub skill_path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UninstallSkillParams {
    /// Name of the skill to uninstall
    pub name: String,
    /// Coding agent to uninstall from, as named in ~/.agents/config.yaml (e.g.
    /// kiro), or "all" for every configured agent. Required.
    pub coding_agent: String,
    /// Optional absolute path to a project directory to uninstall from instead
    /// of the home directory.
    #[serde(default)]
    pub workspace: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListSkillsParams {
    /// Whether to include frontmatter from skill.md files
    #[serde(default)]
    pub include_frontmatter: bool,
    /// Optional absolute path to a project directory. When set, project-level
    /// skills are listed per coding agent instead of the home-level ones.
    #[serde(default)]
    pub workspace: Option<String>,
}

#[tool_router]
impl SkillInstallerMcpServer {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Install an agent skill from a local path or GitHub URL. By default copies to ~/.agents/skills and creates symlinks in the coding agent directories configured in ~/.agents/config.yaml. coding_agent is required: pass a configured agent name (e.g. kiro) or \"all\" for every configured agent. Pass workspace to install at project level instead: the skill is copied into <workspace>/.<coding_agent>/skills with no symlinks, and \"all\" is not allowed there."
    )]
    async fn install_skill(
        &self,
        params: Parameters<InstallSkillParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!("Installing skill from source: {}", params.0.source);

        let selection = AgentSelection::from(params.0.coding_agent.as_str());
        let workspace = params.0.workspace.as_deref().map(Path::new);

        let skill_name = installer::install_from_source(&params.0.source, &selection, workspace)
            .await
            .map_err(|e| {
                tracing::error!("Installation failed: {:?}", e);
                McpError::internal_error(format!("Installation failed: {:#}", e), None)
            })?;

        tracing::info!("Skill '{}' installed successfully", skill_name);
        let content = Content::text(format!("Skill '{}' installed successfully", skill_name));
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(
        description = "Validate that a path contains a valid agent skill (checks for skill.md or SKILL.md marker file)"
    )]
    async fn validate_skill(
        &self,
        params: Parameters<ValidateSkillParams>,
    ) -> Result<CallToolResult, McpError> {
        let skill_path = PathBuf::from(&params.0.skill_path);

        let canonical_path = skill::validate(&skill_path)
            .map_err(|e| McpError::invalid_params(format!("Validation failed: {}", e), None))?;

        let content = Content::text(format!("Skill at '{}' is valid", canonical_path.display()));
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(
        description = "Uninstall an agent skill from the selected coding agent(s). coding_agent is required: pass a configured agent name (e.g. kiro) or \"all\". At user level this removes the agent symlinks and drops the shared ~/.agents/skills copy once no agent links to it. Pass workspace to remove a project-level install from that directory instead. A skill that is not installed for the selection is a no-op, not an error."
    )]
    async fn uninstall_skill(
        &self,
        params: Parameters<UninstallSkillParams>,
    ) -> Result<CallToolResult, McpError> {
        let selection = AgentSelection::from(params.0.coding_agent.as_str());
        let workspace = params.0.workspace.as_deref().map(Path::new);

        let report = installer::uninstall(&params.0.name, &selection, workspace)
            .map_err(|e| McpError::internal_error(format!("Uninstall failed: {:#}", e), None))?;

        let message = if report.is_noop() {
            format!(
                "Skill '{}' is not installed for {}; nothing to do",
                params.0.name, selection
            )
        } else if report.removed_from.is_empty() {
            // The shared copy was left behind with no agent linking it.
            format!("Skill '{}' removed from ~/.agents/skills", params.0.name)
        } else {
            format!(
                "Skill '{}' uninstalled from {}",
                params.0.name,
                report.removed_from.join(", ")
            )
        };

        Ok(CallToolResult::success(vec![Content::text(message)]))
    }

    #[tool(
        description = "List all installed skills. Optionally include frontmatter from skill.md files. Pass workspace to list project-level skills per coding agent instead of the home-level ones."
    )]
    async fn list_skills(
        &self,
        params: Parameters<ListSkillsParams>,
    ) -> Result<CallToolResult, McpError> {
        let output = match params.0.workspace.as_deref().map(Path::new) {
            Some(workspace) => {
                let agents = installer::list_skills_by_agent(
                    params.0.include_frontmatter,
                    &AgentSelection::All,
                    Some(workspace),
                )
                .map_err(|e| {
                    McpError::internal_error(format!("Failed to list skills: {}", e), None)
                })?;
                serde_json::to_string_pretty(&agents)
            }
            None => {
                let skills = installer::list_skills(params.0.include_frontmatter).map_err(|e| {
                    McpError::internal_error(format!("Failed to list skills: {}", e), None)
                })?;
                serde_json::to_string_pretty(&skills)
            }
        }
        .map_err(|e| {
            McpError::internal_error(format!("Failed to serialize skills: {}", e), None)
        })?;

        let content = Content::text(output);
        Ok(CallToolResult::success(vec![content]))
    }
}

#[tool_handler]
impl ServerHandler for SkillInstallerMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2025_03_26,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation::from_build_env(),
            instructions: Some(
                "Skill Installer MCP Server. Install agent skills to multiple AI coding assistants. \
                Available tools: \
                - install_skill: Install a skill from a local path or GitHub URL \
                - validate_skill: Validate that a path contains a valid skill \
                - uninstall_skill: Uninstall a skill by name \
                - list_skills: List all installed skills with optional frontmatter"
                    .to_string(),
            ),
        }
    }

    async fn initialize(
        &self,
        _request: rmcp::model::InitializeRequestParams,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<InitializeResult, McpError> {
        Ok(self.get_info())
    }
}
