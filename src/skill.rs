use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

const SKILL_MARKERS: &[&str] = &["skill.md", "SKILL.md"];

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
