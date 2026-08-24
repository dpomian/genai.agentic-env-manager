# MCP Server Configuration Across Coding Agents

Consolidated reference for **where** MCP server configuration lives, **how** it is loaded, and **what format** it takes, for the major coding agents.

Researched 2026-08-21 from vendor documentation (links at the bottom). Local file paths for Kiro, Copilot CLI, and Amazon Q were additionally verified on this machine.

---

## TL;DR — the 90% that is the same

Almost every agent uses a JSON file containing a map of server name → server definition:

```json
{
  "mcpServers": {
    "my-stdio-server": {
      "command": "npx",
      "args": ["-y", "@example/mcp-server"],
      "env": { "API_KEY": "${API_KEY}" }
    },
    "my-remote-server": {
      "url": "https://mcp.example.com/mcp",
      "headers": { "Authorization": "Bearer ${TOKEN}" }
    }
  }
}
```

Shared conventions:

- Two server flavours: **local/stdio** (`command` + `args` + `env`) and **remote** (`url` + `headers`).
- Streamable HTTP is the recommended remote transport; SSE is supported but deprecated everywhere.
- OAuth with Dynamic Client Registration (DCR) is handled automatically; a `clientId`/`clientSecret` escape hatch exists for providers that don't support DCR (Figma, GitHub, Linear, Slack).
- Two scopes: **user/global** (home directory) and **project/workspace** (repo directory), with project overriding user.
- A per-server disable switch and some form of per-tool allow/deny list.

## The 10% that bites you

