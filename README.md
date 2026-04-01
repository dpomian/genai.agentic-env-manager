# Skill Installer

A Rust-based tool for installing agent skills across multiple AI coding assistants. The skill installer copies skills to a centralized location and creates symlinks in various AI assistant directories, making skills available to tools like Kiro, Windsurf, Copilot, and Claude.

## Installation

### Prerequisites

- Rust (2021 edition or later)
- Cargo package manager

### Build from Source

```bash
git clone <repository-url>
cd skill-installer
cargo build --release
```

The compiled binary will be available at `target/release/skill-installer`.

## Usage

The skill installer provides two main modes of operation:

### Command Line Interface

#### Install a Skill

```bash
skill-installer install --skill /path/to/skill/directory
```

This command:
1. Validates that the skill directory contains a `skill.md` or `SKILL.md` marker file
2. Copies the skill to `~/.agents/skills/<skill-name>`
3. Creates symlinks in configured AI assistant directories

#### Run as MCP Server

```bash
skill-installer serve
```

> **Note**: Use the subcommand `serve`, not `--serve`.

Starts the skill installer as an MCP (Model Context Protocol) server using stdio transport. This mode allows AI assistants to interact with the skill installer programmatically.

### MCP Server Configuration

To use the skill installer as an MCP server with your AI assistant, add the following to your MCP configuration:

```json
{
  "mcpServers": {
    "skill-installer": {
      "command": "./target/release/skill-installer",
      "args": ["serve"],
      "cwd": "/path/to/skill-installer"
    }
  }
}
```

Or with an absolute path to the binary:

```json
{
  "mcpServers": {
    "skill-installer": {
      "command": "/path/to/skill-installer/target/release/skill-installer",
      "args": ["serve"]
    }
  }
}
```

**Configuration file locations:**
- **Claude Desktop**: `~/Library/Application Support/Claude/claude_desktop_config.json` (macOS) or `%APPDATA%\Claude\claude_desktop_config.json` (Windows)
- **Windsurf / Codeium**: `~/.codeium/windsurf/mcp_config.json`
- **Kiro**: `~/.kiro/settings/mcp.json`

> **Note**: Replace `/path/to/skill-installer` with the actual path where you cloned and built the repository.

### MCP Server Tools

When running as an MCP server, the following tools are available:

- **install_skill**: Install a skill from a given absolute path
- **validate_skill**: Validate that a path contains a valid skill

## Configuration

### Skill Requirements

A valid skill directory must contain either:
- `skill.md` - Skill documentation file
- `SKILL.md` - Alternative skill documentation file

### Target Directories

Skills are automatically symlinked to the following directories (if they exist):
- `~/.kiro/skills`
- `~/.codeium/windsurf/skills`
- `~/.copilot/skills`
- `~/.claude/skills`

### Logging

When running as an MCP server, logging is configured via environment variables:
- Set `RUST_LOG` environment variable to control log levels
- Logs are written to stderr with ANSI formatting disabled

## Project Structure

```
skill-installer/
├── src/
│   ├── main.rs          # Application entry point
│   ├── cli.rs           # Command-line interface definitions
│   ├── installer.rs     # Core installation logic
│   ├── skill.rs         # Skill validation functions
│   └── mcp_server.rs    # MCP server implementation
├── Cargo.toml           # Project configuration and dependencies
├── Cargo.lock           # Dependency lock file
└── target/              # Build artifacts (gitignored)
```

### Key Components

- **main.rs**: Handles command parsing and delegates to appropriate modules
- **cli.rs**: Defines the command-line interface using clap
- **installer.rs**: Implements skill copying and symlink creation logic
- **skill.rs**: Validates skill directories for required marker files
- **mcp_server.rs**: Provides MCP server functionality with tool handlers

## Dependencies

- **anyhow**: Error handling and context management
- **clap**: Command-line argument parsing with derive macros
- **dirs**: Cross-platform directory path resolution
- **rmcp**: MCP (Model Context Protocol) server implementation with stdio transport
- **schemars**: JSON schema generation for MCP tool parameters
- **serde**: Serialization and deserialization framework
- **serde_json**: JSON support for serde
- **tokio**: Async runtime with full feature set
- **tracing**: Structured logging framework
- **tracing-subscriber**: Logging subscriber with environment filter support

## Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Ensure code compiles with `cargo build`
5. Run tests with `cargo test` (if tests are present)
6. Submit a pull request

## License

No evidence found in the codebase for license information.