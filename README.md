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
skill-installer install /path/to/skill/directory --agent kiro
```

This command:
1. Validates that the skill directory contains a `skill.md` or `SKILL.md` marker file
2. Resolves the target coding agent(s) from `~/.agents/config.yaml`
3. Copies the skill to `~/.agents/skills/<skill-name>`
4. Creates symlinks in the resolved coding agent directories

The positional argument may be a local path, a GitHub `/tree/` URL, or a
`<source>:<skill>` reference into a saved source — see
[Install from a Saved Source](#install-from-a-saved-source).

You must say which agents to install for. Either name them with `--agent`
(short `-a`), repeating the flag for more than one:

```bash
skill-installer install /path/to/skill -a kiro
skill-installer install /path/to/skill -a kiro -a claude
```

…or ask for every configured agent with `--all-agents`:

```bash
skill-installer install /path/to/skill --all-agents
```

`--all-agents` skips agents whose directory does not exist, so you don't get
directories for tools you don't use. An agent you name explicitly gets its
directory created if it is missing. The two forms cannot be combined, and
omitting both is a usage error — nothing is installed either way. A name that is
not in `config.yaml` prints the configured agents and exits with a non-zero
status without installing anything.

> `--agent all` is still accepted as a synonym for `--all-agents`, and
> `--coding-agent` still works as an alias for `--agent`, so existing scripts
> keep running. Both are deprecated and hidden from `--help`.

#### Install at Project Level

Pass `--workspace` (short `-w`) to install into a project instead of your home
directory:

```bash
skill-installer install /path/to/skill -a kiro -w .
```

The path is canonicalized, so relative paths like `.` work. The skill is copied
directly to `<workspace>/.kiro/skills/<skill-name>` — nothing is written to
`~/.agents/skills` and **no symlinks are created**, so the project directory is
self-contained and can be committed or shared.

The subdirectory comes from the same `config.yaml` mapping used for user-level
installs, so `windsurf: .codeium/windsurf/skills` becomes
`<workspace>/.codeium/windsurf/skills`. An absolute config entry has no
project-level equivalent and falls back to `<workspace>/.<agent>/skills`. An
agent whose project layout differs from its home layout can configure a separate
`workspace` path — see [Per-scope paths](#per-scope-paths).

You can name several agents for a project-level install, and each gets its own
self-contained copy:

```bash
skill-installer install /path/to/skill -a kiro -a claude -w .
```

`--all-agents` is **not** allowed with `--workspace`, because a project should
only gain directories for the tools it actually uses. Passing it is an error and
nothing is written.

`--workspace` also works with `uninstall` and `list`:

```bash
skill-installer list --workspace .
skill-installer uninstall my-skill -a kiro -w .
```

Project-level and user-level installs are independent: `uninstall --workspace`
never touches `~/.agents/skills`, and `uninstall` without it never touches a
project.

#### Uninstall a Skill

`uninstall` selects agents exactly like `install`: name them with `--agent`, or
pass `--all-agents`.

```bash
skill-installer uninstall my-skill -a kiro
skill-installer uninstall my-skill -a kiro -a claude
skill-installer uninstall my-skill --all-agents
```

Uninstalling a skill that was never installed, or that is not installed for the
selected agents, is a **no-op**: it reports that there was nothing to do and
exits zero. An agent that is not in `config.yaml` is still an error.

At user level the agent symlinks are removed first, and the shared copy in
`~/.agents/skills` is removed only once no configured agent links to it any
more — so uninstalling for one agent cannot leave another with a dangling
symlink. A copy that nothing points at is reclaimed on the next uninstall.
Unlike `install`, `--all-agents` *is* allowed with `--workspace`, since
uninstalling only removes what is already there and creates no directories.

#### Manage Skill Sources

A *source* is a GitHub directory that contains skills, saved under a short name
so you don't have to retype the URL:

```bash
skill-installer source add anthropic https://github.com/anthropics/skills/tree/main/skills
skill-installer source list
skill-installer source show anthropic
skill-installer source browse anthropic
skill-installer source remove anthropic
```

`list` and `remove` also answer to `ls` and `rm`.

Sources live in `~/.agents/sources.yaml`, separate from `config.yaml`:

```yaml
sources:
  anthropic: https://github.com/anthropics/skills/tree/main/skills