| Difference | Who is the odd one out |
| --- | --- |
| Top-level wrapper key | **VS Code / Copilot Chat** uses `servers`, not `mcpServers` |
| File format | **Codex / ChatGPT desktop** uses TOML (`[mcp_servers.<name>]`), not JSON |
| Remote URL field name | **Windsurf / Devin Desktop (Cascade)** prefers `serverUrl`; **Gemini CLI** distinguishes `url` (SSE) from `httpUrl` (streamable HTTP) |
| `type` required? | **Claude Code** errors if a `url` entry has no `type`. **VS Code** and **Copilot CLI/coding agent** require `type`. **Kiro**, **Cursor**, **Windsurf**, **Devin CLI**, **Gemini** infer it from `url` vs `command` |
| `tools` field | **GitHub Copilot** (CLI + coding agent) requires/expects a `tools` array; nobody else has this exact field |
| No file at all | **Devin cloud** is configured only through the web UI (org admin permission) |
| Config not a file | **Copilot coding agent / code review** is a JSON blob pasted into repository Settings on GitHub.com |
| Env interpolation syntax | Six mutually incompatible dialects — see [Variable interpolation](#variable-interpolation) |

---

## File locations

| Agent | User / global scope | Project / workspace scope | Format |
| --- | --- | --- | --- |
| Kiro IDE & CLI | `~/.kiro/settings/mcp.json` | `.kiro/settings/mcp.json` (+ `mcpServers` inside agent JSON in `~/.kiro/agents/*.json`) | JSON, `mcpServers` |
| Amazon Q Developer CLI (Kiro predecessor) | `~/.aws/amazonq/mcp.json` | `.amazonq/mcp.json` | JSON, `mcpServers` |
| Claude Code | `~/.claude.json` (user scope, and local scope under `projects["<path>"].mcpServers`) | `.mcp.json` at repo root | JSON, `mcpServers` |
| Claude Desktop | macOS `~/Library/Application Support/Claude/claude_desktop_config.json`; Windows `%APPDATA%\Claude\claude_desktop_config.json` | — | JSON, `mcpServers` |
| VS Code / GitHub Copilot Chat | user profile `mcp.json` (command: *MCP: Open User Configuration*) | `.vscode/mcp.json` | JSON, **`servers`** |
| VS Code Agent Host (the harness behind agent mode) | `~/.copilot/mcp-config.json` | `.mcp.json` at workspace root | JSON, `mcpServers` |
| GitHub Copilot CLI | `~/.copilot/mcp-config.json` | `.mcp.json` (any dir from cwd up to repo root) and `.github/mcp.json` | JSON, `mcpServers` or bare top-level map |
| GitHub Copilot coding agent / code review | — | Repo **Settings → Copilot → MCP servers** (pasted JSON, not a file) | JSON, `mcpServers` |
| Cursor | `~/.cursor/mcp.json` | `.cursor/mcp.json` | JSON, `mcpServers` |
| Windsurf / Devin Desktop — legacy Cascade agent | `~/.codeium/windsurf/mcp_config.json` | — | JSON, `mcpServers` |
| Devin CLI (Devin Local agent) | `~/.config/devin/mcp_config.json` (Windows `%APPDATA%\devin\mcp_config.json`) | `.devin/mcp_config.json` (shared) + `.devin/mcp_config.local.json` (gitignored, default target) | JSON, `mcpServers` |
| Devin cloud | Settings → Connections → MCP servers (web UI only) | — | web form |
| Gemini CLI | `~/.gemini/settings.json` → `mcpServers` key | `.gemini/settings.json` | JSON, `mcpServers` nested in a general settings file |
| Codex CLI / ChatGPT desktop / Codex IDE extension | `~/.codex/config.toml` | `.codex/config.toml` (trusted projects only) | **TOML**, `[mcp_servers.<name>]` |

Notes:

- Kiro, Copilot CLI, and Amazon Q paths above were confirmed to exist on this machine.
- Codex shares one config across the CLI, the ChatGPT desktop app, and the IDE extension — configure once, use everywhere.
- Copilot CLI **deliberately does not read `.vscode/mcp.json`**, because of the `servers` vs `mcpServers` key mismatch.

---

## Per-agent detail

### Kiro (IDE, CLI, Web)

```json
{
  "mcpServers": {
    "fetch": {
      "command": "uvx",
      "args": ["mcp-server-fetch"],
      "env": { "BRAVE_API_KEY": "${BRAVE_API_KEY}" },
      "disabled": false,
      "autoApprove": ["read_file"],
      "disabledTools": ["delete_file"]
    },
    "api": {
      "url": "https://api.example.com/mcp",
      "headers": { "Authorization": "Bearer ${API_TOKEN}" },
      "oauth": {
        "clientId": "my-client-id",
        "clientSecret": "secret (CLI only)",
        "redirectUri": "http://localhost:7778/oauth/callback",
        "oauthScopes": ["files:read"]
      }
    }
  }
}
```

- Distinctive fields: `autoApprove` (accepts `"*"`), `disabledTools`.
- Load precedence: agent config `mcpServers` > workspace `mcp.json` > global `mcp.json`. Whole entries override; fields are not merged.
- Hot-reloads on save; only changed servers restart.
- IDE-only guardrail: env vars must be added to the **Mcp Approved Env Vars** setting before `${VAR}` expands. The CLI just reads your shell env.
- Client secrets work in the CLI (confidential client) but not in the IDE (public/PKCE only).
- CLI has `kiro-cli mcp add --name … --scope global --command … --args … --env …` and `--agent <name>` for agent-scoped servers.

### Claude Code

```json
{
  "mcpServers": {
    "shared-server": {
      "type": "http",
      "url": "${API_BASE_URL:-https://api.example.com}/mcp",
      "headers": { "Authorization": "Bearer ${API_KEY}" },
      "timeout": 600000,
      "alwaysLoad": true,
      "oauth": { "clientId": "…", "callbackPort": 8080, "scopes": "channels:read chat:write" }
    }
  }
}
```

- Three scopes: `local` (default, per-project, private, in `~/.claude.json`), `project` (`.mcp.json`, committed), `user` (all projects, in `~/.claude.json`). Precedence: local > project > user > plugin servers > claude.ai connectors.
- **`type` is mandatory for URL entries.** Accepted: `http` (alias `streamable-http`), `sse`, `ws`. An entry with a `url` and no `type` is read as stdio and skipped with an error.
- Server names are restricted to letters, numbers, hyphens, underscores. Reserved names: `workspace`, `claude-in-chrome`, `computer-use`, `Claude Preview`, `Claude Browser`.
- Unusual extras: `timeout` (ms, per-server tool timeout), `alwaysLoad` (exempt from tool-search deferral), `headersHelper` (shell command emitting a JSON header object, for Kerberos/SSO), `oauth.authServerMetadataUrl`, `oauth.scopes` (single space-separated string, unlike everyone else's array).
- Project-scoped servers require interactive approval; approvals interact with workspace trust.
- CLI: `claude mcp add`, `claude mcp add-json`, `claude mcp add-from-claude-desktop`, `claude mcp login/logout`.

### VS Code / GitHub Copilot Chat

```json
{
  "inputs": [
    { "type": "promptString", "id": "api-key", "description": "API Key", "password": true }
  ],
  "servers": {
    "memory": {
      "type": "stdio",
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-memory"],
      "cwd": "${workspaceFolder}",
      "envFile": "${workspaceFolder}/.env",
      "env": { "API_KEY": "${input:api-key}" },
      "sandboxEnabled": true,
      "dev": { "watch": "src/**/*.ts" }
    },
    "context7": { "type": "http", "url": "https://mcp.context7.com/mcp" }
  },
  "sandbox": {
    "filesystem": { "allowWrite": ["${workspaceFolder}"], "denyRead": ["${userHome}/.ssh"] },
    "network": { "allowedDomains": ["api.example.com", "*.cdn.example.com"] }
  }
}
```

- **`servers`, not `mcpServers`.** This is the single most common copy-paste failure in the ecosystem.
- Three top-level sections: `servers`, `inputs`, `sandbox`.
- `inputs` is unique to VS Code: `promptString` / `pickString` / `command`, referenced as `${input:id}`, prompted once and stored securely.
- `sandbox` + per-server `sandboxEnabled` restricts filesystem/network (macOS and Linux only). Sandboxed servers auto-approve tool calls.
- `dev.watch` / `dev.debug` for MCP server development.
- Supports Unix sockets and Windows named pipes as `url`: `unix:///path/server.sock`, `pipe:///pipe/name`, with `#/subpath` fragments.
- Recommended naming: camelCase, no whitespace or special characters.
- Caveat from the docs: VS Code forwards servers to the Agent Host, but the Agent Host does not itself read `.vscode/mcp.json`. For portable config use workspace `.mcp.json` or `~/.copilot/mcp-config.json`. Servers needing `${input:...}` are not forwarded.

### GitHub Copilot CLI

```json
{
  "mcpServers": {
    "playwright": {
      "type": "local",
      "command": "npx",
      "args": ["@playwright/mcp@latest"],
      "env": {},
      "tools": ["*"]
    },
    "context7": {
      "type": "http",
      "url": "https://mcp.context7.com/mcp",
      "headers": { "CONTEXT7_API_KEY": "YOUR-API-KEY" },
      "tools": ["*"]
    }
  }
}
```

- `type`: `local` and `stdio` are synonyms (`stdio` is the portable choice); plus `http` and `sse`.
- `tools`: `["*"]` or an explicit list. Defaults to `*`.
- Project files may use `mcpServers` **or** a bare top-level map (`{ "playwright": { … } }`).
- Resolution: walks cwd → repo root loading each `.mcp.json` / `.github/mcp.json`; `.mcp.json` beats `.github/mcp.json` in the same directory; closer directories win; project beats `~/.copilot/mcp-config.json`.
- Project-level servers load only after folder trust is confirmed. In `copilot -p` prompt mode, untrusted dirs skip them unless `GITHUB_COPILOT_PROMPT_MODE_WORKSPACE_MCP=true`.
- The GitHub MCP server is built in; no configuration needed.
- CLI: `copilot mcp add [--transport http|sse|stdio] [--env K=V] [--header "H: V"] [--tools …] [--timeout MS]`, plus `/mcp add|show|edit|delete|enable|disable|search`.
- Only `PATH` is inherited from your environment; everything else must be listed in `env`.

### GitHub Copilot coding agent & code review (repository-level)

Configured at **repo Settings → Copilot → MCP servers**, not in a file. Shared by the cloud coding agent and Copilot code review.

```json
{
  "mcpServers": {
    "sentry": {
      "type": "local",
      "command": "npx",
      "args": ["@sentry/mcp-server@latest", "--host=$SENTRY_HOST"],
      "env": {
        "SENTRY_HOST": "https://contoso.sentry.io",
        "SENTRY_ACCESS_TOKEN": "$COPILOT_MCP_SENTRY_ACCESS_TOKEN"
      },
      "tools": ["*"]
    }
  }
}
```

- `tools` and `type` are the required keys; `type` accepts `local`, `stdio`, `http`, `sse`.
- Secrets must be GitHub Agents secrets/variables whose names start with `COPILOT_MCP_`. Substitution: `$VAR`, `${VAR}`, `${VAR:-default}` — supported in every string and string-array field except `tools` and `type`.
- **No OAuth support for remote servers.** Tools only — resources and prompts are ignored.
- No approval prompt exists: whatever you list in `tools` the agent may call autonomously. Allowlist read-only tools.
- GitHub and Playwright MCP servers are on by default. The built-in GitHub server uses a read-only, current-repo-scoped token; widen it by configuring `https://api.githubcopilot.com/mcp/` with an `X-MCP-Toolsets` header and a `COPILOT_MCP_GITHUB_PERSONAL_ACCESS_TOKEN` secret.
- Porting from `.vscode/mcp.json`: add `tools`, and replace `inputs` / `envFile` with `env`.

### Cursor

```json
{
  "mcpServers": {
    "local-server": {
      "type": "stdio",
      "command": "python",
      "args": ["${workspaceFolder}/tools/mcp_server.py"],
      "env": { "API_KEY": "${env:API_KEY}" },
      "envFile": ".env"
    },
    "oauth-server": {
      "url": "https://api.example.com/mcp",
      "auth": {
        "CLIENT_ID": "${env:MCP_CLIENT_ID}",
        "CLIENT_SECRET": "${env:MCP_CLIENT_SECRET}",
        "scopes": ["read", "write"]
      }
    }
  }
}
```

- Static OAuth block is named `auth` with **SCREAMING_SNAKE_CASE keys** (`CLIENT_ID`, `CLIENT_SECRET`) plus lowercase `scopes` — unlike everyone else's `oauth: { clientId }`.
- Fixed OAuth redirect URLs to register with providers: `https://www.cursor.com/agents/mcp/oauth/callback` (web/Cloud Agents) and `http://localhost:8787/callback` (desktop).
- `envFile` is stdio-only.
- Interpolation is resolved only in `command`, `args`, `env`, `url`, `headers` (and `auth`).
- Also supports programmatic registration from an extension: `vscode.cursor.mcp.registerServer()`.
- Enterprise: dashboard allowlist by command pattern / URL pattern, per-server network modes (allow all / allowlist / deny all / no sandbox).

### Windsurf / Devin Desktop — legacy Cascade agent

```json
{
  "mcpServers": {
    "github": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-github"],
      "env": { "GITHUB_PERSONAL_ACCESS_TOKEN": "${file:~/.secrets/gh_token}" }
    },
    "remote-http-mcp": {
      "serverUrl": "https://your-server/mcp",
      "headers": { "API_KEY": "Bearer ${env:AUTH_TOKEN}" }
    }
  }
}
```

- Remote servers use **`serverUrl`** (`url` is also accepted).
- Interpolation in `command`, `args`, `env`, `serverUrl`, `url`, `headers`: `${env:VAR}` and `${file:/path/to/file}` (tilde paths allowed; file contents are trimmed).
- Cascade caps total exposed tools at 100; per-server tool toggles live in the UI, and `disabledTools` in the file.
- One-click install deeplink: `windsurf://windsurf-mcp-registry?serverName=<name>`.
- Team admin allowlist matches Server ID plus regex-anchored `command`/`args` (arg array length must match exactly); `env` is never regex-matched. Allowlisting one server blocks all others.
- **Important:** this file applies to the legacy Cascade agent only. The default Devin Local agent in new tabs uses the Devin CLI config below.

### Devin CLI (Devin Local agent)

```json
{
  "mcpServers": {
    "github": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-github"],
      "env": { "GITHUB_TOKEN": "ghp_…" },
      "disabled": false
    },
    "notion": {
      "url": "https://mcp.notion.com/mcp",
      "transport": "http",
      "oauthClientId": "Iv1.abc123",
      "oauthClientSecret": "${env:MY_MCP_CLIENT_SECRET}",
      "oauthResource": ""
    }
  }
}
```

- Transport discriminator is **`transport`**: `"http"` (default for URL servers, with automatic SSE fallback on 4xx) or `"sse"`.
- Flat OAuth fields: `oauthClientId`, `oauthClientSecret`, `oauthResource` (RFC 8707 `resource`; set to `""` to omit entirely for providers that reject it).
- Three scopes with `local` as the default write target: `.devin/mcp_config.local.json` (gitignored) → `.devin/mcp_config.json` (shared) → `~/.config/devin/mcp_config.json`.
- Historical gotcha: before v3000.3 servers lived under `mcpServers` in the main `config.json` files; they are auto-migrated to the dedicated `mcp_config.json` files on startup.
- Permissions use Claude-Code-style matchers: `mcp__server__tool`, `mcp__server__*`, `mcp__*` under `permissions.allow/deny/ask`.
- CLI: `devin mcp add|list|get|remove|login|logout|enable|disable`, with `-s local|project|user`.

### Devin cloud

- No config file. Org admins use **Settings → Connections → MCP servers → Add a custom MCP** (requires the *Manage MCP Servers* permission).
- Transport values are uppercase in the documented JSON shape: `STDIO`, `SSE`, `HTTP`. Env vars key is `env_variables`, auth selector is `auth_method` (`none` / `auth_header` / `oauth`).
- OAuth redirect URI to allowlist with your provider: `https://api.devin.ai/mcp/oauth/callback`.
- OAuth connections have an **Access** setting: `Organization` (one shared identity — use a service account) or `Personal` (per-member).
- **Test listing tools** validates the server in an isolated environment before use.

### Gemini CLI

MCP config is nested inside the general settings file, not a dedicated file.

```json
{
  "mcp": { "allowed": ["my-trusted-server"], "excluded": ["experimental-server"] },
  "mcpServers": {
    "pythonTools": {
      "command": "python",
      "args": ["-m", "my_mcp_server"],
      "cwd": "./mcp-servers/python",
      "env": { "API_KEY": "$EXTERNAL_API_KEY", "TEMP_DIR": "%TEMP%" },
      "timeout": 15000,
      "trust": false,
      "includeTools": ["safe_tool"],
      "excludeTools": ["dangerous_tool"]
    },
    "httpServer": { "httpUrl": "http://localhost:3000/mcp" },
    "sseServer": { "url": "https://api.example.com/sse" }
  }
}
```

- **Transport is chosen by field name**: `httpUrl` → streamable HTTP, `url` → SSE, `command` → stdio.
- `trust: true` bypasses all tool confirmations for that server.
- `includeTools` / `excludeTools`, with `excludeTools` winning. When merging with extension-provided config, exclusions union and inclusions intersect.
- Google-specific auth: `authProviderType` of `dynamic_discovery` (default), `google_credentials` (ADC), or `service_account_impersonation` (with `targetAudience` + `targetServiceAccount`, for IAP-protected Cloud Run).
- Security: the CLI **redacts** sensitive host env vars (`*TOKEN*`, `*SECRET*`, `*PASSWORD*`, `*KEY*`, `*AUTH*`, `*CREDENTIAL*`, certificates) from spawned servers unless you list them explicitly in `env`.
- **Do not put underscores in server names** — the policy parser splits fully-qualified names on the first underscore after `mcp_` and will silently misapply security rules.
- Side files: OAuth tokens in `~/.gemini/mcp-oauth-tokens.json`, enable/disable state in `~/.gemini/mcp-server-enablement.json`.
- CLI: `gemini mcp add|list|remove|enable|disable`, `-s user|project` (default `project`).

### Codex CLI / ChatGPT desktop / Codex IDE extension

TOML, shared across all three surfaces.

```toml
[mcp_servers.context7]
command = "npx"
args = ["-y", "@upstash/context7-mcp"]
env_vars = ["LOCAL_TOKEN", { name = "REMOTE_TOKEN", source = "remote" }]

[mcp_servers.context7.env]
MY_ENV_VAR = "MY_ENV_VALUE"

[mcp_servers.figma]
url = "https://mcp.figma.com/mcp"
bearer_token_env_var = "FIGMA_OAUTH_TOKEN"
http_headers = { "X-Figma-Region" = "us-east-1" }

[mcp_servers.chrome_devtools]
url = "http://localhost:3000/mcp"
enabled_tools = ["open", "screenshot"]
disabled_tools = ["screenshot"]          # applied after enabled_tools
default_tools_approval_mode = "prompt"   # auto | prompt | writes | approve
startup_timeout_sec = 20
tool_timeout_sec = 45
enabled = true

[mcp_servers.chrome_devtools.tools.open]
approval_mode = "approve"

# top-level OAuth callback overrides
mcp_oauth_callback_port = 5555
mcp_oauth_callback_url = "https://devbox.example.internal/callback"
```

- Only snake_case format in this list. Field names differ correspondingly: `http_headers`, `env_http_headers`, `bearer_token_env_var`, `startup_timeout_sec` (default 10), `tool_timeout_sec` (default 60).
- `env_vars` forwards named variables from the local (or remote executor) environment, distinct from `env` which sets literal values.
- `auth` selects `"oauth"` (default, stored MCP OAuth creds) or `"chatgpt"` (reuse the ChatGPT session, first-party origin only).
- `required = true` makes startup fail if the server can't initialize.
- Approval granularity is per tool: `default_tools_approval_mode` plus `tools.<tool>.approval_mode`.
- Supports OAuth Client ID Metadata Documents (CIMD) in addition to DCR; `codex mcp login --oauth-client-registration cimd|dcr|auto`.
- Project-scoped `.codex/config.toml` is honoured only in trusted projects.
- CLI: `codex mcp add <name> --env K=V -- <command>`, `codex mcp list`, `codex mcp login <name>`.

---

## Variable interpolation

The largest source of non-portability. Each dialect below is mutually exclusive.

| Agent | Syntax | Where it applies |
| --- | --- | --- |
| Kiro | `${VAR}` | `env`, `headers`, and other string fields; IDE requires the var to be pre-approved |
| Claude Code | `${VAR}`, `${VAR:-default}` | `command`, `args`, `env`, `url`, `headers` |
| VS Code / Copilot Chat | `${input:id}`, `${workspaceFolder}`, `${userHome}`, and other VS Code predefined variables | server config + `sandbox` paths |
| Copilot coding agent | `$VAR`, `${VAR}`, `${VAR:-default}` — names must start with `COPILOT_MCP_` | all string / string[] fields except `tools`, `type` |
| Copilot CLI | none documented; only `PATH` is inherited | — |
| Cursor | `${env:NAME}`, `${userHome}`, `${workspaceFolder}`, `${workspaceFolderBasename}`, `${pathSeparator}`, `${/}` | `command`, `args`, `env`, `url`, `headers`, `auth` |
| Windsurf / Cascade | `${env:VAR}`, `${file:/path}` | `command`, `args`, `env`, `serverUrl`, `url`, `headers` |
| Devin CLI | `${env:VAR}`, `${file:/path}` | incl. the `oauth*` fields |
| Gemini CLI | `$VAR`, `${VAR}`, `%VAR%` (Windows) | `env` |
| Codex | no string interpolation — use `env_vars`, `bearer_token_env_var`, `env_http_headers` | — |

Claude Code also injects `CLAUDE_PROJECT_DIR` into every spawned stdio server's environment, and plugin-provided servers get `${CLAUDE_PLUGIN_ROOT}` / `${CLAUDE_PLUGIN_DATA}`.

## Tool filtering and approval

| Agent | Allow list | Deny list | Auto-approve | Disable server |
| --- | --- | --- | --- | --- |
| Kiro | — | `disabledTools` | `autoApprove` (`"*"` allowed) | `disabled: true` |
| Claude Code | permission rules `mcp__server__tool` | `disabledMcpjsonServers`, `deniedMcpServers` | permission `allow` rules | `/mcp` toggle → `disabledMcpServers` in `~/.claude.json` |
| VS Code | `chat.mcp.access` setting | — | implicit when `sandboxEnabled` | UI / *MCP: List Servers* |
| Copilot CLI | `tools: [...]` | `tools` omission | — | `/mcp disable NAME` |
| Copilot coding agent | `tools` (required) | — | always auto (no prompts exist) | remove from config |
| Cursor | dashboard allowlist (enterprise) | — | Run Modes | UI toggle |
| Windsurf | admin allowlist | `disabledTools` | — | UI toggle |
| Devin CLI | `permissions.allow` | `permissions.deny` | `permissions.ask` for forced prompts | `disabled: true` / `devin mcp disable` |
| Gemini CLI | `includeTools`, `mcp.allowed` | `excludeTools`, `mcp.excluded` | `trust: true` | `gemini mcp disable` |
| Codex | `enabled_tools` | `disabled_tools` | `default_tools_approval_mode = "auto"` | `enabled = false` |

## Practical porting notes

- **Broadest-compatibility stdio entry**: `mcpServers` wrapper + `"type": "stdio"` + `command` + `args` + `env` with literal values. Accepted by Kiro, Claude Code, Copilot CLI, Cursor, Windsurf, Devin CLI, Gemini CLI, Amazon Q, Claude Desktop. Needs the key renamed to `servers` for VS Code, and rewriting to TOML for Codex.
- **Broadest-compatibility remote entry**: `"type": "http"` + `url` + `headers`. Add `serverUrl` for Windsurf, `httpUrl` for Gemini, `transport: "http"` for Devin CLI. Never omit `type` if Claude Code is a target.
- Docker stdio servers must run in the foreground — never pass `-d`.
- Don't write to stdout from a stdio server; log to stderr or you break the JSON-RPC framing.
- OAuth sessions are **per client**. Authenticating Notion in Claude Code does not authenticate it in Devin CLI, Codex, or Kiro; run each client's `mcp login`.
- Redirect URIs to register when a provider requires pre-registration: Devin cloud `https://api.devin.ai/mcp/oauth/callback`; Cursor `https://www.cursor.com/agents/mcp/oauth/callback` and `http://localhost:8787/callback`; Claude Code / Kiro / Codex / Gemini all use loopback with a configurable port.
- Never commit credentials. Prefer, in order: OAuth, then env-var interpolation, then gitignored local-scope files (`.devin/mcp_config.local.json`, `.claude/settings.local.json`), then platform secret stores (`COPILOT_MCP_*` Agents secrets, Devin Secrets).
- Copilot's coding agent has **no approval prompt**, and Gemini's `trust: true` and Kiro's `autoApprove: ["*"]` remove it. Treat those as granting the server unattended execution rights.

## Sources

- Kiro — https://kiro.dev/docs/mcp/configuration/
- Claude Code — https://docs.claude.com/en/docs/claude-code/mcp
- VS Code / Copilot Chat — https://code.visualstudio.com/docs/copilot/reference/mcp-configuration
- GitHub Copilot CLI — https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/add-mcp-servers
- GitHub Copilot coding agent / code review — https://docs.github.com/en/copilot/how-tos/copilot-on-github/customize-copilot/configure-mcp-servers
- Cursor — https://cursor.com/docs/mcp
- Windsurf / Devin Desktop Cascade — https://docs.windsurf.com/windsurf/cascade/mcp
- Devin CLI — https://docs.devin.ai/cli/extensibility/mcp/configuration
- Devin cloud — https://docs.devin.ai/work-with-devin/mcp
- Gemini CLI — https://github.com/google-gemini/gemini-cli/blob/main/docs/tools/mcp-server.md
- Codex — https://developers.openai.com/codex/extend/mcp and https://developers.openai.com/codex/config-file/config-reference
