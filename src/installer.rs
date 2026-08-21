use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::{self, AgentSelection, Config};
use crate::github;
use crate::skill;

/// Recursively copies a directory and its contents to `dest`.
fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)
        .with_context(|| format!("failed to create directory '{}'", dest.display()))?;

    for entry in fs::read_dir(src)
        .with_context(|| format!("failed to read directory '{}'", src.display()))?
    {
        let entry = entry?;
        let src_path = entry.path();
        let dest_path = dest.join(entry.file_name());

        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dest_path)?;
        } else {
            fs::copy(&src_path, &dest_path).with_context(|| {
                format!(
                    "failed to copy '{}' to '{}'",
                    src_path.display(),
                    dest_path.display()
                )
            })?;
        }
    }

    Ok(())
}

/// Detects if the source is a GitHub URL
fn is_github_url(source: &str) -> bool {
    source.starts_with("https://github.com/")
}

/// Installs a skill from a source (local path or GitHub URL).
/// Auto-detects whether the source is a GitHub URL or local path.
///
/// `selection` names one configured agent or every one of them.
/// `workspace` installs at project level instead of the home directory.
pub async fn install_from_source(
    source: &str,
    selection: &AgentSelection,
    workspace: Option<&Path>,
) -> Result<String> {
    if is_github_url(source) {
        install_from_github(source, selection, workspace).await
    } else {
        let path = PathBuf::from(source);
        install(&path, selection, workspace)?;
        let skill_name = path
            .file_name()
            .context("Could not determine skill name from path")?
            .to_string_lossy()
            .to_string();
        Ok(skill_name)
    }
}

/// Installs a skill either at project level (when `workspace` is set) or at user
/// level in the home directory.
pub fn install(
    skill_path: &Path,
    selection: &AgentSelection,
    workspace: Option<&Path>,
) -> Result<()> {
    match workspace {
        Some(workspace) => install_in_workspace(skill_path, selection, workspace),
        None => install_in_home(skill_path, selection),
    }
}

/// Installs a skill by validating, copying to `~/.agents/skills`, and creating
/// symlinks in the target directories configured for the selected coding agent(s).
fn install_in_home(skill_path: &Path, selection: &AgentSelection) -> Result<()> {
    install_in_home_at(skill_path, selection, &agents_skills_dir()?)
}

/// `install_in_home` with the shared skill directory injected, so tests do not
/// have to relocate the home directory.
fn install_in_home_at(
    skill_path: &Path,
    selection: &AgentSelection,
    install_dir: &Path,
) -> Result<()> {
    let skill_path = &skill::validate(skill_path)?;

    // Resolve targets before copying anything so an unknown --coding-agent
    // aborts without leaving a partially installed skill behind.
    let config = Config::load()?;
    let targets = config.resolve(selection)?;

    let skill_name = skill_path
        .file_name()
        .context("skill path has no directory name")?;

    // Copy skill to ~/.agents/skills/<name>
    fs::create_dir_all(install_dir)
        .with_context(|| format!("failed to create '{}'", install_dir.display()))?;

    let installed_path = install_dir.join(skill_name);
    if installed_path.exists() {
        fs::remove_dir_all(&installed_path).with_context(|| {
            format!(
                "failed to remove existing skill at '{}'",
                installed_path.display()
            )
        })?;
    }

    copy_dir_recursive(skill_path, &installed_path)?;
    println!("Copied skill to {}", installed_path.display());

    // Create symlinks in each resolved target directory
    for target in &targets {
        if !target.dir.is_dir() {
            if selection.is_all() {
                // Linking into every agent: don't create directories for tools
                // that aren't installed.
                println!(
                    "Skipping {} ({} does not exist)",
                    target.name,
                    target.dir.display()
                );
                continue;
            }
            // The agent was requested by name, so honour it.
            fs::create_dir_all(&target.dir).with_context(|| {
                format!("failed to create directory '{}'", target.dir.display())
            })?;
        }

        let link_path = target.dir.join(skill_name);
        if link_path.exists() || link_path.is_symlink() {
            fs::remove_file(&link_path)
                .or_else(|_| fs::remove_dir_all(&link_path))
                .with_context(|| {
                    format!(
                        "failed to remove existing entry at '{}'",
                        link_path.display()
                    )
                })?;
        }

        create_symlink(&installed_path, &link_path)
            .with_context(|| format!("failed to create symlink at '{}'", link_path.display()))?;
        println!("Linked {} -> {}", link_path.display(), target.name);
    }

    Ok(())
}

