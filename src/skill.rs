use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

/// File names that mark a directory as a skill.
pub(crate) const SKILL_MARKERS: &[&str] = &["skill.md", "SKILL.md"];

/// True when `name` is a skill marker file.
pub(crate) fn is_marker(name: &str) -> bool {
    SKILL_MARKERS.contains(&name)
}

/// Parses YAML frontmatter from markdown content (between `---` delimiters).
/// Returns `None` when there is no frontmatter block or it is empty.
pub(crate) fn parse_frontmatter(content: &str) -> Option<String> {
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

/// Validates a skill path by canonicalizing it and checking for marker files.
/// Returns the canonicalized path on success.
pub fn validate(path: &Path) -> Result<PathBuf> {
    let canonical = std::fs::canonicalize(path)
        .with_context(|| format!("Invalid skill path '{}'", path.display()))?;

    if !canonical.is_dir() {
        bail!("'{}' is not a directory", canonical.display());
    }

    let has_marker = SKILL_MARKERS
        .iter()
        .any(|name| canonical.join(name).is_file());
    if !has_marker {
        bail!(
            "'{}' is not a valid skill: no skill.md or SKILL.md found",
            canonical.display()
        );
    }

    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_is_read_from_between_the_delimiters() {
        let content = "---\nname: demo\ndescription: does things\n---\n\nbody\n";

        assert_eq!(
            parse_frontmatter(content).as_deref(),
            Some("name: demo\ndescription: does things")
        );
    }

    #[test]
    fn leading_blank_lines_before_the_block_are_tolerated() {
        assert_eq!(
            parse_frontmatter("\n\n---\nname: demo\n---\nbody").as_deref(),
            Some("name: demo")
        );
    }

    #[test]
    fn content_without_usable_frontmatter_yields_none() {
        for content in [
            "no frontmatter here",
            "# Heading\n\n---\nname: demo\n---\n",
            "---\nname: demo\n",
            "---\n\n---\nbody",
            "",
        ] {
            assert_eq!(parse_frontmatter(content), None, "{content:?}");
        }
    }

    #[test]
    fn both_marker_spellings_are_recognized() {
        assert!(is_marker("SKILL.md"));
        assert!(is_marker("skill.md"));
        assert!(!is_marker("README.md"));
        assert!(!is_marker("Skill.md"));
    }
}