```

The URL must be a GitHub `/tree/` directory URL — it is parsed when you add it,
so a typo is caught immediately rather than at install time. The path may be
empty (`.../tree/main`), which means the repository root. `show` prints the
owner, repo, branch, and path the URL resolves to.

Adding a name that already exists is an error, so a mistyped `add` cannot
silently repoint a source you rely on. Pass `--force` to repoint it deliberately:

```bash
skill-installer source add anthropic https://github.com/anthropics/skills/tree/main --force
```

Set `SKILL_INSTALLER_SOURCES` to keep the file somewhere else.

#### Browse the Skills in a Source

`source browse` lists what a source actually contains, so you can see the names
before installing one:

```bash
skill-installer source browse anthropic
skill-installer source browse anthropic --frontmatter
```

Every immediate subdirectory holding a `skill.md` or `SKILL.md` counts as a
skill. `--frontmatter` additionally fetches each marker file and prints its
frontmatter block, in the same shape as `list --frontmatter`:

```
anthropic -> https://github.com/anthropics/skills/tree/main/skills

  - pdf
    Frontmatter:
      name: pdf
      description: ...

19 skills found. Install one with `skill-installer install anthropic:<name> --agent <agent>`.
```

A GitHub URL can be browsed without saving it first, which is useful for deciding
whether a source is worth keeping:

```bash
skill-installer source browse https://github.com/anthropics/skills/tree/main/skills
```

Subdirectories with no skill marker are reported as skipped rather than silently
dropped, so a source pointing one level too high is obvious. A source that points
at a single skill instead of a directory of them says so. `browse` also answers to
`skills`.

Listing costs one GitHub API request per subdirectory, and two with
`--frontmatter`; up to 8 run at a time.

#### Install from a Saved Source

Once a source is saved, `install` accepts `<source>:<skill>` in place of a URL:

```bash
skill-installer source add anthropic https://github.com/anthropics/skills/tree/main/skills
skill-installer install anthropic:pdf -a kiro
```

The skill name is appended to the source's directory, so `anthropic:pdf` fetches
`https://github.com/anthropics/skills/tree/main/skills/pdf`. The resolved URL is
printed before the download. A nested skill works too
(`anthropic:document/pdf`), and a source pointing at a repository root appends
directly to it.

Everything else about the install is unchanged — `--agent`, `--all-agents` and
`--workspace` behave exactly as they do for a URL or a local path.

The three forms are told apart like this:

- anything starting with `http://` or `https://` is a URL, so the `:` in
  `https://` is never read as a separator;
- otherwise a `:` means a source reference — unless a file or directory with
  that literal name exists, in which case the path wins;
- everything else is a local path.

A reference into a source that is not saved is an error that lists the saved
sources, and nothing is installed. The skill part must stay inside the source, so
`..` segments, absolute paths and empty segments are refused.

**Uninstall does not take a reference.** A source only says where a skill was
fetched from; nothing about it is recorded once the skill is installed. Uninstall
by name, as shown above:

```bash
skill-installer install anthropic:pdf -a kiro
skill-installer uninstall pdf -a kiro
```

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

- **install_skill**: Install a skill from a local path, a GitHub URL, or a
  `<source>:<skill>` reference into a saved source. Select agents with `agents`
  (an array of configured names) or `all_agents: true`; one of the two is
  required. Optional `workspace` mirrors the CLI flag for a project-level
  install.
- **validate_skill**: Validate that `source` contains a valid skill
- **uninstall_skill**: Uninstall a skill by `name` only — references are not
  accepted, since the source is not recorded. Selects agents like `install_skill`
  (`agents` or `all_agents`); optional `workspace` for a project-level uninstall.
  Not being installed is a no-op.
- **list_skills**: List installed skills, with optional `frontmatter`. With
  `workspace` set, returns the project-level skills broken down per coding agent.

