//! Installing MCP servers into each coding agent's own configuration file.
//!
//! One generic server definition goes in — the shape almost every vendor's docs
//! use:
//!
//! ```json
//! {
//!   "mcpServers": {
//!     "azure-devops": {
//!       "command": "npx",
//!       "args": ["-y", "@dockndevai/mcp-azure-devops"],
//!       "env": { "AZDO_PAT": "${AZDO_PAT}" }
//!     }
//!   }
//! }
//! ```
//!
//! …and each agent gets it written in the dialect it actually reads. The
//! differences are small but breaking: VS Code wants `servers` instead of
//! `mcpServers`, Codex wants TOML, Claude Code rejects a `url` entry with no
//! `type`, Windsurf calls it `serverUrl`, Gemini splits `url` (SSE) from
//! `httpUrl` (streamable HTTP), and every agent spells environment-variable
//! placeholders differently.
//!
//! See `docs/mcp-config-across-agents.md` for the research this encodes.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::{Map, Value};

use crate::config::home_dir;

/// How an agent's config file is serialized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Json,
    Toml,
}

/// The per-agent config dialect. Each variant is a family of quirks rather than
/// a single field, because the vendors differ in more than one place at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// `mcpServers`, transport inferred from `command` vs `url`, no `type`.
    /// Kiro, Amazon Q, Cursor, Claude Desktop.
    McpServersInferred,
    /// `mcpServers` plus a mandatory `type` — Claude Code skips a `url` entry
    /// that has none.
    ClaudeCode,
    /// `servers` (not `mcpServers`) plus a mandatory `type`. VS Code / Copilot Chat.
    VsCode,
    /// `mcpServers`, `type`, and a `tools` allowlist the agent expects. Copilot CLI.
    CopilotCli,
    /// `mcpServers`, but a remote server's URL field is `serverUrl`. Windsurf / Cascade.
    Windsurf,
    /// `mcpServers` plus a `transport` discriminator on remote servers. Devin CLI.
    DevinCli,
    /// `mcpServers` nested in a general settings file; `httpUrl` for streamable
    /// HTTP, `url` for SSE; `includeTools`. Gemini CLI.
    GeminiCli,
    /// TOML `[mcp_servers.<name>]` with snake_case fields. Codex CLI, ChatGPT
    /// desktop, Codex IDE extension.
    Codex,
}

impl Dialect {
    pub fn format(self) -> Format {
        match self {
            Self::Codex => Format::Toml,
            _ => Format::Json,
        }
    }

    /// The top-level key holding the server map, for the JSON dialects.
    pub fn wrapper_key(self) -> &'static str {
        match self {
            Self::VsCode => "servers",
            Self::Codex => "mcp_servers",
            _ => "mcpServers",
        }
    }

    /// How this dialect writes a `${VAR}` placeholder.
    fn placeholder(self, var: &str) -> String {
        match self {
            // As-is.
            Self::McpServersInferred | Self::ClaudeCode => format!("${{{var}}}"),
            // `${env:NAME}`.
            Self::VsCode | Self::Windsurf | Self::DevinCli => format!("${{env:{var}}}"),
            // Bare `$NAME`.
            Self::GeminiCli => format!("${var}"),
            // No interpolation at all. Copilot CLI has none; Codex forwards
            // named variables through `env_vars` / `env_http_headers` instead,
            // which `render_codex` handles, so this is only a fallback.
            Self::CopilotCli | Self::Codex => format!("${{{var}}}"),
        }
    }

    /// True when the dialect cannot expand placeholders at all, so a `${VAR}` in
    /// the definition would reach the server as a literal string.
    fn expands_placeholders(self) -> bool {
        !matches!(self, Self::CopilotCli)
    }

    /// True when the dialect has a per-server tool allowlist to map `tools` onto.
    fn supports_tools(self) -> bool {
        matches!(self, Self::CopilotCli | Self::GeminiCli | Self::Codex)
    }
}

/// One supported agent: where its MCP config lives, and which dialect it reads.
#[derive(Debug, Clone)]
pub struct AgentMcp {
    /// Canonical agent name, as accepted by `--agent`.
    pub name: &'static str,
    /// User-level config file, relative to the home directory.
    pub global: &'static str,
    /// Project-level config file, relative to the workspace root. `None` means
    /// the agent has no documented project-level MCP config.
    pub workspace: Option<&'static str>,
    pub dialect: Dialect,
    /// Shown by `mcp agents`, so the table is self-documenting.
    pub note: &'static str,
}

impl AgentMcp {
    /// Absolute path to the file this agent's servers are written to.
    pub fn config_path(&self, workspace: Option<&Path>) -> Result<PathBuf> {
        match workspace {
            Some(root) => {
                let relative = self.workspace.ok_or_else(|| {
                    anyhow::anyhow!(
                        "agent '{}' has no project-level MCP config; drop --workspace to \
                         configure it at user level in ~/{}",
                        self.name,
                        self.global
                    )
                })?;
                Ok(root.join(relative))
            }
            None => Ok(home_dir()?.join(self.global)),
        }
    }
}

/// Every agent we know how to write MCP config for.
///
/// Paths come from each vendor's documentation; the Kiro, Copilot CLI and
/// Amazon Q entries were additionally verified against a real machine. Adding an
/// agent that reuses an existing dialect is a one-line change here.
const AGENTS: &[AgentMcp] = &[
    AgentMcp {
        name: "kiro",
        global: ".kiro/settings/mcp.json",
        workspace: Some(".kiro/settings/mcp.json"),
        dialect: Dialect::McpServersInferred,
        note: "Kiro IDE and CLI",
    },
    AgentMcp {
        name: "amazonq",
        global: ".aws/amazonq/mcp.json",
        workspace: Some(".amazonq/mcp.json"),
        dialect: Dialect::McpServersInferred,
        note: "Amazon Q Developer CLI",
    },
    AgentMcp {
        name: "claude",
        global: ".claude.json",
        workspace: Some(".mcp.json"),
        dialect: Dialect::ClaudeCode,
        note: "Claude Code (user scope; --workspace writes .mcp.json)",
    },
    AgentMcp {
        name: "claude-desktop",
        global: "Library/Application Support/Claude/claude_desktop_config.json",
        workspace: None,
        dialect: Dialect::McpServersInferred,
        note: "Claude Desktop (macOS path)",
    },
    AgentMcp {
        name: "copilot",
        global: ".copilot/mcp-config.json",
        workspace: Some(".github/mcp.json"),
        dialect: Dialect::CopilotCli,
        note: "GitHub Copilot CLI",
    },
    AgentMcp {
        name: "vscode",
        global: "Library/Application Support/Code/User/mcp.json",
        workspace: Some(".vscode/mcp.json"),
        dialect: Dialect::VsCode,
        note: "VS Code / Copilot Chat (uses the `servers` key)",
    },
    AgentMcp {
        name: "cursor",
        global: ".cursor/mcp.json",
        workspace: Some(".cursor/mcp.json"),
        dialect: Dialect::McpServersInferred,
        note: "Cursor",
    },
    AgentMcp {
        name: "windsurf",
        global: ".codeium/windsurf/mcp_config.json",
        workspace: None,
        dialect: Dialect::Windsurf,
        note: "Windsurf / Devin Desktop legacy Cascade agent",
    },
    AgentMcp {
        name: "devin",
        global: ".config/devin/mcp_config.json",
        workspace: Some(".devin/mcp_config.json"),
        dialect: Dialect::DevinCli,
        note: "Devin CLI / Devin Local agent",
    },
    AgentMcp {
        name: "gemini",
        global: ".gemini/settings.json",
        workspace: Some(".gemini/settings.json"),
        dialect: Dialect::GeminiCli,
        note: "Gemini CLI (mcpServers nested in settings.json)",
    },
    AgentMcp {
        name: "codex",
        global: ".codex/config.toml",
        workspace: Some(".codex/config.toml"),
        dialect: Dialect::Codex,
        note: "Codex CLI / ChatGPT desktop / Codex IDE (TOML)",
    },
];

