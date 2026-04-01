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

use crate::installer;

#[derive(Clone)]
pub struct SkillInstallerMcpServer {
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct InstallSkillParams {
    /// Local path or GitHub URL (e.g., ./my-skill or https://github.com/owner/repo/tree/branch/path/to/skill)
    pub source: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UninstallSkillParams {
    /// Name of the skill to uninstall
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListSkillsParams {
    /// Whether to include frontmatter from skill.md files
    #[serde(default)]
    pub include_frontmatter: bool,
}

#[tool_router]
impl SkillInstallerMcpServer {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Install an agent skill from a local path or GitHub URL. Copies to ~/.agents/skills and creates symlinks in configured agent directories (.kiro/skills, .codeium/windsurf/skills, .copilot/skills, .claude/skills)"
    )]
    async fn install_skill(
        &self,
        params: Parameters<InstallSkillParams>,
    ) -> Result<CallToolResult, McpError> {
        let skill_name = installer::install_from_source(&params.0.source)
            .await
            .map_err(|e| McpError::internal_error(format!("Installation failed: {}", e), None))?;

        let content = Content::text(format!("Skill '{}' installed successfully", skill_name));
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(
        description = "Uninstall an agent skill by removing it from ~/.agents/skills and removing symlinks from configured agent directories"
    )]
    async fn uninstall_skill(
        &self,
        params: Parameters<UninstallSkillParams>,
    ) -> Result<CallToolResult, McpError> {
        installer::uninstall(&params.0.name)
            .map_err(|e| McpError::internal_error(format!("Uninstall failed: {}", e), None))?;

        let content = Content::text(format!(
            "Skill '{}' uninstalled successfully",
            params.0.name
        ));
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(
        description = "List all installed skills. Optionally include frontmatter from skill.md files."
    )]
    async fn list_skills(
        &self,
        params: Parameters<ListSkillsParams>,
    ) -> Result<CallToolResult, McpError> {
        let skills = installer::list_skills(params.0.include_frontmatter)
            .map_err(|e| McpError::internal_error(format!("Failed to list skills: {}", e), None))?;

        let output = serde_json::to_string_pretty(&skills).map_err(|e| {
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