/// Installs a skill at project level: the skill is copied straight into
/// `<workspace>/<agent dir>/<name>` and no symlink is created, so the project
/// directory is self-contained.
///
/// A project-level install always targets exactly one coding agent, so
/// `AgentSelection::All` is rejected here.
fn install_in_workspace(
    skill_path: &Path,
    selection: &AgentSelection,
    workspace: &Path,
) -> Result<()> {
    let skill_path = &skill::validate(skill_path)?;
    let workspace = canonical_workspace(workspace)?;

    let config = Config::load()?;
    let AgentSelection::One(coding_agent) = selection else {
        return Err(config.all_agents_in_workspace_error());
    };
    let target = config.resolve_one_in_workspace(coding_agent, &workspace)?;

    let skill_name = skill_path
        .file_name()
        .context("skill path has no directory name")?;

    fs::create_dir_all(&target.dir)
        .with_context(|| format!("failed to create directory '{}'", target.dir.display()))?;

    let dest = target.dir.join(skill_name);

    // Guard against wiping the source when re-installing a skill that already
    // lives at the destination.
    if fs::canonicalize(&dest).is_ok_and(|p| p == *skill_path) {
        bail!(
            "skill source and destination are the same path ('{}')",
            dest.display()
        );
    }

    if dest.exists() || dest.is_symlink() {
        fs::remove_file(&dest)
            .or_else(|_| fs::remove_dir_all(&dest))
            .with_context(|| format!("failed to remove existing entry at '{}'", dest.display()))?;
    }

    copy_dir_recursive(skill_path, &dest)?;
    println!("Installed {} for {}", dest.display(), target.name);

    Ok(())
}

/// Canonicalizes a workspace path, which must already exist.
fn canonical_workspace(workspace: &Path) -> Result<PathBuf> {
    let canonical = fs::canonicalize(workspace)
        .with_context(|| format!("invalid workspace path '{}'", workspace.display()))?;

    if !canonical.is_dir() {
        bail!("workspace '{}' is not a directory", canonical.display());
    }

    Ok(canonical)
}

/// What an [`uninstall`] call actually removed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UninstallReport {
    /// Coding agents the skill was removed from.
    pub removed_from: Vec<String>,
    /// Whether the shared `~/.agents/skills` copy was removed. Always false for
    /// a project-level uninstall, which has no shared copy.
    pub removed_copy: bool,
}

impl UninstallReport {
    /// True when there was nothing to remove.
    pub fn is_noop(&self) -> bool {
        self.removed_from.is_empty() && !self.removed_copy
    }
}

/// Uninstalls a skill from the selected coding agent(s), mirroring `install`.
///
/// At project level (`workspace` set) the skill directory is removed from the
/// selected agent directories in that workspace. At user level the agent
/// symlinks are removed, and the shared copy in `~/.agents/skills` is removed
/// once no configured agent links to it any more.
///
/// A skill that was never installed — or was not installed for the selected
/// agent — is a no-op, reported in the returned [`UninstallReport`]. An agent
/// that is not in `config.yaml` is still an error.
pub fn uninstall(
    skill_name: &str,
    selection: &AgentSelection,
    workspace: Option<&Path>,
) -> Result<UninstallReport> {
    let config = Config::load()?;

    match workspace {
        Some(workspace) => uninstall_in_workspace(skill_name, selection, &config, workspace),
        None => uninstall_in_home(skill_name, selection, &config),
    }
}