/// Alternate spellings, so the names people actually type resolve.
const ALIASES: &[(&str, &str)] = &[
    ("claude-code", "claude"),
    ("claudedesktop", "claude-desktop"),
    ("amazon-q", "amazonq"),
    ("q", "amazonq"),
    ("copilot-cli", "copilot"),
    ("code", "vscode"),
    ("copilot-chat", "vscode"),
    ("cascade", "windsurf"),
    ("devin-cli", "devin"),
    ("gemini-cli", "gemini"),
    ("codex-cli", "codex"),
    ("chatgpt", "codex"),
];

/// Looks up one agent by name or alias, case-insensitively.
pub fn agent(name: &str) -> Result<&'static AgentMcp> {
    let wanted = name.trim().to_ascii_lowercase();
    let canonical = ALIASES
        .iter()
        .find(|(alias, _)| *alias == wanted)
        .map(|(_, target)| *target)
        .unwrap_or(&wanted);

    AGENTS
        .iter()
        .find(|a| a.name == canonical)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "unknown agent '{name}' for MCP configuration\n\n\
                 Supported agents:\n{}\n\n\
                 Unlike skills, MCP support is built in rather than read from \
                 config.yaml, because each agent needs its own config dialect.",
                agent_listing()
            )
        })
}

/// Every supported agent, one per line, for `mcp agents` and error messages.
pub fn agent_listing() -> String {
    AGENTS
        .iter()
        .map(|a| format!("  {:<15} {:<45} {}", a.name, a.global, a.note))
        .collect::<Vec<_>>()
        .join("\n")
}

/// All supported agents, for the `--all-agents` selection.
pub fn all_agents() -> &'static [AgentMcp] {
    AGENTS
}

/// Which transport a server uses. Inferred from the definition unless it says so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Stdio,
    /// Streamable HTTP, the recommended remote transport everywhere.
    Http,
    /// Legacy SSE. Deprecated in the spec but still needed by some servers.
    Sse,
}

impl Transport {
    fn as_str(self) -> &'static str {
        match self {
            Self::Stdio => "stdio",
            Self::Http => "http",
            Self::Sse => "sse",
        }
    }
}

/// One MCP server in the generic, agent-independent form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenericServer {
    pub name: String,
    pub transport: Transport,
    /// Executable for a stdio server.
    pub command: Option<String>,
    pub args: Vec<String>,
    /// Environment variables. A value of exactly `${VAR}` is a placeholder and
    /// gets translated per dialect; anything else is a literal.
    pub env: BTreeMap<String, String>,
    /// Endpoint for a remote server.
    pub url: Option<String>,
    pub headers: BTreeMap<String, String>,
    /// Tool allowlist. Only some dialects have somewhere to put this.
    pub tools: Option<Vec<String>>,
    pub disabled: bool,
}

/// Keys that mark an object as a server definition rather than a map of them.
const SERVER_KEYS: &[&str] = &[
    "command", "args", "env", "url", "serverUrl", "httpUrl", "headers", "type", "transport",
    "tools", "disabled",
];

/// Accepts a fragment copied out of the middle of a config file: a bare
/// `"name": { ... }` member, with or without a trailing comma, is wrapped into
/// an object so it parses. Several comma-separated members work too, since
/// wrapping them yields exactly the bare-map form.
fn normalize_definition(raw: &str) -> String {
    let trimmed = raw.trim();

    // A leading quote can only be an object key at this position; a complete
    // definition always starts with '{'.
    if trimmed.starts_with('"') {
        return format!("{{{}}}", trimmed.trim_end().trim_end_matches(','));
    }

    trimmed.to_string()
}

impl GenericServer {
    /// Parses one or more servers out of a generic definition.
    ///
    /// Accepts the `mcpServers` wrapper from the vendor docs, VS Code's
    /// `servers` wrapper, a bare `{"name": {...}}` map, a bare `"name": {...}`
    /// fragment, or a single server object — which needs `name` to supply the key.
    pub fn parse_all(raw: &str, name: Option<&str>) -> Result<Vec<Self>> {
        let normalized = normalize_definition(raw);

        let value: Value = serde_json::from_str(&normalized).context(
            "definition is not valid JSON\n\nExpected one of:\n\
             \x20 {\"mcpServers\": {\"my-server\": {\"command\": \"npx\"}}}\n\
             \x20 {\"my-server\": {\"command\": \"npx\"}}\n\
             \x20 \"my-server\": {\"command\": \"npx\"}\n\
             \x20 {\"command\": \"npx\"}   (with --name my-server)",
        )?;

        let Value::Object(top) = value else {
            bail!("definition must be a JSON object, not an array or scalar");
        };

        // An explicit wrapper is unambiguous, so check it first.
        for key in ["mcpServers", "servers", "mcp_servers"] {
            if let Some(inner) = top.get(key) {
                let inner = inner.as_object().ok_or_else(|| {
                    anyhow::anyhow!("'{key}' must be an object mapping server names to definitions")
                })?;
                return Self::parse_map(inner);
            }
        }

        // No wrapper: either a single server, or a bare map of them. A server
        // object always carries at least one recognised field.
        if top.keys().any(|k| SERVER_KEYS.contains(&k.as_str())) {
            let name = name.ok_or_else(|| {
                anyhow::anyhow!(
                    "this definition is a single server with no name; pass --name <name>, \
                     or wrap it:\n  {{\"mcpServers\": {{\"<name>\": {{ ... }}}}}}"
                )
            })?;
            return Ok(vec![Self::parse_one(name, &top)?]);
        }

        if top.is_empty() {
            bail!("definition contains no servers");
        }

        Self::parse_map(&top)
    }

    fn parse_map(map: &Map<String, Value>) -> Result<Vec<Self>> {
        if map.is_empty() {
            bail!("definition contains no servers");
        }

        map.iter()
            .map(|(name, definition)| {
                let object = definition.as_object().ok_or_else(|| {
                    anyhow::anyhow!("server '{name}' must be a JSON object")
                })?;
                Self::parse_one(name, object)
            })
            .collect()
    }

    fn parse_one(name: &str, object: &Map<String, Value>) -> Result<Self> {
        let name = name.trim();
        if name.is_empty() {
            bail!("server name cannot be empty");
        }

        let command = string_field(object, "command", name)?;
        // Accept every dialect's URL spelling on the way in, so a config copied
        // from any agent's docs works as input.
        let url = first_string_field(object, &["url", "serverUrl", "httpUrl"], name)?;

        let transport = Self::transport(object, name, command.as_deref(), url.as_deref())?;

        let server = Self {
            name: name.to_string(),
            transport,
            command,
            args: string_list(object, "args", name)?.unwrap_or_default(),
            env: string_map(object, "env", name)?,
            url,
            headers: string_map(object, "headers", name)?,
            tools: string_list(object, "tools", name)?,
            disabled: object
                .get("disabled")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        };

        server.validate()?;
        Ok(server)
    }

    /// Resolves the transport from an explicit `type` / `transport` field, or
    /// infers it the way most agents do: `command` means stdio, `url` means HTTP.
    fn transport(
        object: &Map<String, Value>,
        name: &str,
        command: Option<&str>,
        url: Option<&str>,
    ) -> Result<Transport> {
        let declared = first_string_field(object, &["type", "transport"], name)?;

        if let Some(declared) = declared {
            return match declared.to_ascii_lowercase().as_str() {
                // Copilot's "local" and the spec's "stdio" mean the same thing.
                "stdio" | "local" => Ok(Transport::Stdio),
                "http" | "streamable-http" | "streamablehttp" => Ok(Transport::Http),
                "sse" => Ok(Transport::Sse),
                other => bail!(
                    "server '{name}' has unsupported type '{other}'; use \
                     'stdio', 'http', or 'sse'"
                ),
            };
        }

        match (command, url) {
            (Some(_), None) => Ok(Transport::Stdio),
            (None, Some(_)) => Ok(Transport::Http),
            (Some(_), Some(_)) => bail!(
                "server '{name}' has both 'command' and 'url'; a server is either \
                 local (command) or remote (url)"
            ),
            (None, None) => bail!(
                "server '{name}' needs either 'command' (a local stdio server) or \
                 'url' (a remote server)"
            ),
        }
    }