The older parameter names still deserialize — `coding_agent` (a single name or
`"all"`), `skill_path`, and `include_frontmatter` — so existing callers keep
working, but they are absent from the advertised tool schemas.

## Configuration

### Skill Requirements

A valid skill directory must contain either:
- `skill.md` - Skill documentation file
- `SKILL.md` - Alternative skill documentation file

### Coding Agents (`~/.agents/config.yaml`)

The mapping from an `--agent` value to the directory that receives the skill
lives in `~/.agents/config.yaml`. The file is created with these defaults the
first time you install a skill:

```yaml
coding_agents:
  kiro: .kiro/skills
  windsurf: .codeium/windsurf/skills
  copilot: .copilot/skills
  claude: .claude/skills
```

A single path is used for both scopes: resolved against your home directory for
a user-level install, and against the project root for a `--workspace` install.
It may also start with `~/`, or be absolute. To support a new IDE, add an entry —
no rebuild required:

```yaml
coding_agents:
  cursor: .cursor/skills
```

#### Per-scope paths

An agent whose user-level and project-level directories differ can spell them
out instead of giving one path:

```yaml
coding_agents:
  windsurf:
    global: .codeium/windsurf/skills
    workspace: .windsurf/skills
```

- `global` behaves exactly like the single-path form: relative to your home
  directory, `~/`-prefixed, or absolute. It is used for user-level installs.
- `workspace` is always relative to the project root and is used for
  `--workspace` installs. An absolute or `~/` value is a config error.
- Omitting `workspace` reuses `global`, so `kiro: .kiro/skills` and

  ```yaml
  kiro:
    global: .kiro/skills
  ```

  behave identically. An entry with no `global` path is a config error.

When `workspace` is omitted and `global` is absolute, there is no meaningful
project-level equivalent, so `--workspace` falls back to
`<workspace>/.<agent>/skills`. Give an explicit `workspace` path to control that.

Behaviour notes:
- `--agent <name>` (repeatable): links only into the named agents' directories,
  creating them if they do not exist. An unknown name is an error and nothing is
  installed.
- `--all-agents`: links into every configured agent, skipping those whose
  directory does not exist (so you don't get directories for tools you don't use).
- `install` and `uninstall` require one of the two; `list` defaults to every agent.

Set `SKILL_INSTALLER_CONFIG` to use a config file from another location.

### Skill Sources (`~/.agents/sources.yaml`)

Saved GitHub skill directories, written by `skill-installer source add` and
`source remove`. Unlike `config.yaml` it is tool-managed and rewritten in full on
every change, so comments added by hand are not preserved — which is why it is a
separate file. Override its location with `SKILL_INSTALLER_SOURCES`. See
[Manage Skill Sources](#manage-skill-sources).

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
│   ├── config.rs        # Coding agent mapping loaded from config.yaml
│   ├── sources.rs       # Saved skill sources in sources.yaml
│   ├── installer.rs     # Core installation logic
│   ├── skill.rs         # Skill validation functions
│   ├── github.rs        # GitHub URL parsing and skill download
│   └── mcp_server.rs    # MCP server implementation
├── Cargo.toml           # Project configuration and dependencies
├── Cargo.lock           # Dependency lock file
└── target/              # Build artifacts (gitignored)
```

### Key Components

- **main.rs**: Handles command parsing and delegates to appropriate modules
- **cli.rs**: Defines the command-line interface using clap
- **sources.rs**: Reads and writes the saved skill sources
- **installer.rs**: Implements skill copying and symlink creation logic
- **skill.rs**: Validates skill directories for required marker files
- **github.rs**: Parses GitHub URLs and downloads skill directories
- **mcp_server.rs**: Provides MCP server functionality with tool handlers

## Dependencies

- **anyhow**: Error handling and context management
- **clap**: Command-line argument parsing with derive macros
- **dirs**: Cross-platform directory path resolution
- **rmcp**: MCP (Model Context Protocol) server implementation with stdio transport
- **schemars**: JSON schema generation for MCP tool parameters
- **serde**: Serialization and deserialization framework
- **serde_json**: JSON support for serde
- **serde_yaml**: YAML parsing for `config.yaml` and `sources.yaml`
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