fn uninstall_in_home(
    skill_name: &str,
    selection: &AgentSelection,
    config: &Config,
) -> Result<UninstallReport> {
    uninstall_in_home_at(skill_name, selection, config, &agents_skills_dir()?)
}

/// `uninstall_in_home` with the shared skill directory injected, so tests do not
/// have to relocate the home directory.
fn uninstall_in_home_at(
    skill_name: &str,
    selection: &AgentSelection,
    config: &Config,
    install_dir: &Path,
) -> Result<UninstallReport> {
    let mut report = UninstallReport::default();

    // Remove the symlinks for the selected agents first.
    for target in config.resolve(selection)? {
        let link_path = target.dir.join(skill_name);

        if remove_entry(&link_path)? {
            println!("Removed {}", link_path.display());
            report.removed_from.push(target.name);
        }
    }

    // The copy under ~/.agents/skills is shared by every agent symlink, so it
    // can only go once nothing points at it any more. Otherwise uninstalling
    // for one agent would leave the others dangling.
    let installed_path = install_dir.join(skill_name);

    if installed_path.exists() && !any_agent_links(config, skill_name)? {
        fs::remove_dir_all(&installed_path)
            .with_context(|| format!("failed to remove skill at '{}'", installed_path.display()))?;
        println!("Removed skill from {}", installed_path.display());
        report.removed_copy = true;
    }

    Ok(report)
}

fn uninstall_in_workspace(
    skill_name: &str,
    selection: &AgentSelection,
    config: &Config,
    workspace: &Path,
) -> Result<UninstallReport> {
    let workspace = canonical_workspace(workspace)?;
    let mut report = UninstallReport::default();

    // Each workspace install is a self-contained copy, so there is no shared
    // copy to reference-count here.
    for target in config.resolve_in_workspace(selection, &workspace)? {
        let path = target.dir.join(skill_name);

        if remove_entry(&path)? {
            println!("Removed {}", path.display());
            report.removed_from.push(target.name);
        }
    }

    Ok(report)
}

/// True when any configured agent still has an entry for `skill_name`.
fn any_agent_links(config: &Config, skill_name: &str) -> Result<bool> {
    for target in config.resolve(&AgentSelection::All)? {
        let path = target.dir.join(skill_name);
        if path.exists() || path.is_symlink() {
            return Ok(true);
        }
    }

    Ok(false)
}

/// Removes a file, symlink or directory if it is there. Returns whether
/// anything was removed, so a missing entry stays a no-op.
fn remove_entry(path: &Path) -> Result<bool> {
    // `exists()` follows symlinks and so reports false for a dangling one.
    if !path.exists() && !path.is_symlink() {
        return Ok(false);
    }

    fs::remove_file(path)
        .or_else(|_| fs::remove_dir_all(path))
        .with_context(|| format!("failed to remove '{}'", path.display()))?;

    Ok(true)
}

/// Installs a skill from a GitHub URL.
/// Downloads the skill to a temp directory, validates it, installs it, and cleans up.
pub async fn install_from_github(
    url: &str,
    selection: &AgentSelection,
    workspace: Option<&Path>,
) -> Result<String> {
    let github_path = github::GitHubPath::parse(url)?;
    let skill_name = github_path.skill_name()?;

    let temp_dir = std::env::temp_dir().join(&skill_name);
    if temp_dir.exists() {
        fs::remove_dir_all(&temp_dir)?;
    }

    github::download_skill(&github_path, &temp_dir).await?;

    let result = install(&temp_dir, selection, workspace);

    fs::remove_dir_all(&temp_dir)?;
    result?;

    Ok(skill_name)
}

/// Information about an installed skill
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillInfo {
    pub name: String,
    pub path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frontmatter: Option<String>,
}

/// Per-agent skill breakdown
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSkills {
    pub agent_name: String,
    /// Skills linked in this agent's directory (subset of all installed skills)
    pub skills: Vec<SkillInfo>,
}

const SKILL_MARKERS: &[&str] = &["skill.md", "SKILL.md"];