    fn validate(&self) -> Result<()> {
        match self.transport {
            Transport::Stdio if self.command.is_none() => bail!(
                "server '{}' is declared as stdio but has no 'command'",
                self.name
            ),
            Transport::Http | Transport::Sse if self.url.is_none() => bail!(
                "server '{}' is declared as {} but has no 'url'",
                self.name,
                self.transport.as_str()
            ),
            _ => Ok(()),
        }
    }

    fn is_remote(&self) -> bool {
        !matches!(self.transport, Transport::Stdio)
    }
}

/// Reads a required-to-be-a-string field, if present.
fn string_field(object: &Map<String, Value>, key: &str, server: &str) -> Result<Option<String>> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => bail!("server '{server}': '{key}' must be a string"),
    }
}

/// Reads the first of several alternative spellings of the same field, so a
/// definition copied from any agent's documentation is accepted.
fn first_string_field(
    object: &Map<String, Value>,
    keys: &[&str],
    server: &str,
) -> Result<Option<String>> {
    for key in keys {
        if let Some(found) = string_field(object, key, server)? {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

/// Reads an array-of-strings field. Numbers and booleans are stringified, since
/// vendor examples sometimes write ports and flags unquoted.
fn string_list(object: &Map<String, Value>, key: &str, server: &str) -> Result<Option<Vec<String>>> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| scalar_to_string(item, key, server))
            .collect::<Result<Vec<_>>>()
            .map(Some),
        Some(_) => bail!("server '{server}': '{key}' must be an array of strings"),
    }
}

/// Reads a string-valued object such as `env` or `headers`.
fn string_map(
    object: &Map<String, Value>,
    key: &str,
    server: &str,
) -> Result<BTreeMap<String, String>> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(BTreeMap::new()),
        Some(Value::Object(map)) => map
            .iter()
            .map(|(k, v)| Ok((k.clone(), scalar_to_string(v, key, server)?)))
            .collect(),
        Some(_) => bail!("server '{server}': '{key}' must be an object of string values"),
    }
}

fn scalar_to_string(value: &Value, key: &str, server: &str) -> Result<String> {
    match value {
        Value::String(s) => Ok(s.clone()),
        Value::Number(n) => Ok(n.to_string()),
        Value::Bool(b) => Ok(b.to_string()),
        _ => bail!("server '{server}': '{key}' may only contain strings, numbers, or booleans"),
    }
}

/// A value written as exactly `${VAR}` is a placeholder; anything else is a
/// literal. Returns the variable name.
fn placeholder_var(value: &str) -> Option<&str> {
    let inner = value.strip_prefix("${")?.strip_suffix('}')?;
    valid_var_name(inner).then_some(inner)
}

fn valid_var_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !name.starts_with(|c: char| c.is_ascii_digit())
}

/// Every `${VAR}` occurrence in a value, in order. Covers the embedded case
/// (`"Bearer ${TOKEN}"`) as well as a value that is nothing but a placeholder.
fn placeholder_vars(value: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut rest = value;

    while let Some(start) = rest.find("${") {
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else { break };
        let name = &after[..end];
        if valid_var_name(name) {
            found.push(name);
        }
        rest = &after[end + 1..];
    }

    found
}

/// Rewrites every placeholder in a value into the dialect's own syntax, leaving
/// literal text around it untouched. `"Bearer ${TOKEN}"` becomes
/// `"Bearer ${env:TOKEN}"` for Cursor, `"Bearer $TOKEN"` for Gemini, and so on.
fn translate(value: &str, dialect: Dialect) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;

    while let Some(start) = rest.find("${") {
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else { break };
        let name = &after[..end];

        out.push_str(&rest[..start]);
        if valid_var_name(name) {
            out.push_str(&dialect.placeholder(name));
        } else {
            // Not a variable reference (e.g. `${input:key}`); pass it through
            // exactly as written rather than mangling it.
            out.push_str(&rest[start..start + 2 + end + 1]);
        }
        rest = &after[end + 1..];
    }

    out.push_str(rest);
    out
}

/// `Bearer ${VAR}` — the shape Codex's `bearer_token_env_var` expresses directly.
fn bearer_placeholder(header: &str) -> Option<&str> {
    let rest = header.trim().strip_prefix("Bearer ")?;
    placeholder_var(rest.trim())
}

/// Codex has no string interpolation, so a placeholder only survives where it
/// maps onto a dedicated field. Anything else has to be flagged.
fn codex_warnings(server: &GenericServer) -> Vec<String> {
    let mut warnings = Vec::new();

    for (key, env) in &server.env {
        if placeholder_var(env).is_none() && !placeholder_vars(env).is_empty() {
            warnings.push(format!(
                "env '{key}' mixes text with a placeholder, which Codex cannot expand; \
                 it was written literally — set the whole value in your environment and \
                 use \"${{VAR}}\" alone instead"
            ));
        }
    }

    for (key, header) in &server.headers {
        let embedded_only = placeholder_var(header).is_none() && !placeholder_vars(header).is_empty();
        if embedded_only && bearer_placeholder(header).is_none() {
            warnings.push(format!(
                "header '{key}' mixes text with a placeholder, which Codex cannot expand; \
                 it was written literally"
            ));
        }
    }

    warnings
}

/// The result of translating a server for one agent: what to write, plus
/// anything the user should know was changed or dropped.
#[derive(Debug, Clone)]
pub struct Rendered {
    /// The server entry, as JSON. For the Codex dialect this is unused and the
    /// TOML is built directly; see [`render_codex_into`].
    pub json: Value,
    /// Notes worth printing: fields this dialect cannot represent, and
    /// translations that changed the meaning of a value.
    pub warnings: Vec<String>,
}

