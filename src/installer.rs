use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::github;
use crate::skill;

const SYMLINK_TARGETS: &[&str] = &[
    ".kiro/skills",
    ".codeium/windsurf/skills",
    ".copilot/skills",
    ".claude/skills",
];

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
pub async fn install_from_source(source: &str) -> Result<String> {
    if is_github_url(source) {
        install_from_github(source).await
    } else {
        let path = PathBuf::from(source);
        install(&path)?;
        let skill_name = path
            .file_name()
            .context("Could not determine skill name from path")?
            .to_string_lossy()
            .to_string();
        Ok(skill_name)
    }
}

/// Installs a skill by validating, copying to `~/.agents/skills`, and creating symlinks
/// in each configured target directory.
pub fn install(skill_path: &Path) -> Result<()> {
    let skill_path = &skill::validate(skill_path)?;
    let home = home_dir()?;
    let skill_name = skill_path
        .file_name()
        .context("skill path has no directory name")?;

    // Copy skill to ~/.agents/skills/<name>
    let install_dir = home.join(".agents/skills");
    fs::create_dir_all(&install_dir).context("failed to create ~/.agents/skills")?;

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

    // Create symlinks in each target directory
    for target in SYMLINK_TARGETS {
        let target_dir = home.join(target);
        if !target_dir.is_dir() {
            println!("Skipping ~/{target} (directory does not exist)");
            continue;
        }

        let link_path = target_dir.join(skill_name);
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
        println!("Linked ~/{target}/{}", skill_name.to_string_lossy());
    }

    Ok(())
}

/// Uninstalls a skill by removing it from `~/.agents/skills` and removing symlinks
/// from each configured target directory.
pub fn uninstall(skill_name: &str) -> Result<()> {
    let home = home_dir()?;

    // Remove symlinks from each target directory first
    for target in SYMLINK_TARGETS {
        let target_dir = home.join(target);
        let link_path = target_dir.join(skill_name);

        if link_path.exists() || link_path.is_symlink() {
            fs::remove_file(&link_path)
                .or_else(|_| fs::remove_dir_all(&link_path))
                .with_context(|| {
                    format!("failed to remove symlink at '{}'", link_path.display())
                })?;
            println!("Removed ~/{target}/{skill_name}");
        }
    }

    // Remove skill from ~/.agents/skills/<name>
    let install_dir = home.join(".agents/skills");
    let installed_path = install_dir.join(skill_name);

    if installed_path.exists() {
        fs::remove_dir_all(&installed_path)
            .with_context(|| format!("failed to remove skill at '{}'", installed_path.display()))?;
        println!("Removed skill from {}", installed_path.display());
    } else {
        println!(
            "Skill '{}' not found at {}",
            skill_name,
            installed_path.display()
        );
    }

    Ok(())
}

/// Installs a skill from a GitHub URL.
/// Downloads the skill to a temp directory, validates it, installs it, and cleans up.
pub async fn install_from_github(url: &str) -> Result<String> {
    let github_path = github::GitHubPath::parse(url)?;
    let skill_name = github_path.skill_name()?;

    let temp_dir = std::env::temp_dir().join(&skill_name);
    if temp_dir.exists() {
        fs::remove_dir_all(&temp_dir)?;
    }

    github::download_skill(&github_path, &temp_dir).await?;

    install(&temp_dir)?;

    fs::remove_dir_all(&temp_dir)?;

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

const SKILL_MARKERS: &[&str] = &["skill.md", "SKILL.md"];

/// Lists all installed skills from `~/.agents/skills`
pub fn list_skills(include_frontmatter: bool) -> Result<Vec<SkillInfo>> {
    let home = home_dir()?;
    let install_dir = home.join(".agents/skills");

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
    dirs::home_dir().context("could not determine home directory")
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