/// Lists all installed skills from `~/.agents/skills`
pub fn list_skills(include_frontmatter: bool) -> Result<Vec<SkillInfo>> {
    let install_dir = agents_skills_dir()?;

    if !install_dir.exists() {
        return Ok(Vec::new());
    }

    let mut skills = Vec::new();

    for entry in fs::read_dir(&install_dir)
        .with_context(|| format!("failed to read directory '{}'", install_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();

        if !path.is_dir() {
            continue;
        }

        let name = entry.file_name().to_string_lossy().to_string();

        let frontmatter = if include_frontmatter {
            extract_frontmatter(&path)
        } else {
            None
        };

        skills.push(SkillInfo {
            name,
            path,
            frontmatter,
        });
    }

    skills.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(skills)
}

/// Returns one `AgentSkills` entry per configured coding agent, each listing
/// the skills that have a symlink (or directory) in that agent's skill directory.
///
/// `selection` limits the breakdown to a single configured agent (an unknown
/// name is an error listing the configured agents) or covers every agent.
/// `workspace` lists project-level installs instead of the home-level ones.
pub fn list_skills_by_agent(
    include_frontmatter: bool,
    selection: &AgentSelection,
    workspace: Option<&Path>,
) -> Result<Vec<AgentSkills>> {
    let config = Config::load()?;

    // Home-level skills live in ~/.agents/skills and are linked into the agent
    // directories; workspace installs are plain directories with no shared root.
    let (targets, install_dir) = match workspace {
        Some(workspace) => {
            let workspace = canonical_workspace(workspace)?;
            (config.resolve_in_workspace(selection, &workspace)?, None)
        }
        None => (
            config.resolve(selection)?,
            Some(agents_skills_dir()?),
        ),
    };

    let mut result = Vec::new();

    for target in targets {
        let mut skills: Vec<SkillInfo> = Vec::new();

        if target.dir.is_dir() {
            for entry in fs::read_dir(&target.dir).with_context(|| {
                format!("failed to read directory '{}'", target.dir.display())
            })? {
                let entry = entry?;
                let entry_path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();

                let (skill_path, fm_path) = match &install_dir {
                    Some(install_dir) => {
                        // Resolve symlinks: we only want entries that point into
                        // ~/.agents/skills so we don't accidentally list
                        // unrelated dirs.
                        let resolved = if entry_path.is_symlink() {
                            fs::read_link(&entry_path).ok()
                        } else {
                            Some(entry_path.clone())
                        };

                        let is_skill = resolved
                            .as_ref()
                            .is_some_and(|p| p.starts_with(install_dir) || entry_path.is_dir());
                        if !is_skill {
                            continue;
                        }

                        // Read frontmatter from the canonical install path if
                        // available, falling back to the symlink target itself.
                        let canonical_path = install_dir.join(&name);
                        let fm_path = if canonical_path.is_dir() {
                            canonical_path.clone()
                        } else {
                            entry_path.clone()
                        };
                        (canonical_path, fm_path)
                    }
                    None => {
                        if !entry_path.is_dir() {
                            continue;
                        }
                        (entry_path.clone(), entry_path.clone())
                    }
                };

                let frontmatter = if include_frontmatter {
                    extract_frontmatter(&fm_path)
                } else {
                    None
                };

                skills.push(SkillInfo {
                    name,
                    path: skill_path,
                    frontmatter,
                });
            }
        }

        skills.sort_by(|a, b| a.name.cmp(&b.name));
        result.push(AgentSkills {
            agent_name: target.name,
            skills,
        });
    }

    Ok(result)
}

/// Extracts frontmatter from a skill's marker file (skill.md or SKILL.md)
fn extract_frontmatter(skill_path: &Path) -> Option<String> {
    for marker in SKILL_MARKERS {
        let marker_path = skill_path.join(marker);
        if marker_path.is_file() {
            if let Ok(content) = fs::read_to_string(&marker_path) {
                return parse_frontmatter(&content);
            }
        }
    }
    None
}

/// Parses YAML frontmatter from markdown content (between --- delimiters)
fn parse_frontmatter(content: &str) -> Option<String> {
    let content = content.trim_start();
    if !content.starts_with("---") {
        return None;
    }

    let after_first = &content[3..];
    if let Some(end_idx) = after_first.find("\n---") {
        let frontmatter = after_first[..end_idx].trim();
        if !frontmatter.is_empty() {
            return Some(frontmatter.to_string());
        }
    }

    None
}

fn home_dir() -> Result<PathBuf> {
    config::home_dir()
}

/// The shared directory holding the canonical copy of every user-level skill.
fn agents_skills_dir() -> Result<PathBuf> {
    Ok(home_dir()?.join(".agents/skills"))
}

#[cfg(unix)]
fn create_symlink(original: &Path, link: &Path) -> Result<()> {
    std::os::unix::fs::symlink(original, link)?;
    Ok(())
}

#[cfg(windows)]
fn create_symlink(original: &Path, link: &Path) -> Result<()> {
    std::os::windows::fs::symlink_dir(original, link)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Mutex, MutexGuard};

    const TEST_CONFIG: &str = "coding_agents:\n  kiro: .kiro/skills\n  claude: .claude/skills\n";

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    /// `SKILL_INSTALLER_CONFIG` is process-global, so only one test may have a
    /// config installed at a time.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn lock_env() -> MutexGuard<'static, ()> {
        // Poisoning only means some other test failed; the env var is still
        // ours to overwrite.
        ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Creates a unique temp directory for one test case.
    fn temp_dir(label: &str) -> PathBuf {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("skill-installer-test-{label}-{id}"));
        if dir.exists() {
            fs::remove_dir_all(&dir).unwrap();
        }
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Points `Config::load` at a fixture config with home-relative agent
    /// directories. Hold the returned guard for the duration of the test.
    fn use_test_config(root: &Path) -> MutexGuard<'static, ()> {
        let guard = lock_env();
        let config_path = root.join("config.yaml");
        fs::write(&config_path, TEST_CONFIG).unwrap();
        std::env::set_var(config::CONFIG_ENV_VAR, &config_path);
        guard
    }

    /// Creates a minimal valid skill directory.
    fn make_skill(root: &Path, name: &str) -> PathBuf {
        let dir = root.join(name);
        fs::create_dir_all(dir.join("nested")).unwrap();
        fs::write(dir.join("SKILL.md"), "---\nname: demo\n---\nbody\n").unwrap();
        fs::write(dir.join("nested/extra.txt"), "extra").unwrap();
        dir
    }

    fn one(name: &str) -> AgentSelection {
        AgentSelection::One(name.to_string())
    }

    /// Points `Config::load` at a config whose agent directories are absolute
    /// paths inside `root`, so user-level installs can be exercised without
    /// relocating the home directory. Returns the env guard and the shared
    /// skills directory to pass to the `*_at` helpers.
    fn use_absolute_config(root: &Path) -> (MutexGuard<'static, ()>, PathBuf) {
        let guard = lock_env();
        let config_path = root.join("config.yaml");
        let yaml = format!(
            "coding_agents:\n  kiro: {}/.kiro/skills\n  claude: {}/.claude/skills\n",
            root.display(),
            root.display()
        );
        fs::write(&config_path, yaml).unwrap();
        std::env::set_var(config::CONFIG_ENV_VAR, &config_path);

        (guard, root.join("agents/skills"))
    }

    #[test]
    fn home_uninstall_keeps_the_shared_copy_while_another_agent_links_it() {
        let root = temp_dir("home-refcount");
        let (_env, shared) = use_absolute_config(&root);
        let skill = make_skill(&root, "demo");

        install_in_home_at(&skill, &one("kiro"), &shared).unwrap();
        install_in_home_at(&skill, &one("claude"), &shared).unwrap();

        // One copy, two symlinks.
        assert!(shared.join("demo").is_dir());
        assert!(root.join(".kiro/skills/demo").is_symlink());
        assert!(root.join(".claude/skills/demo").is_symlink());

        let config = Config::load().unwrap();
        let report = uninstall_in_home_at("demo", &one("kiro"), &config, &shared).unwrap();

        assert_eq!(report.removed_from, vec!["kiro"]);
        assert!(
            !report.removed_copy,
            "shared copy must survive while claude links it"
        );
        assert!(!root.join(".kiro/skills/demo").exists());
        // claude's symlink still resolves, so it is not dangling.
        assert!(root.join(".claude/skills/demo").is_symlink());
        assert!(root.join(".claude/skills/demo").exists());
        assert!(shared.join("demo").is_dir());
    }

    #[test]
    fn home_uninstall_drops_the_shared_copy_with_the_last_agent() {
        let root = temp_dir("home-last-agent");
        let (_env, shared) = use_absolute_config(&root);
        let skill = make_skill(&root, "demo");

        install_in_home_at(&skill, &one("kiro"), &shared).unwrap();
        install_in_home_at(&skill, &one("claude"), &shared).unwrap();

        let config = Config::load().unwrap();
        uninstall_in_home_at("demo", &one("kiro"), &config, &shared).unwrap();
        let report = uninstall_in_home_at("demo", &one("claude"), &config, &shared).unwrap();

        assert_eq!(report.removed_from, vec!["claude"]);
        assert!(report.removed_copy, "last agent should drop the copy");
        assert!(!shared.join("demo").exists());
    }

    #[test]
    fn home_uninstall_all_removes_links_and_copy_at_once() {
        let root = temp_dir("home-all");
        let (_env, shared) = use_absolute_config(&root);
        let skill = make_skill(&root, "demo");

        // `all` only links into agents that already have a directory.
        fs::create_dir_all(root.join(".kiro/skills")).unwrap();
        fs::create_dir_all(root.join(".claude/skills")).unwrap();
        install_in_home_at(&skill, &AgentSelection::All, &shared).unwrap();
        assert!(root.join(".kiro/skills/demo").is_symlink());
        assert!(root.join(".claude/skills/demo").is_symlink());

        let config = Config::load().unwrap();
        let report =
            uninstall_in_home_at("demo", &AgentSelection::All, &config, &shared).unwrap();

        assert_eq!(report.removed_from, vec!["claude", "kiro"]);
        assert!(report.removed_copy);
        assert!(!shared.join("demo").exists());
        assert!(!root.join(".kiro/skills/demo").exists());
        assert!(!root.join(".claude/skills/demo").exists());
    }

    #[test]
    fn home_uninstall_of_an_unknown_skill_is_a_noop() {
        let root = temp_dir("home-noop");
        let (_env, shared) = use_absolute_config(&root);
        let skill = make_skill(&root, "demo");

        install_in_home_at(&skill, &one("kiro"), &shared).unwrap();

        let config = Config::load().unwrap();

        let report = uninstall_in_home_at("ghost", &AgentSelection::All, &config, &shared).unwrap();
        assert!(report.is_noop());

        // Installed, but not for the agent asked about.
        let report = uninstall_in_home_at("demo", &one("claude"), &config, &shared).unwrap();
        assert!(report.is_noop());
        // kiro's install is untouched.
        assert!(root.join(".kiro/skills/demo").is_symlink());
        assert!(shared.join("demo").is_dir());
    }

    #[test]
    fn home_uninstall_reclaims_an_orphaned_copy() {
        let root = temp_dir("home-orphan");
        let (_env, shared) = use_absolute_config(&root);
        let skill = make_skill(&root, "demo");

        install_in_home_at(&skill, &one("kiro"), &shared).unwrap();
        // Simulate the agent directory being cleaned up by hand, leaving the
        // shared copy with nothing pointing at it.
        fs::remove_dir_all(root.join(".kiro/skills")).unwrap();

        let config = Config::load().unwrap();
        let report = uninstall_in_home_at("demo", &one("kiro"), &config, &shared).unwrap();

        assert!(report.removed_from.is_empty());
        assert!(report.removed_copy, "orphaned copy should be reclaimed");
        assert!(!shared.join("demo").exists());
        assert!(!report.is_noop());
    }

    #[test]
    fn workspace_install_copies_into_the_agent_directory_without_symlinks() {
        let root = temp_dir("explicit-agent");
        let _env = use_test_config(&root);
        let skill = make_skill(&root, "demo");
        let workspace = root.join("project");
        fs::create_dir_all(&workspace).unwrap();

        install(&skill, &one("kiro"), Some(&workspace)).unwrap();

        let installed = workspace.join(".kiro/skills/demo");
        assert!(installed.is_dir(), "{} should exist", installed.display());
        assert!(!installed.is_symlink(), "install must be a real directory");
        assert!(installed.join("SKILL.md").is_file());
        assert_eq!(
            fs::read_to_string(installed.join("nested/extra.txt")).unwrap(),
            "extra"
        );
        // The other configured agent was not requested.
        assert!(!workspace.join(".claude").exists());
    }

    #[test]
    fn workspace_install_is_idempotent() {
        let root = temp_dir("reinstall");
        let _env = use_test_config(&root);
        let skill = make_skill(&root, "demo");
        let workspace = root.join("project");
        fs::create_dir_all(&workspace).unwrap();

        install(&skill, &one("kiro"), Some(&workspace)).unwrap();

        // A stale file from a previous version must not survive the reinstall.
        let stale = workspace.join(".kiro/skills/demo/stale.txt");
        fs::write(&stale, "old").unwrap();

        install(&skill, &one("kiro"), Some(&workspace)).unwrap();

        assert!(!stale.exists(), "stale file should have been removed");
        assert!(workspace.join(".kiro/skills/demo/SKILL.md").is_file());
    }

    #[test]
    fn workspace_install_rejects_all_agents() {
        let root = temp_dir("all-in-workspace");
        let _env = use_test_config(&root);
        let skill = make_skill(&root, "demo");
        let workspace = root.join("project");
        fs::create_dir_all(&workspace).unwrap();

        let err = install(&skill, &AgentSelection::All, Some(&workspace)).unwrap_err();
        let message = format!("{err:#}");

        assert!(
            message.contains("--coding-agent all is not supported with --workspace"),
            "{message}"
        );
        // The message points at the configured agents to choose from.
        assert!(message.contains("kiro -> .kiro/skills"), "{message}");
        // Nothing was written to the workspace.
        assert!(!workspace.join(".kiro").exists());
        assert!(!workspace.join(".claude").exists());
    }

    #[test]
    fn workspace_install_rejects_an_unknown_agent() {
        let root = temp_dir("unknown-agent");
        let _env = use_test_config(&root);
        let skill = make_skill(&root, "demo");
        let workspace = root.join("project");
        fs::create_dir_all(&workspace).unwrap();

        let err = install(&skill, &one("cursor"), Some(&workspace)).unwrap_err();

        assert!(
            format!("{err:#}").contains("unknown coding agent 'cursor'"),
            "{err:#}"
        );
        assert!(!workspace.join(".cursor").exists());
    }

    #[test]
    fn workspace_install_rejects_a_missing_workspace() {
        let root = temp_dir("missing-workspace");
        let _env = use_test_config(&root);
        let skill = make_skill(&root, "demo");

        let err = install(&skill, &one("kiro"), Some(&root.join("nope"))).unwrap_err();
        assert!(format!("{err:#}").contains("invalid workspace path"), "{err:#}");
    }

    #[test]
    fn workspace_install_refuses_to_overwrite_its_own_source() {
        let root = temp_dir("self-install");
        let _env = use_test_config(&root);
        let workspace = root.join("project");
        let skill = make_skill(&workspace.join(".kiro/skills"), "demo");

        let err = install(&skill, &one("kiro"), Some(&workspace)).unwrap_err();
        assert!(
            format!("{err:#}").contains("source and destination are the same"),
            "{err:#}"
        );
        // Source survived.
        assert!(skill.join("SKILL.md").is_file());
    }

    #[test]
    fn workspace_uninstall_removes_the_skill_from_the_selected_agent_only() {
        let root = temp_dir("uninstall-one");
        let _env = use_test_config(&root);
        let skill = make_skill(&root, "demo");
        let workspace = root.join("project");
        fs::create_dir_all(&workspace).unwrap();

        install(&skill, &one("kiro"), Some(&workspace)).unwrap();
        install(&skill, &one("claude"), Some(&workspace)).unwrap();

        let report = uninstall("demo", &one("kiro"), Some(&workspace)).unwrap();

        assert_eq!(report.removed_from, vec!["kiro"]);
        assert!(!report.is_noop());
        assert!(!workspace.join(".kiro/skills/demo").exists());
        // The other agent keeps its own copy.
        assert!(workspace.join(".claude/skills/demo").is_dir());
        // The original source is untouched.
        assert!(skill.join("SKILL.md").is_file());
    }

    #[test]
    fn workspace_uninstall_all_removes_the_skill_from_every_agent_directory() {
        let root = temp_dir("uninstall-all");
        let _env = use_test_config(&root);
        let skill = make_skill(&root, "demo");
        let workspace = root.join("project");
        fs::create_dir_all(&workspace).unwrap();

        install(&skill, &one("kiro"), Some(&workspace)).unwrap();
        install(&skill, &one("claude"), Some(&workspace)).unwrap();

        let report = uninstall("demo", &AgentSelection::All, Some(&workspace)).unwrap();

        assert_eq!(report.removed_from, vec!["claude", "kiro"]);
        assert!(!workspace.join(".kiro/skills/demo").exists());
        assert!(!workspace.join(".claude/skills/demo").exists());
    }

    #[test]
    fn uninstalling_a_skill_that_is_not_there_is_a_noop() {
        let root = temp_dir("uninstall-noop");
        let _env = use_test_config(&root);
        let skill = make_skill(&root, "demo");
        let workspace = root.join("project");
        fs::create_dir_all(&workspace).unwrap();

        install(&skill, &one("kiro"), Some(&workspace)).unwrap();

        // Never installed at all.
        let report = uninstall("ghost", &AgentSelection::All, Some(&workspace)).unwrap();
        assert!(report.is_noop());
        assert!(report.removed_from.is_empty());

        // Installed, but not for the selected agent.
        let report = uninstall("demo", &one("claude"), Some(&workspace)).unwrap();
        assert!(report.is_noop());
        // The kiro install is left alone.
        assert!(workspace.join(".kiro/skills/demo").is_dir());
    }

    #[test]
    fn uninstalling_from_an_unknown_agent_is_still_an_error() {
        let root = temp_dir("uninstall-unknown");
        let _env = use_test_config(&root);
        let workspace = root.join("project");
        fs::create_dir_all(&workspace).unwrap();

        let err = uninstall("demo", &one("cursor"), Some(&workspace)).unwrap_err();
        assert!(
            format!("{err:#}").contains("unknown coding agent 'cursor'"),
            "{err:#}"
        );
    }

    #[test]
    fn workspace_list_reports_project_level_skills_per_agent() {
        let root = temp_dir("list");
        let _env = use_test_config(&root);
        let skill = make_skill(&root, "demo");
        let workspace = root.join("project");
        fs::create_dir_all(&workspace).unwrap();

        install(&skill, &one("kiro"), Some(&workspace)).unwrap();

        let agents =
            list_skills_by_agent(true, &AgentSelection::All, Some(&workspace)).unwrap();

        let kiro = agents.iter().find(|a| a.agent_name == "kiro").unwrap();
        assert_eq!(kiro.skills.len(), 1);
        assert_eq!(kiro.skills[0].name, "demo");
        assert_eq!(
            kiro.skills[0].path,
            fs::canonicalize(&workspace).unwrap().join(".kiro/skills/demo")
        );
        assert_eq!(kiro.skills[0].frontmatter.as_deref(), Some("name: demo"));

        let claude = agents.iter().find(|a| a.agent_name == "claude").unwrap();
        assert!(claude.skills.is_empty());
    }
}