/// Translates one generic server into an agent's dialect.
pub fn render(server: &GenericServer, dialect: Dialect) -> Rendered {
    let mut entry = Map::new();
    let mut warnings = Vec::new();

    // `type` is mandatory for some dialects and merely harmless for others, so
    // only emit it where the agent actually reads it.
    match dialect {
        Dialect::ClaudeCode | Dialect::VsCode | Dialect::CopilotCli => {
            entry.insert(
                "type".to_string(),
                Value::String(server.transport.as_str().to_string()),
            );
        }
        _ => {}
    }

    if server.is_remote() {
        let url = server
            .url
            .as_deref()
            .expect("a remote server has a url")
            .to_string();
        let url = translate(&url, dialect);

        // The one place the vendors disagree on the field name itself.
        let url_key = match (dialect, server.transport) {
            (Dialect::Windsurf, _) => "serverUrl",
            // Gemini picks the transport by which field you set.
            (Dialect::GeminiCli, Transport::Http) => "httpUrl",
            (Dialect::GeminiCli, _) => "url",
            _ => "url",
        };
        entry.insert(url_key.to_string(), Value::String(url));

        // Devin CLI needs the transport spelled out separately.
        if dialect == Dialect::DevinCli {
            entry.insert(
                "transport".to_string(),
                Value::String(match server.transport {
                    Transport::Sse => "sse",
                    _ => "http",
                }
                .to_string()),
            );
        }

        if !server.headers.is_empty() {
            entry.insert(
                "headers".to_string(),
                translated_map(&server.headers, dialect),
            );
        }
    } else {
        entry.insert(
            "command".to_string(),
            Value::String(translate(
                server.command.as_deref().expect("a stdio server has a command"),
                dialect,
            )),
        );

        if !server.args.is_empty() {
            entry.insert(
                "args".to_string(),
                Value::Array(
                    server
                        .args
                        .iter()
                        .map(|arg| Value::String(translate(arg, dialect)))
                        .collect(),
                ),
            );
        }

        if !server.env.is_empty() {
            entry.insert("env".to_string(), translated_map(&server.env, dialect));
        }
    }

    // Tool allowlists. Copilot CLI expects the field, so default it to "all"
    // rather than leaving the agent to guess.
    match (&server.tools, dialect) {
        (Some(tools), Dialect::CopilotCli) => {
            entry.insert("tools".to_string(), string_array(tools));
        }
        (Some(tools), Dialect::GeminiCli) => {
            entry.insert("includeTools".to_string(), string_array(tools));
        }
        (Some(_), dialect) if !dialect.supports_tools() => {
            warnings.push(format!(
                "'tools' has no equivalent in this agent's config and was dropped; \
                 restrict tools in {}'s own settings instead",
                dialect_label(dialect)
            ));
        }
        (None, Dialect::CopilotCli) => {
            entry.insert("tools".to_string(), string_array(&["*".to_string()]));
        }
        _ => {}
    }

    if server.disabled {
        entry.insert("disabled".to_string(), Value::Bool(true));
        if matches!(dialect, Dialect::VsCode | Dialect::CopilotCli) {
            warnings.push(
                "'disabled' is not a documented field for this agent; disable the \
                 server from its UI or slash command instead"
                    .to_string(),
            );
        }
    }

    warnings.extend(placeholder_warnings(server, dialect));

    if dialect == Dialect::Codex {
        warnings.extend(codex_warnings(server));
    }

    Rendered {
        json: Value::Object(entry),
        warnings,
    }
}

/// Warns when a definition uses `${VAR}` but the target cannot expand it.
fn placeholder_warnings(server: &GenericServer, dialect: Dialect) -> Vec<String> {
    if dialect.expands_placeholders() {
        return Vec::new();
    }

    let mut vars: Vec<&str> = server
        .env
        .values()
        .chain(server.headers.values())
        .flat_map(|value| placeholder_vars(value))
        .collect();
    vars.sort_unstable();
    vars.dedup();

    if vars.is_empty() {
        return Vec::new();
    }

    vec![format!(
        "{} does not expand environment placeholders, so {} will be passed \
         through literally; substitute the real value instead",
        dialect_label(dialect),
        vars.iter()
            .map(|v| format!("${{{v}}}"))
            .collect::<Vec<_>>()
            .join(", ")
    )]
}

fn translated_map(map: &BTreeMap<String, String>, dialect: Dialect) -> Value {
    Value::Object(
        map.iter()
            .map(|(k, v)| (k.clone(), Value::String(translate(v, dialect))))
            .collect(),
    )
}

fn string_array(items: &[String]) -> Value {
    Value::Array(items.iter().map(|s| Value::String(s.clone())).collect())
}

/// Human-readable name for a dialect, for messages.
fn dialect_label(dialect: Dialect) -> &'static str {
    match dialect {
        Dialect::McpServersInferred => "this agent",
        Dialect::ClaudeCode => "Claude Code",
        Dialect::VsCode => "VS Code",
        Dialect::CopilotCli => "Copilot CLI",
        Dialect::Windsurf => "Windsurf",
        Dialect::DevinCli => "Devin CLI",
        Dialect::GeminiCli => "Gemini CLI",
        Dialect::Codex => "Codex",
    }
}

/// What one install or uninstall did to one agent's file.
#[derive(Debug, Clone)]
pub struct Change {
    pub agent: &'static str,
    pub path: PathBuf,
    /// True when an entry of the same name was already there and was replaced.
    pub replaced: bool,
    pub warnings: Vec<String>,
}

/// Adds (or replaces) a server in one agent's config file, creating the file and
/// its parent directories if needed. Every other key in the file is preserved.
pub fn install(
    agent: &'static AgentMcp,
    server: &GenericServer,
    workspace: Option<&Path>,
) -> Result<Change> {
    let path = agent.config_path(workspace)?;
    let rendered = render(server, agent.dialect);

    let replaced = match agent.dialect.format() {
        Format::Json => json_install(&path, agent.dialect, &server.name, rendered.json)?,
        Format::Toml => toml_install(&path, server)?,
    };

    Ok(Change {
        agent: agent.name,
        path,
        replaced,
        warnings: rendered.warnings,
    })
}

/// Removes a server from one agent's config file. `Ok(None)` means it was not
/// configured there, which callers treat as a no-op rather than an error.
pub fn uninstall(
    agent: &'static AgentMcp,
    name: &str,
    workspace: Option<&Path>,
) -> Result<Option<Change>> {
    let path = agent.config_path(workspace)?;

    if !path.exists() {
        return Ok(None);
    }

    let removed = match agent.dialect.format() {
        Format::Json => json_uninstall(&path, agent.dialect, name)?,
        Format::Toml => toml_uninstall(&path, name)?,
    };

    Ok(removed.then(|| Change {
        agent: agent.name,
        path,
        replaced: false,
        warnings: Vec::new(),
    }))
}

/// The server names currently configured for one agent.
pub fn list(agent: &'static AgentMcp, workspace: Option<&Path>) -> Result<Vec<String>> {
    let path = agent.config_path(workspace)?;

    if !path.exists() {
        return Ok(Vec::new());
    }

    match agent.dialect.format() {
        Format::Json => {
            let document = read_json(&path)?;
            Ok(document
                .get(agent.dialect.wrapper_key())
                .and_then(Value::as_object)
                .map(|servers| servers.keys().cloned().collect())
                .unwrap_or_default())
        }
        Format::Toml => {
            let document = read_toml(&path)?;
            Ok(document
                .get("mcp_servers")
                .and_then(|item| item.as_table_like())
                .map(|servers| servers.iter().map(|(key, _)| key.to_string()).collect())
                .unwrap_or_default())
        }
    }
}

// --- JSON dialects -------------------------------------------------------

/// Parses an agent's JSON config, treating a missing file as an empty document.
///
/// Some of these files tolerate comments when the agent reads them (VS Code's
/// `mcp.json` is JSONC), but rewriting the file would silently discard them, so
/// an unparseable file is reported rather than clobbered.
fn read_json(path: &Path) -> Result<Map<String, Value>> {
    if !path.exists() {
        return Ok(Map::new());
    }

    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read '{}'", path.display()))?;

    if raw.trim().is_empty() {
        return Ok(Map::new());
    }

    let value: Value = serde_json::from_str(&raw).with_context(|| {
        format!(
            "failed to parse '{}' as JSON\n\nEdit the file by hand this once — it may \
             contain comments or a trailing comma, which this tool will not rewrite for \
             fear of discarding them",
            path.display()
        )
    })?;

    match value {
        Value::Object(map) => Ok(map),
        _ => bail!(
            "'{}' does not contain a JSON object at its top level",
            path.display()
        ),
    }
}

fn write_json(path: &Path, document: &Map<String, Value>) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create '{}'", parent.display()))?;
    }

    // Two-space indent and a trailing newline, matching what every one of these
    // agents writes itself.
    let mut text = serde_json::to_string_pretty(&Value::Object(document.clone()))
        .context("failed to serialize config")?;
    text.push('\n');

    std::fs::write(path, text).with_context(|| format!("failed to write '{}'", path.display()))
}

