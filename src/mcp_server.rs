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
    /// Local path, GitHub URL, or "<source>:<skill>" reference into a source
    /// saved in ~/.agents/sources.yaml (e.g. ./my-skill,
    /// https://github.com/owner/repo/tree/branch/path/to/skill, or anthropic:pdf)
    pub source: String,
    /// Coding agents to install for, as named in ~/.agents/config.yaml (e.g.
    /// ["kiro"]). Give one or more names, or set all_agents instead. Required
    /// unless all_agents is true.
    #[serde(default)]
    pub agents: Vec<String>,
    /// Install for every coding agent in ~/.agents/config.yaml, skipping those
    /// whose directory does not exist. Cannot be combined with agents, and not
    /// allowed together with workspace.
    #[serde(default)]
    pub all_agents: bool,
    /// Deprecated: use `agents`. Accepts a single agent name or "all".
    #[serde(default)]
    #[schemars(skip)]
    pub coding_agent: Option<String>,
    /// Optional absolute path to a project directory. When set, the skill is
    /// copied into <workspace>/<agent dir> and no symlink is created.
    /// Omit for a user-level install in the home directory.
    #[serde(default)]
    pub workspace: Option<String>,
}

impl InstallSkillParams {
    fn selection(&self) -> Result<AgentSelection, McpError> {
        selection(&self.agents, self.all_agents, &self.coding_agent)
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ValidateSkillParams {
    /// Absolute path to the skill folder to validate
    #[serde(alias = "skill_path")]
    pub source: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UninstallSkillParams {
    /// Name of the skill to uninstall, as reported by list_skills. Where the
    /// skill was installed from does not matter.
    pub name: String,
    /// Coding agents to uninstall from, as named in ~/.agents/config.yaml (e.g.
    /// ["kiro"]). Give one or more names, or set all_agents instead. Required
    /// unless all_agents is true.
    #[serde(default)]
    pub agents: Vec<String>,
    /// Uninstall from every coding agent in ~/.agents/config.yaml. Cannot be
    /// combined with agents.
    #[serde(default)]
    pub all_agents: bool,
    /// Deprecated: use `agents`. Accepts a single agent name or "all".
    #[serde(default)]
    #[schemars(skip)]
    pub coding_agent: Option<String>,
    /// Optional absolute path to a project directory to uninstall from instead
    /// of the home directory.
    #[serde(default)]
    pub workspace: Option<String>,
}

impl UninstallSkillParams {
    fn selection(&self) -> Result<AgentSelection, McpError> {
        selection(&self.agents, self.all_agents, &self.coding_agent)
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListSkillsParams {
    /// Whether to include frontmatter from skill.md files
    #[serde(default, alias = "include_frontmatter")]
    pub frontmatter: bool,
    /// Optional absolute path to a project directory. When set, project-level
    /// skills are listed per coding agent instead of the home-level ones.
    #[serde(default)]
    pub workspace: Option<String>,
}

/// Builds an agent selection from the tool parameters, folding in the deprecated
/// single-valued `coding_agent`. An empty selection is an error here: unlike the
/// CLI's `list`, every tool that selects agents requires one.
fn selection(
    agents: &[String],
    all_agents: bool,
    coding_agent: &Option<String>,
) -> Result<AgentSelection, McpError> {
    let mut names = agents.to_vec();
    names.extend(coding_agent.clone());

    if names.is_empty() && !all_agents {
        return Err(McpError::invalid_params(
            "no coding agent selected: pass agents (e.g. [\"kiro\"]) or set all_agents".to_string(),
            None,
        ));
    }

    AgentSelection::parse(&names, all_agents)
        .map_err(|e| McpError::invalid_params(format!("{e:#}"), None))
}

#[tool_router]
impl SkillInstallerMcpServer {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Install an agent skill from a local path, a GitHub URL, or a \"<source>:<skill>\" reference into a source saved in ~/.agents/sources.yaml (e.g. anthropic:pdf). By default copies to ~/.agents/skills and creates symlinks in the coding agent directories configured in ~/.agents/config.yaml. Select agents with agents (one or more configured names, e.g. [\"kiro\"]) or all_agents for every configured agent; one of the two is required. Pass workspace to install at project level instead: the skill is copied into <workspace>/<agent dir> with no symlinks, and all_agents is not allowed there."
    )]
    async fn install_skill(
        &self,
        params: Parameters<InstallSkillParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!("Installing skill from source: {}", params.0.source);

        let selection = params.0.selection()?;
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
        let skill_path = PathBuf::from(&params.0.source);

        let canonical_path = skill::validate(&skill_path)
            .map_err(|e| McpError::invalid_params(format!("Validation failed: {}", e), None))?;

        let content = Content::text(format!("Skill at '{}' is valid", canonical_path.display()));
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(
        description = "Uninstall an agent skill from the selected coding agent(s). Select agents with agents (one or more configured names, e.g. [\"kiro\"]) or all_agents; one of the two is required. At user level this removes the agent symlinks and drops the shared ~/.agents/skills copy once no agent links to it. Pass workspace to remove a project-level install from that directory instead. A skill that is not installed for the selection is a no-op, not an error."
    )]
    async fn uninstall_skill(
        &self,
        params: Parameters<UninstallSkillParams>,
    ) -> Result<CallToolResult, McpError> {
        let selection = params.0.selection()?;
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
                    params.0.frontmatter,
                    &AgentSelection::All,
                    Some(workspace),
                )
                .map_err(|e| {
                    McpError::internal_error(format!("Failed to list skills: {}", e), None)
                })?;
                serde_json::to_string_pretty(&agents)
            }
            None => {
                let skills = installer::list_skills(params.0.frontmatter).map_err(|e| {
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
                - install_skill: Install a skill from a local path, GitHub URL, or <source>:<skill> reference \
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn install_params(value: serde_json::Value) -> InstallSkillParams {
        serde_json::from_value(value).expect("params should deserialize")
    }

    #[test]
    fn agents_array_selects_the_named_agents() {
        let params = install_params(json!({
            "source": "./my-skill",
            "agents": ["kiro", "claude"],
        }));

        assert_eq!(
            params.selection().unwrap(),
            AgentSelection::named(["kiro".to_string(), "claude".to_string()])
        );
    }

    #[test]
    fn all_agents_selects_every_agent() {
        let params = install_params(json!({ "source": "./my-skill", "all_agents": true }));

        assert_eq!(params.selection().unwrap(), AgentSelection::All);
    }

    #[test]
    fn deprecated_coding_agent_string_is_still_accepted() {
        let params = install_params(json!({ "source": "./my-skill", "coding_agent": "kiro" }));
        assert_eq!(params.selection().unwrap(), AgentSelection::one("kiro"));

        let params = install_params(json!({ "source": "./my-skill", "coding_agent": "all" }));
        assert_eq!(params.selection().unwrap(), AgentSelection::All);
    }

    #[test]
    fn deprecated_coding_agent_is_hidden_from_the_schema() {
        let schema = serde_json::to_value(schemars::schema_for!(InstallSkillParams)).unwrap();
        let properties = schema["properties"].as_object().unwrap();

        assert!(properties.contains_key("agents"), "{properties:?}");
        assert!(properties.contains_key("all_agents"), "{properties:?}");
        assert!(!properties.contains_key("coding_agent"), "{properties:?}");
    }

    #[test]
    fn an_empty_selection_is_rejected() {
        let params = install_params(json!({ "source": "./my-skill" }));
        let err = params.selection().expect_err("a selection is required");

        assert!(
            format!("{err}").contains("no coding agent selected"),
            "{err}"
        );
    }

    #[test]
    fn uninstall_params_select_agents_the_same_way() {
        let params: UninstallSkillParams =
            serde_json::from_value(json!({ "name": "demo", "agents": ["kiro"] })).unwrap();
        assert_eq!(params.selection().unwrap(), AgentSelection::one("kiro"));

        let params: UninstallSkillParams =
            serde_json::from_value(json!({ "name": "demo", "coding_agent": "all" })).unwrap();
        assert_eq!(params.selection().unwrap(), AgentSelection::All);
    }

    #[test]
    fn validate_params_accept_the_old_skill_path_name() {
        let params: ValidateSkillParams =
            serde_json::from_value(json!({ "skill_path": "./my-skill" })).unwrap();
        assert_eq!(params.source, "./my-skill");

        let params: ValidateSkillParams =
            serde_json::from_value(json!({ "source": "./my-skill" })).unwrap();
        assert_eq!(params.source, "./my-skill");
    }

    #[test]
    fn list_params_accept_the_old_include_frontmatter_name() {
        let params: ListSkillsParams =
            serde_json::from_value(json!({ "include_frontmatter": true })).unwrap();
        assert!(params.frontmatter);

        let params: ListSkillsParams =
            serde_json::from_value(json!({ "frontmatter": true })).unwrap();
        assert!(params.frontmatter);
    }
}