/// Returns true when an existing entry of the same name was replaced.
fn json_install(path: &Path, dialect: Dialect, name: &str, entry: Value) -> Result<bool> {
    let mut document = read_json(path)?;
    let wrapper = dialect.wrapper_key();

    // Gemini nests `mcpServers` inside a general settings.json, so the wrapper
    // has to be created without disturbing sibling keys.
    let servers = document
        .entry(wrapper.to_string())
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "'{}' has a '{wrapper}' key that is not an object; fix it by hand and retry",
                path.display()
            )
        })?;

    let replaced = servers.insert(name.to_string(), entry).is_some();
    write_json(path, &document)?;
    Ok(replaced)
}

fn json_uninstall(path: &Path, dialect: Dialect, name: &str) -> Result<bool> {
    let mut document = read_json(path)?;
    let wrapper = dialect.wrapper_key();

    let Some(servers) = document
        .get_mut(wrapper)
        .and_then(Value::as_object_mut)
    else {
        return Ok(false);
    };

    if servers.remove(name).is_none() {
        return Ok(false);
    }

    // Leave an empty wrapper behind rather than deleting it: an empty
    // `mcpServers` is valid everywhere, and removing the key from a shared file
    // like Gemini's settings.json would be a surprising extra edit.
    write_json(path, &document)?;
    Ok(true)
}

// --- Codex's TOML --------------------------------------------------------

fn read_toml(path: &Path) -> Result<toml_edit::DocumentMut> {
    if !path.exists() {
        return Ok(toml_edit::DocumentMut::new());
    }

    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read '{}'", path.display()))?;

    raw.parse::<toml_edit::DocumentMut>()
        .with_context(|| format!("failed to parse '{}' as TOML", path.display()))
}

fn write_toml(path: &Path, document: &toml_edit::DocumentMut) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create '{}'", parent.display()))?;
    }

    std::fs::write(path, document.to_string())
        .with_context(|| format!("failed to write '{}'", path.display()))
}

/// Builds the `[mcp_servers.<name>]` table for a server.
///
/// Codex has no string interpolation, but it does have somewhere better to put a
/// placeholder: `env_vars` forwards named variables from the environment, and
/// `env_http_headers` maps a header to a variable name. So `${AZDO_PAT}` becomes
/// a forwarded variable rather than a literal.
fn codex_table(server: &GenericServer) -> toml_edit::Table {
    use toml_edit::{value, Array, InlineTable, Item, Table};

    let mut table = Table::new();

    if server.is_remote() {
        table["url"] = value(server.url.as_deref().expect("a remote server has a url"));

        let mut literal = InlineTable::new();
        let mut from_env = InlineTable::new();
        for (key, header) in &server.headers {
            // Codex has a dedicated field for the overwhelmingly common
            // "Authorization: Bearer <token>" case.
            if key.eq_ignore_ascii_case("authorization") {
                if let Some(var) = bearer_placeholder(header) {
                    table["bearer_token_env_var"] = value(var);
                    continue;
                }
            }

            match placeholder_var(header) {
                Some(var) => {
                    from_env.insert(key, var.into());
                }
                None => {
                    literal.insert(key, header.as_str().into());
                }
            }
        }
        if !literal.is_empty() {
            table["http_headers"] = Item::Value(literal.into());
        }
        if !from_env.is_empty() {
            table["env_http_headers"] = Item::Value(from_env.into());
        }
    } else {
        table["command"] = value(server.command.as_deref().expect("a stdio server has a command"));

        if !server.args.is_empty() {
            let mut args = Array::new();
            for arg in &server.args {
                args.push(arg.as_str());
            }
            table["args"] = value(args);
        }

        let mut literal = InlineTable::new();
        let mut forwarded = Array::new();
        for (key, env) in &server.env {
            match placeholder_var(env) {
                // Codex reads the variable from its own environment.
                Some(var) => forwarded.push(var),
                None => {
                    literal.insert(key, env.as_str().into());
                }
            }
        }
        if !literal.is_empty() {
            table["env"] = Item::Value(literal.into());
        }
        if !forwarded.is_empty() {
            table["env_vars"] = value(forwarded);
        }
    }

    if let Some(tools) = &server.tools {
        let mut allowed = Array::new();
        for tool in tools {
            allowed.push(tool.as_str());
        }
        table["enabled_tools"] = value(allowed);
    }

    if server.disabled {
        table["enabled"] = value(false);
    }

    table
}

fn toml_install(path: &Path, server: &GenericServer) -> Result<bool> {
    let mut document = read_toml(path)?;

    let servers = document
        .entry("mcp_servers")
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "'{}' has an 'mcp_servers' key that is not a table; fix it by hand and retry",
                path.display()
            )
        })?;

    // Implicit means TOML renders `[mcp_servers.name]` without also emitting a
    // bare `[mcp_servers]` header.
    servers.set_implicit(true);

    let replaced = servers
        .insert(&server.name, toml_edit::Item::Table(codex_table(server)))
        .is_some();

    write_toml(path, &document)?;
    Ok(replaced)
}

fn toml_uninstall(path: &Path, name: &str) -> Result<bool> {
    let mut document = read_toml(path)?;

    let Some(servers) = document
        .get_mut("mcp_servers")
        .and_then(|item| item.as_table_mut())
    else {
        return Ok(false);
    };

    if servers.remove(name).is_none() {
        return Ok(false);
    }

    write_toml(path, &document)?;
    Ok(true)
}

/// Resolves the `definition` argument into JSON text: `-` reads stdin, an
/// existing file is read, and anything else is treated as inline JSON.
pub fn load_definition(argument: &str) -> Result<String> {
    use std::io::Read;

    if argument == "-" {
        let mut buffer = String::new();
        std::io::stdin()
            .read_to_string(&mut buffer)
            .context("failed to read the definition from stdin")?;
        return Ok(buffer);
    }

    // Check the filesystem before guessing, so a file called `{weird}.json`
    // still wins over being read as JSON.
    let path = Path::new(argument);
    if path.is_file() {
        return std::fs::read_to_string(path)
            .with_context(|| format!("failed to read definition '{}'", path.display()));
    }

    // A complete definition starts with '{'; a fragment copied out of a config
    // file starts with its quoted key.
    let inline = argument.trim_start();
    if inline.starts_with('{') || inline.starts_with('"') {
        return Ok(argument.to_string());
    }

    bail!(
        "'{argument}' is neither a file nor inline JSON\n\nPass a path to a .json file, \
         inline JSON, a bare \"<name>\": {{...}} fragment, or '-' to read stdin."
    )
}

/// Renders a server as the TOML that would be added to Codex's config, for
/// `--dry-run`. Building a throwaway document keeps the preview identical to
/// what an install actually writes.
pub fn preview_toml(server: &GenericServer) -> String {
    let mut document = toml_edit::DocumentMut::new();
    let mut servers = toml_edit::Table::new();
    servers.set_implicit(true);
    servers.insert(&server.name, toml_edit::Item::Table(codex_table(server)));
    document.insert("mcp_servers", toml_edit::Item::Table(servers));
    document.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn temp_dir(label: &str) -> PathBuf {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("skill-installer-mcp-{label}-{id}"));
        if dir.exists() {
            std::fs::remove_dir_all(&dir).unwrap();
        }
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The definition from the README, and the shape every vendor's docs use.
    const AZDO: &str = r#"{
      "mcpServers": {
        "azure-devops": {
          "command": "npx",
          "args": ["-y", "@dockndevai/mcp-azure-devops"],
          "env": { "AZDO_ORG_URL": "https://dev.azure.com/org", "AZDO_PAT": "${AZDO_PAT}" }
        }
      }
    }"#;

    fn parse_one(raw: &str) -> GenericServer {
        let mut servers = GenericServer::parse_all(raw, None).expect("definition should parse");
        assert_eq!(servers.len(), 1, "expected exactly one server");
        servers.remove(0)
    }

    /// The rendered entry as a JSON object, for field-level assertions.
    fn entry(server: &GenericServer, dialect: Dialect) -> Map<String, Value> {
        match render(server, dialect).json {
            Value::Object(map) => map,
            other => panic!("expected an object, got {other}"),
        }
    }

    fn field(server: &GenericServer, dialect: Dialect, key: &str) -> String {
        entry(server, dialect)
            .get(key)
            .unwrap_or_else(|| panic!("missing '{key}'"))
            .as_str()
            .unwrap_or_else(|| panic!("'{key}' is not a string"))
            .to_string()
    }

    // --- parsing ---------------------------------------------------------

    #[test]
    fn parses_the_mcp_servers_wrapper() {
        let server = parse_one(AZDO);

        assert_eq!(server.name, "azure-devops");
        assert_eq!(server.transport, Transport::Stdio);
        assert_eq!(server.command.as_deref(), Some("npx"));
        assert_eq!(server.args, vec!["-y", "@dockndevai/mcp-azure-devops"]);
        assert_eq!(server.env["AZDO_PAT"], "${AZDO_PAT}");
        assert!(!server.disabled);
    }

    #[test]
    fn parses_vs_codes_servers_wrapper_too() {
        let server = parse_one(r#"{"servers": {"a": {"command": "x"}}}"#);
        assert_eq!(server.name, "a");
    }

    #[test]
    fn parses_a_bare_map_of_servers() {
        let server = parse_one(r#"{"a": {"command": "x"}}"#);
        assert_eq!(server.name, "a");
    }

    #[test]
    fn parses_a_bare_fragment_copied_out_of_a_config_file() {
        // No enclosing braces: the shape you get by selecting one entry inside
        // an existing mcpServers block.
        let server = parse_one(
            r#""chroma2": {
                 "args": ["chroma-mcp", "--client-type", "persistent"],
                 "command": "uvx",
                 "disabled": true
               }"#,
        );

        assert_eq!(server.name, "chroma2");
        assert_eq!(server.command.as_deref(), Some("uvx"));
        assert_eq!(server.args.len(), 3);
        assert!(server.disabled);
    }

    #[test]
    fn parses_a_fragment_with_the_trailing_comma_left_on() {
        // Copying from the middle of a block brings the separator along.
        let server = parse_one("\"a\": {\"command\": \"x\"},\n");
        assert_eq!(server.name, "a");
    }

    #[test]
    fn parses_several_comma_separated_fragments() {
        let servers =
            GenericServer::parse_all(r#""a": {"command": "x"}, "b": {"command": "y"}"#, None)
                .unwrap();

        let names: Vec<_> = servers.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b"]);
    }

    #[test]
    fn parses_several_servers_at_once() {
        let servers = GenericServer::parse_all(
            r#"{"mcpServers": {"a": {"command": "x"}, "b": {"url": "https://e/mcp"}}}"#,
            None,
        )
        .unwrap();

        let names: Vec<_> = servers.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b"]);
    }

    #[test]
    fn a_single_server_object_needs_a_name() {
        let raw = r#"{"command": "npx", "args": ["-y", "pkg"]}"#;

        let err = GenericServer::parse_all(raw, None).unwrap_err();
        assert!(format!("{err}").contains("--name"), "{err}");

        let server = GenericServer::parse_all(raw, Some("named")).unwrap();
        assert_eq!(server[0].name, "named");
    }

    #[test]
    fn infers_transport_from_command_or_url() {
        assert_eq!(
            parse_one(r#"{"a": {"command": "x"}}"#).transport,
            Transport::Stdio
        );
        assert_eq!(
            parse_one(r#"{"a": {"url": "https://e/mcp"}}"#).transport,
            Transport::Http
        );
    }

    #[test]
    fn honours_an_explicit_type_including_each_dialects_spelling() {
        for (declared, expected) in [
            ("stdio", Transport::Stdio),
            // Copilot's name for stdio.
            ("local", Transport::Stdio),
            ("http", Transport::Http),
            // The name the MCP spec uses.
            ("streamable-http", Transport::Http),
            ("sse", Transport::Sse),
        ] {
            let raw = format!(
                r#"{{"a": {{"type": "{declared}", "command": "x", "url": "https://e/mcp"}}}}"#
            );
            // A declared type resolves the command/url ambiguity, so this parses.
            let server = parse_one(&raw);
            assert_eq!(server.transport, expected, "for type '{declared}'");
        }
    }

    #[test]
    fn accepts_devins_transport_field_as_the_type() {
        assert_eq!(
            parse_one(r#"{"a": {"url": "https://e/mcp", "transport": "sse"}}"#).transport,
            Transport::Sse
        );
    }

    #[test]
    fn accepts_every_dialects_url_spelling_as_input() {
        for key in ["url", "serverUrl", "httpUrl"] {
            let server = parse_one(&format!(r#"{{"a": {{"{key}": "https://e/mcp"}}}}"#));
            assert_eq!(server.url.as_deref(), Some("https://e/mcp"), "for '{key}'");
        }
    }

    #[test]
    fn rejects_a_server_with_neither_command_nor_url() {
        let err = GenericServer::parse_all(r#"{"a": {"env": {}}}"#, None).unwrap_err();
        assert!(format!("{err}").contains("needs either 'command'"), "{err}");
    }

    #[test]
    fn rejects_a_server_with_both_command_and_url() {
        let err =
            GenericServer::parse_all(r#"{"a": {"command": "x", "url": "https://e"}}"#, None)
                .unwrap_err();
        assert!(format!("{err}").contains("has both 'command' and 'url'"), "{err}");
    }

    #[test]
    fn rejects_an_unsupported_type() {
        let err =
            GenericServer::parse_all(r#"{"a": {"type": "ws", "url": "wss://e"}}"#, None)
                .unwrap_err();
        assert!(format!("{err}").contains("unsupported type 'ws'"), "{err}");
    }

    #[test]
    fn rejects_empty_and_malformed_definitions() {
        assert!(GenericServer::parse_all("{}", None).is_err());
        assert!(GenericServer::parse_all("[]", None).is_err());
        assert!(GenericServer::parse_all("not json", None).is_err());
        assert!(GenericServer::parse_all(r#"{"mcpServers": {}}"#, None).is_err());
    }

    #[test]
    fn stringifies_numeric_and_boolean_scalars() {
        // Vendor examples sometimes leave ports and flags unquoted.
        let server = parse_one(r#"{"a": {"command": "x", "args": [8080, true]}}"#);
        assert_eq!(server.args, vec!["8080", "true"]);
    }

    // --- dialect rendering -----------------------------------------------

    #[test]
    fn vs_code_uses_the_servers_wrapper_key() {
        assert_eq!(Dialect::VsCode.wrapper_key(), "servers");
        // Everyone else agrees on mcpServers.
        for dialect in [
            Dialect::McpServersInferred,
            Dialect::ClaudeCode,
            Dialect::CopilotCli,
            Dialect::Windsurf,
            Dialect::DevinCli,
            Dialect::GeminiCli,
        ] {
            assert_eq!(dialect.wrapper_key(), "mcpServers", "for {dialect:?}");
        }
    }

    #[test]
    fn only_the_dialects_that_read_type_get_one() {
        let server = parse_one(AZDO);

        for dialect in [Dialect::ClaudeCode, Dialect::VsCode, Dialect::CopilotCli] {
            assert_eq!(field(&server, dialect, "type"), "stdio", "for {dialect:?}");
        }
        for dialect in [
            Dialect::McpServersInferred,
            Dialect::Windsurf,
            Dialect::DevinCli,
            Dialect::GeminiCli,
        ] {
            assert!(
                !entry(&server, dialect).contains_key("type"),
                "{dialect:?} should not get a type"
            );
        }
    }

    #[test]
    fn claude_code_always_types_a_remote_server() {
        // Claude Code skips a url entry with no type, so this is the one that matters.
        let http = parse_one(r#"{"a": {"url": "https://e/mcp"}}"#);
        assert_eq!(field(&http, Dialect::ClaudeCode, "type"), "http");

        let sse = parse_one(r#"{"a": {"url": "https://e/sse", "type": "sse"}}"#);
        assert_eq!(field(&sse, Dialect::ClaudeCode, "type"), "sse");
    }

    #[test]
    fn windsurf_renames_the_url_field() {
        let server = parse_one(r#"{"a": {"url": "https://e/mcp"}}"#);

        assert_eq!(field(&server, Dialect::Windsurf, "serverUrl"), "https://e/mcp");
        assert!(!entry(&server, Dialect::Windsurf).contains_key("url"));
    }

    #[test]
    fn gemini_picks_the_url_field_by_transport() {
        let http = parse_one(r#"{"a": {"url": "https://e/mcp"}}"#);
        assert_eq!(field(&http, Dialect::GeminiCli, "httpUrl"), "https://e/mcp");
        assert!(!entry(&http, Dialect::GeminiCli).contains_key("url"));

        let sse = parse_one(r#"{"a": {"url": "https://e/sse", "type": "sse"}}"#);
        assert_eq!(field(&sse, Dialect::GeminiCli, "url"), "https://e/sse");
        assert!(!entry(&sse, Dialect::GeminiCli).contains_key("httpUrl"));
    }

    #[test]
    fn devin_spells_out_the_transport() {
        let http = parse_one(r#"{"a": {"url": "https://e/mcp"}}"#);
        assert_eq!(field(&http, Dialect::DevinCli, "transport"), "http");

        let sse = parse_one(r#"{"a": {"url": "https://e/sse", "type": "sse"}}"#);
        assert_eq!(field(&sse, Dialect::DevinCli, "transport"), "sse");

        // A stdio server has no transport field.
        assert!(!entry(&parse_one(AZDO), Dialect::DevinCli).contains_key("transport"));
    }

    #[test]
    fn copilot_gets_a_tools_allowlist_defaulting_to_all() {
        let server = parse_one(AZDO);
        assert_eq!(
            entry(&server, Dialect::CopilotCli)["tools"],
            serde_json::json!(["*"])
        );

        let explicit = parse_one(r#"{"a": {"command": "x", "tools": ["read", "search"]}}"#);
        assert_eq!(
            entry(&explicit, Dialect::CopilotCli)["tools"],
            serde_json::json!(["read", "search"])
        );
    }

    #[test]
    fn tools_map_onto_each_dialects_own_field() {
        let server = parse_one(r#"{"a": {"command": "x", "tools": ["read"]}}"#);

        assert_eq!(
            entry(&server, Dialect::GeminiCli)["includeTools"],
            serde_json::json!(["read"])
        );

        // A dialect with nowhere to put them says so rather than dropping silently.
        let rendered = render(&server, Dialect::McpServersInferred);
        assert!(!rendered.json.as_object().unwrap().contains_key("tools"));
        assert!(
            rendered.warnings.iter().any(|w| w.contains("'tools' has no equivalent")),
            "{:?}",
            rendered.warnings
        );
    }

    // --- placeholder translation -----------------------------------------

    #[test]
    fn translates_a_standalone_placeholder_per_dialect() {
        for (dialect, expected) in [
            (Dialect::McpServersInferred, "${TOKEN}"),
            (Dialect::ClaudeCode, "${TOKEN}"),
            (Dialect::VsCode, "${env:TOKEN}"),
            (Dialect::Windsurf, "${env:TOKEN}"),
            (Dialect::DevinCli, "${env:TOKEN}"),
            (Dialect::GeminiCli, "$TOKEN"),
        ] {
            assert_eq!(translate("${TOKEN}", dialect), expected, "for {dialect:?}");
        }
    }

    #[test]
    fn translates_a_placeholder_embedded_in_text() {
        // The common shape for an auth header.
        assert_eq!(
            translate("Bearer ${TOKEN}", Dialect::Windsurf),
            "Bearer ${env:TOKEN}"
        );
        assert_eq!(
            translate("Bearer ${TOKEN}", Dialect::GeminiCli),
            "Bearer $TOKEN"
        );
        // Several in one value.
        assert_eq!(
            translate("${A}/path/${B}", Dialect::GeminiCli),
            "$A/path/$B"
        );
    }

    #[test]
    fn leaves_literals_and_non_variable_braces_alone() {
        assert_eq!(translate("read-only", Dialect::GeminiCli), "read-only");
        // VS Code's own input syntax must survive untouched.
        assert_eq!(
            translate("${input:api-key}", Dialect::VsCode),
            "${input:api-key}"
        );
        assert_eq!(translate("${}", Dialect::GeminiCli), "${}");
        // An unterminated brace is not a placeholder.
        assert_eq!(translate("${UNCLOSED", Dialect::GeminiCli), "${UNCLOSED");
    }

    #[test]
    fn warns_when_the_target_cannot_expand_placeholders() {
        let server = parse_one(AZDO);
        let rendered = render(&server, Dialect::CopilotCli);

        assert!(
            rendered
                .warnings
                .iter()
                .any(|w| w.contains("does not expand") && w.contains("${AZDO_PAT}")),
            "{:?}",
            rendered.warnings
        );
        // A dialect that does expand them stays quiet.
        assert!(render(&server, Dialect::ClaudeCode).warnings.is_empty());
    }

    // --- Codex's TOML ----------------------------------------------------

    #[test]
    fn codex_forwards_placeholders_as_env_vars() {
        let toml = preview_toml(&parse_one(AZDO));

        // Literal values stay in `env`; the placeholder becomes a forwarded name.
        assert!(toml.contains(r#"env_vars = ["AZDO_PAT"]"#), "{toml}");
        assert!(toml.contains(r#"AZDO_ORG_URL = "https://dev.azure.com/org""#), "{toml}");
        assert!(!toml.contains("${AZDO_PAT}"), "{toml}");
        assert!(toml.contains("[mcp_servers.azure-devops]"), "{toml}");
    }

    #[test]
    fn codex_maps_an_authorization_bearer_header_to_its_own_field() {
        let server = parse_one(
            r#"{"a": {"url": "https://e/mcp", "headers": {"Authorization": "Bearer ${GH_PAT}"}}}"#,
        );
        let toml = preview_toml(&server);

        assert!(toml.contains(r#"bearer_token_env_var = "GH_PAT""#), "{toml}");
        assert!(!toml.contains("${GH_PAT}"), "{toml}");
    }

    #[test]
    fn codex_maps_a_standalone_header_placeholder_to_env_http_headers() {
        let server = parse_one(
            r#"{"a": {"url": "https://e/mcp", "headers": {"X-Key": "${KEY}", "X-Lit": "v"}}}"#,
        );
        let toml = preview_toml(&server);

        assert!(toml.contains(r#"env_http_headers = { X-Key = "KEY" }"#), "{toml}");
        assert!(toml.contains(r#"http_headers = { X-Lit = "v" }"#), "{toml}");
    }

    #[test]
    fn codex_warns_about_a_placeholder_it_cannot_express() {
        let server = parse_one(
            r#"{"a": {"command": "x", "env": {"DSN": "postgres://${USER}@host"}}}"#,
        );
        let warnings = render(&server, Dialect::Codex).warnings;

        assert!(
            warnings.iter().any(|w| w.contains("mixes text with a placeholder")),
            "{warnings:?}"
        );
    }

    #[test]
    fn codex_renders_tools_and_the_disable_switch() {
        let server = parse_one(
            r#"{"a": {"command": "x", "tools": ["open"], "disabled": true}}"#,
        );
        let toml = preview_toml(&server);

        assert!(toml.contains(r#"enabled_tools = ["open"]"#), "{toml}");
        // Codex inverts the flag.
        assert!(toml.contains("enabled = false"), "{toml}");
    }

    // --- file round trips ------------------------------------------------

    #[test]
    fn json_install_creates_the_file_and_reports_a_replacement() {
        let path = temp_dir("json-create").join("nested/mcp.json");
        let server = parse_one(AZDO);
        let dialect = Dialect::McpServersInferred;

        let replaced =
            json_install(&path, dialect, &server.name, render(&server, dialect).json).unwrap();
        assert!(!replaced, "a fresh file has nothing to replace");
        assert!(path.exists(), "parent directories should be created");

        let again =
            json_install(&path, dialect, &server.name, render(&server, dialect).json).unwrap();
        assert!(again, "the second write replaces the first");

        let document = read_json(&path).unwrap();
        assert_eq!(
            document["mcpServers"]
                .as_object()
                .unwrap()
                .keys()
                .collect::<Vec<_>>(),
            vec!["azure-devops"]
        );
    }

    #[test]
    fn json_install_preserves_unrelated_keys() {
        // Gemini keeps mcpServers inside a general settings.json, so nothing
        // else in the file may be disturbed.
        let path = temp_dir("json-preserve").join("settings.json");
        std::fs::write(
            &path,
            r#"{"theme": "dark", "mcp": {"allowed": ["keep-me"]}}"#,
        )
        .unwrap();

        let server = parse_one(AZDO);
        json_install(
            &path,
            Dialect::GeminiCli,
            &server.name,
            render(&server, Dialect::GeminiCli).json,
        )
        .unwrap();

        let document = read_json(&path).unwrap();
        assert_eq!(document["theme"], "dark");
        assert_eq!(document["mcp"]["allowed"], serde_json::json!(["keep-me"]));
        assert!(document["mcpServers"]["azure-devops"].is_object());
    }

    #[test]
    fn json_uninstall_removes_only_the_named_server() {
        let path = temp_dir("json-remove").join("mcp.json");
        std::fs::write(
            &path,
            r#"{"other": 1, "mcpServers": {"a": {"command": "x"}, "b": {"command": "y"}}}"#,
        )
        .unwrap();

        assert!(json_uninstall(&path, Dialect::McpServersInferred, "a").unwrap());

        let document = read_json(&path).unwrap();
        assert_eq!(document["other"], 1);
        let servers = document["mcpServers"].as_object().unwrap();
        assert!(!servers.contains_key("a"));
        assert!(servers.contains_key("b"));
    }

    #[test]
    fn json_uninstall_reports_a_no_op() {
        let path = temp_dir("json-noop").join("mcp.json");
        std::fs::write(&path, r#"{"mcpServers": {"a": {"command": "x"}}}"#).unwrap();

        assert!(!json_uninstall(&path, Dialect::McpServersInferred, "missing").unwrap());
        // Wrong wrapper key for this file, so still a no-op rather than an error.
        assert!(!json_uninstall(&path, Dialect::VsCode, "a").unwrap());
    }

    #[test]
    fn an_unparseable_json_file_is_reported_not_clobbered() {
        let path = temp_dir("json-bad").join("mcp.json");
        // JSONC, which VS Code tolerates but serde_json does not.
        let original = "{\n  // a comment\n  \"servers\": {}\n}";
        std::fs::write(&path, original).unwrap();

        let err = read_json(&path).unwrap_err();
        assert!(format!("{err:#}").contains("may contain comments"), "{err:#}");
        // The file is untouched.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn toml_install_preserves_comments_and_other_tables() {
        let path = temp_dir("toml-preserve").join("config.toml");
        std::fs::write(
            &path,
            "# keep this comment\nmodel = \"gpt-5.6\"\n\n[mcp_servers.existing]\ncommand = \"true\"\n",
        )
        .unwrap();

        let server = parse_one(AZDO);
        assert!(!toml_install(&path, &server).unwrap());

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# keep this comment"), "{text}");
        assert!(text.contains("model = \"gpt-5.6\""), "{text}");
        assert!(text.contains("[mcp_servers.existing]"), "{text}");
        assert!(text.contains("[mcp_servers.azure-devops]"), "{text}");
        // No stray `[mcp_servers]` header of its own.
        assert!(!text.contains("\n[mcp_servers]\n"), "{text}");

        // Replacing reports it, and removing leaves the rest intact.
        assert!(toml_install(&path, &server).unwrap());
        assert!(toml_uninstall(&path, "azure-devops").unwrap());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# keep this comment"), "{text}");
        assert!(text.contains("[mcp_servers.existing]"), "{text}");
        assert!(!text.contains("azure-devops"), "{text}");

        assert!(!toml_uninstall(&path, "azure-devops").unwrap(), "now a no-op");
    }

    // --- the agent table -------------------------------------------------

    #[test]
    fn agents_resolve_by_name_and_alias() {
        assert_eq!(agent("kiro").unwrap().name, "kiro");
        assert_eq!(agent("claude-code").unwrap().name, "claude");
        assert_eq!(agent("amazon-q").unwrap().name, "amazonq");
        assert_eq!(agent("chatgpt").unwrap().name, "codex");
        // Case and surrounding whitespace do not matter.
        assert_eq!(agent("  KIRO ").unwrap().name, "kiro");
    }

    #[test]
    fn an_unknown_agent_lists_the_supported_ones() {
        let err = agent("nope").unwrap_err();
        let message = format!("{err}");

        assert!(message.contains("unknown agent 'nope'"), "{message}");
        assert!(message.contains("kiro"), "{message}");
        assert!(message.contains("codex"), "{message}");
    }

    #[test]
    fn every_agent_has_a_usable_table_entry() {
        for agent in all_agents() {
            assert!(!agent.global.is_empty(), "{} has no global path", agent.name);
            assert!(
                !agent.global.starts_with('/') && !agent.global.starts_with('~'),
                "{} must be home-relative",
                agent.name
            );
            if let Some(workspace) = agent.workspace {
                assert!(
                    !workspace.starts_with('/') && !workspace.starts_with('~'),
                    "{} workspace path must be project-relative",
                    agent.name
                );
            }
            // The alias table must not shadow a canonical name.
            assert!(
                !ALIASES.iter().any(|(alias, _)| *alias == agent.name),
                "{} is both a name and an alias",
                agent.name
            );
        }
    }

    #[test]
    fn workspace_paths_are_rooted_at_the_project() {
        let kiro = agent("kiro").unwrap();
        assert_eq!(
            kiro.config_path(Some(Path::new("/work/project"))).unwrap(),
            PathBuf::from("/work/project/.kiro/settings/mcp.json")
        );

        // Claude Code writes .mcp.json at the project root instead of ~/.claude.json.
        let claude = agent("claude").unwrap();
        assert_eq!(
            claude.config_path(Some(Path::new("/work/project"))).unwrap(),
            PathBuf::from("/work/project/.mcp.json")
        );
    }

    #[test]
    fn an_agent_without_a_project_config_says_so() {
        let err = agent("windsurf")
            .unwrap()
            .config_path(Some(Path::new("/work/project")))
            .unwrap_err();

        assert!(
            format!("{err}").contains("no project-level MCP config"),
            "{err}"
        );
    }

    // --- definition loading ----------------------------------------------

    #[test]
    fn loads_a_definition_from_inline_json_or_a_file() {
        assert_eq!(load_definition("{\"a\": 1}").unwrap(), "{\"a\": 1}");
        // A fragment is inline input too.
        let fragment = r#""a": {"command": "x"}"#;
        assert_eq!(load_definition(fragment).unwrap(), fragment);

        let path = temp_dir("load").join("server.json");
        std::fs::write(&path, AZDO).unwrap();
        assert_eq!(
            load_definition(path.to_str().unwrap()).unwrap(),
            AZDO.to_string()
        );
    }

    #[test]
    fn rejects_a_definition_that_is_neither_a_file_nor_json() {
        let err = load_definition("./does-not-exist.json").unwrap_err();
        assert!(
            format!("{err}").contains("neither a file nor inline JSON"),
            "{err}"
        );
    }
}
