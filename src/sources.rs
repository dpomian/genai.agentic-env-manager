use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::home_dir;
use crate::github::GitHubPath;

/// Environment variable used to point at an alternate sources file.
pub(crate) const SOURCES_ENV_VAR: &str = "SKILL_INSTALLER_SOURCES";

/// Sources file location relative to the home directory.
const SOURCES_RELATIVE_PATH: &str = ".agents/sources.yaml";

/// Written above the serialized sources so a reader who opens the file knows it
/// is tool-managed. Unlike `config.yaml`, this file is rewritten on every
/// `source add` / `source remove`, so hand-written comments would not survive.
const FILE_HEADER: &str = "\
# skill-installer skill sources
#
# Saved GitHub directories that contain skills. Managed with
# `skill-installer source add` / `source remove`; this file is rewritten in
# full on every change, so comments added by hand are not preserved.
";

/// Characters that may not appear in a source name.
///
/// `:` is reserved so a later release can spell a skill inside a source as
/// `<source>:<skill>`; `/` and `\` are excluded so a name can never be confused
/// with a path or URL.
const RESERVED_IN_NAME: &[char] = &[':', '/', '\\'];

/// The sources file exactly as parsed. `sources` is optional so both an empty
/// file and a bare `sources:` key load as "no sources saved".
#[derive(Debug, Default, Deserialize, Serialize)]
struct RawSources {
    #[serde(default)]
    sources: Option<BTreeMap<String, String>>,
}

/// The saved skill sources: a name for each GitHub directory that holds skills.
#[derive(Debug, Clone)]
pub struct Sources {
    /// Maps source name -> GitHub URL, kept sorted by name for stable output.
    sources: BTreeMap<String, String>,
    /// Where this was read from, and where [`Sources::save`] writes back.
    path: PathBuf,
}

impl Sources {
    /// Loads the saved sources, treating a missing file as an empty set.
    ///
    /// Nothing is written here: unlike `config.yaml` there is no useful default
    /// content, so the file only appears once a source is actually added.
    pub fn load() -> Result<Self> {
        let path = Self::resolve_path()?;

        if !path.exists() {
            return Ok(Self {
                sources: BTreeMap::new(),
                path,
            });
        }

        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read sources '{}'", path.display()))?;

        Self::from_yaml(&raw, &path)
    }

    /// Parses sources from YAML. `path` is recorded for messages and saving.
    pub fn from_yaml(raw: &str, path: &Path) -> Result<Self> {
        let parsed: RawSources = if raw.trim().is_empty() {
            RawSources::default()
        } else {
            serde_yaml::from_str(raw)
                .with_context(|| format!("failed to parse sources '{}'", path.display()))?
        };

        let sources = parsed.sources.unwrap_or_default();

        // A URL that is already saved is reported but not rejected, so one bad
        // entry cannot make `source list` or `source remove` unusable.
        for (name, url) in &sources {
            if let Err(err) = GitHubPath::parse(url) {
                eprintln!(
                    "warning: source '{name}' in '{}' is not a usable GitHub URL: {err:#}",
                    path.display()
                );
            }
        }

        Ok(Self {
            sources,
            path: path.to_path_buf(),
        })
    }

    /// Serializes the sources, including the "tool-managed" header.
    pub fn to_yaml(&self) -> Result<String> {
        let raw = RawSources {
            sources: Some(self.sources.clone()),
        };
        let body = serde_yaml::to_string(&raw).context("failed to serialize sources")?;

        Ok(format!("{FILE_HEADER}{body}"))
    }

    /// Writes the sources back to the file they were loaded from, creating the
    /// `.agents` directory if this is the first source saved.
    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create sources directory '{}'", parent.display())
            })?;
        }

        fs::write(&self.path, self.to_yaml()?)
            .with_context(|| format!("failed to write sources '{}'", self.path.display()))
    }

    /// Saves `url` under `name`, returning the URL it replaced, if any.
    ///
    /// An existing name is only overwritten when `force` is set, so a typo'd
    /// `add` cannot silently repoint a source you already rely on.
    pub fn add(&mut self, name: &str, url: &str, force: bool) -> Result<Option<String>> {
        let name = validate_name(name)?;
        let url = url.trim();

        // Parse eagerly: a URL that cannot resolve to a skill directory is
        // worth rejecting now rather than at install time.
        GitHubPath::parse(url).with_context(|| {
            format!(
                "cannot save source '{name}': expected a GitHub directory URL such as \
                 https://github.com/owner/repo/tree/main/skills"
            )
        })?;

        if let Some(existing) = self.sources.get(&name) {
            if !force {
                bail!(
                    "source '{name}' already exists:\n  {existing}\n\n\
                     Pass --force to repoint it, or remove it first with \
                     `skill-installer source remove {name}`."
                );
            }
        }

        Ok(self.sources.insert(name, url.to_string()))
    }

    /// Removes a source, returning the URL it pointed at.
    pub fn remove(&mut self, name: &str) -> Result<String> {
        self.sources
            .remove(name.trim())
            .ok_or_else(|| self.unknown_source_error(name.trim()))
    }

    /// The URL saved under `name`.
    pub fn get(&self, name: &str) -> Result<&str> {
        let name = name.trim();
        self.sources
            .get(name)
            .map(String::as_str)
            .ok_or_else(|| self.unknown_source_error(name))
    }

    /// Expands a `<source>:<skill>` reference into the GitHub URL of that skill,
    /// by appending `skill` to the saved source's directory path.
    pub fn skill_url(&self, source: &str, skill: &str) -> Result<String> {
        let url = self.get(source)?;

        let base = GitHubPath::parse(url).with_context(|| {
            format!(
                "source '{}' in '{}' is not a usable GitHub URL; fix it with \
                 `skill-installer source add {} <url> --force`",
                source.trim(),
                self.path.display(),
                source.trim()
            )
        })?;

        let base_path = base.path.trim_matches('/');
        let path = if base_path.is_empty() {
            skill.to_string()
        } else {
            format!("{base_path}/{skill}")
        };

        Ok(format!(
            "https://github.com/{}/{}/tree/{}/{}",
            base.owner, base.repo, base.branch, path
        ))
    }

    /// Every saved source as `(name, url)`, ordered by name.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.sources
            .iter()
            .map(|(name, url)| (name.as_str(), url.as_str()))
    }

    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    /// The file these sources were loaded from.
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn unknown_source_error(&self, name: &str) -> anyhow::Error {
        if self.is_empty() {
            return anyhow::anyhow!(
                "unknown source '{name}'; no sources are saved yet\n\n\
                 Save one with:\n\
                 \x20 skill-installer source add <name> \
                 https://github.com/owner/repo/tree/main/skills"
            );
        }

        anyhow::anyhow!(
            "unknown source '{name}'\n\n\
             Saved sources in {}:\n{}",
            self.path.display(),
            self.listing()
        )
    }

    /// The saved sources, one `name -> url` per line, for messages.
    fn listing(&self) -> String {
        self.iter()
            .map(|(name, url)| format!("  {name} -> {url}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The sources file to read and write: `SKILL_INSTALLER_SOURCES` when set,
    /// otherwise `~/.agents/sources.yaml`.
    fn resolve_path() -> Result<PathBuf> {
        if let Some(path) = std::env::var_os(SOURCES_ENV_VAR) {
            return Ok(PathBuf::from(path));
        }
        Ok(home_dir()?.join(SOURCES_RELATIVE_PATH))
    }
}

/// Checks a source name and returns it trimmed.
fn validate_name(name: &str) -> Result<String> {
    let name = name.trim();

    if name.is_empty() {
        bail!("a source name cannot be empty");
    }

    if name.chars().any(char::is_whitespace) {
        bail!("source name '{name}' cannot contain whitespace");
    }

    if let Some(bad) = name.chars().find(|c| RESERVED_IN_NAME.contains(c)) {
        bail!(
            "source name '{name}' cannot contain '{bad}'; use a plain name such as `anthropic`"
        );
    }

    if name.starts_with('-') || name.starts_with('.') {
        bail!("source name '{name}' cannot start with '{}'", &name[..1]);
    }

    Ok(name.to_string())
}

/// How an `install` or `uninstall` argument was written.
///
/// The three forms are told apart without any I/O beyond one existence check, so
/// the same argument can be passed to either command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillReference {
    /// A full GitHub directory URL.
    Url(String),
    /// `<source>:<skill>`, naming a skill inside a saved source.
    Source { source: String, skill: String },
    /// A local path, or (for `uninstall`) a bare installed skill name.
    Local(String),
}

impl SkillReference {
    /// Classifies an `install` / `uninstall` argument.
    ///
    /// A URL wins first, so the `:` in `https://` is never mistaken for a
    /// reference separator. Otherwise a `:` marks a source reference, unless
    /// something with that literal name exists on disk — a real path always wins
    /// over a reference, so an unusual directory name stays installable.
    pub fn classify(input: &str) -> Result<Self> {
        let input = input.trim();

        if input.is_empty() {
            bail!("no skill given");
        }

        if input.starts_with("https://") || input.starts_with("http://") {
            return Ok(Self::Url(input.to_string()));
        }

        if let Some((source, skill)) = input.split_once(':') {
            // A path-shaped prefix (./a:b, /tmp/a:b) is a path, not a source.
            let looks_like_a_path =
                source.contains('/') || source.contains('\\') || Path::new(input).exists();

            if !looks_like_a_path {
                return Self::source_reference(source, skill);
            }
        }

        Ok(Self::Local(input.to_string()))
    }

    /// Builds and validates a `<source>:<skill>` reference.
    fn source_reference(source: &str, skill: &str) -> Result<Self> {
        let reference = format!("{source}:{skill}");

        let source = validate_name(source).with_context(|| {
            format!("invalid source in '{reference}'; expected <source>:<skill>")
        })?;
        let skill = validate_skill_path(skill).with_context(|| {
            format!("invalid skill in '{reference}'; expected <source>:<skill>")
        })?;

        Ok(Self::Source { source, skill })
    }
}

/// Checks the skill part of a `<source>:<skill>` reference and returns it
/// trimmed. It may name a nested directory (`docs/pdf`) but must stay inside the
/// source, so absolute and `..` paths are refused.
fn validate_skill_path(skill: &str) -> Result<String> {
    let skill = skill.trim();

    if skill.is_empty() {
        bail!("a skill name cannot be empty; write <source>:<skill>, e.g. anthropic:pdf");
    }

    if skill.chars().any(char::is_whitespace) {
        bail!("skill '{skill}' cannot contain whitespace");
    }

    if skill.contains('\\') || skill.contains(':') {
        bail!("skill '{skill}' cannot contain '\\' or ':'");
    }

    if skill.starts_with('/') || skill.ends_with('/') {
        bail!("skill '{skill}' cannot start or end with '/'");
    }

    if skill.split('/').any(|part| part.is_empty()) {
        bail!("skill '{skill}' has an empty path segment");
    }

    if skill.split('/').any(|part| part == "." || part == "..") {
        bail!("skill '{skill}' cannot contain '.' or '..'; it must stay inside the source");
    }

    Ok(skill.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    const URL: &str = "https://github.com/anthropics/skills/tree/main/skills";
    const OTHER_URL: &str = "https://github.com/owner/repo/tree/main/pack";

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn temp_dir(label: &str) -> PathBuf {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("skill-installer-sources-{label}-{id}"));
        if dir.exists() {
            fs::remove_dir_all(&dir).unwrap();
        }
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Sources backed by a path inside a temp dir, without touching the real
    /// home directory or the process environment.
    fn empty(root: &Path) -> Sources {
        Sources {
            sources: BTreeMap::new(),
            path: root.join("sources.yaml"),
        }
    }

    fn parse(raw: &str) -> Sources {
        Sources::from_yaml(raw, Path::new("/tmp/sources.yaml")).expect("sources should parse")
    }

    #[test]
    fn a_saved_source_can_be_read_back() {
        let root = temp_dir("round-trip");
        let mut sources = empty(&root);

        assert_eq!(sources.add("anthropic", URL, false).unwrap(), None);
        sources.save().unwrap();

        let reloaded = Sources::from_yaml(
            &fs::read_to_string(root.join("sources.yaml")).unwrap(),
            &root.join("sources.yaml"),
        )
        .unwrap();

        assert_eq!(reloaded.get("anthropic").unwrap(), URL);
    }

    #[test]
    fn the_saved_file_keeps_its_tool_managed_header() {
        let yaml = parse(&format!("sources:\n  anthropic: {URL}\n"))
            .to_yaml()
            .unwrap();

        assert!(yaml.starts_with("# skill-installer skill sources"), "{yaml}");
        assert!(yaml.contains(&format!("anthropic: {URL}")), "{yaml}");
    }

    #[test]
    fn saving_creates_the_agents_directory() {
        let root = temp_dir("mkdir");
        let path = root.join("nested/.agents/sources.yaml");
        let mut sources = Sources {
            sources: BTreeMap::new(),
            path: path.clone(),
        };

        sources.add("anthropic", URL, false).unwrap();
        sources.save().unwrap();

        assert!(path.is_file());
    }

    #[test]
    fn an_empty_or_missing_sources_list_parses_as_empty() {
        for raw in ["", "   \n", "sources:\n", "sources: {}\n"] {
            assert!(parse(raw).is_empty(), "{raw:?} should parse as empty");
        }
    }

    #[test]
    fn sources_are_listed_in_name_order() {
        let sources = parse(&format!(
            "sources:\n  zed: {OTHER_URL}\n  anthropic: {URL}\n"
        ));

        let names: Vec<_> = sources.iter().map(|(name, _)| name).collect();
        assert_eq!(names, vec!["anthropic", "zed"]);
    }

    #[test]
    fn adding_an_existing_name_needs_force() {
        let root = temp_dir("duplicate");
        let mut sources = empty(&root);
        sources.add("anthropic", URL, false).unwrap();

        let err = sources.add("anthropic", OTHER_URL, false).unwrap_err();
        let message = format!("{err}");
        assert!(message.contains("source 'anthropic' already exists"), "{message}");
        assert!(message.contains("--force"), "{message}");
        // The original URL survives the rejected add.
        assert_eq!(sources.get("anthropic").unwrap(), URL);

        let replaced = sources.add("anthropic", OTHER_URL, true).unwrap();
        assert_eq!(replaced.as_deref(), Some(URL));
        assert_eq!(sources.get("anthropic").unwrap(), OTHER_URL);
    }

    #[test]
    fn a_non_github_url_is_rejected_when_added() {
        let root = temp_dir("bad-url");
        let mut sources = empty(&root);

        for url in [
            "./local/skills",
            "https://gitlab.com/owner/repo/tree/main/skills",
            "https://github.com/owner/repo",
            "https://github.com/owner/repo/blob/main/skills",
        ] {
            let err = sources.add("bad", url, false).unwrap_err();
            assert!(
                format!("{err:#}").contains("cannot save source 'bad'"),
                "{url} should be rejected: {err:#}"
            );
        }

        assert!(sources.is_empty());
    }

    #[test]
    fn names_are_trimmed_and_checked() {
        let root = temp_dir("names");
        let mut sources = empty(&root);

        sources.add("  anthropic  ", URL, false).unwrap();
        assert_eq!(sources.get("anthropic").unwrap(), URL);

        // ':' stays free for a future `<source>:<skill>` install reference.
        let err = sources.add("an:thropic", URL, false).unwrap_err();
        assert!(format!("{err}").contains("cannot contain ':'"), "{err}");

        for (name, expected) in [
            ("", "cannot be empty"),
            ("   ", "cannot be empty"),
            ("two words", "cannot contain whitespace"),
            ("own/repo", "cannot contain '/'"),
            ("-dash", "cannot start with '-'"),
            (".dot", "cannot start with '.'"),
        ] {
            let err = sources.add(name, URL, false).unwrap_err();
            assert!(format!("{err}").contains(expected), "{name:?}: {err}");
        }
    }

    #[test]
    fn removing_a_source_returns_its_url() {
        let root = temp_dir("remove");
        let mut sources = empty(&root);
        sources.add("anthropic", URL, false).unwrap();

        assert_eq!(sources.remove("anthropic").unwrap(), URL);
        assert!(sources.is_empty());
    }

    #[test]
    fn unknown_names_list_the_saved_sources() {
        let sources = parse(&format!("sources:\n  anthropic: {URL}\n"));

        for err in [
            sources.get("nope").unwrap_err(),
            sources.clone().remove("nope").unwrap_err(),
        ] {
            let message = format!("{err}");
            assert!(message.contains("unknown source 'nope'"), "{message}");
            assert!(message.contains(&format!("anthropic -> {URL}")), "{message}");
            assert!(message.contains("/tmp/sources.yaml"), "{message}");
        }
    }

    #[test]
    fn an_unknown_name_with_nothing_saved_explains_how_to_add_one() {
        let err = parse("").get("nope").unwrap_err();
        let message = format!("{err}");

        assert!(message.contains("no sources are saved yet"), "{message}");
        assert!(message.contains("source add"), "{message}");
    }

    #[test]
    fn a_malformed_saved_url_still_loads() {
        // Warned about, not rejected, so `list` and `remove` keep working.
        let sources = parse("sources:\n  broken: not-a-url\n");

        assert_eq!(sources.get("broken").unwrap(), "not-a-url");
    }

    #[test]
    fn malformed_yaml_is_reported_with_the_file_path() {
        let err = Sources::from_yaml("sources: [oops\n", Path::new("/tmp/sources.yaml")).unwrap_err();

        assert!(
            format!("{err:#}").contains("failed to parse sources '/tmp/sources.yaml'"),
            "{err:#}"
        );
    }

    #[test]
    fn a_reference_resolves_to_a_url_under_the_source_directory() {
        let sources = parse(&format!("sources:\n  anthropic: {URL}\n"));

        assert_eq!(
            sources.skill_url("anthropic", "pdf").unwrap(),
            "https://github.com/anthropics/skills/tree/main/skills/pdf"
        );
        // A nested skill keeps its subdirectory.
        assert_eq!(
            sources.skill_url("anthropic", "docs/pdf").unwrap(),
            "https://github.com/anthropics/skills/tree/main/skills/docs/pdf"
        );
    }

    #[test]
    fn a_repo_root_source_resolves_without_a_doubled_slash() {
        let sources = parse("sources:\n  mine: https://github.com/me/skills/tree/main\n");

        assert_eq!(
            sources.skill_url("mine", "pdf").unwrap(),
            "https://github.com/me/skills/tree/main/pdf"
        );
    }

    #[test]
    fn a_trailing_slash_in_the_saved_url_does_not_double_up() {
        let sources = parse("sources:\n  mine: https://github.com/me/skills/tree/main/skills/\n");

        assert_eq!(
            sources.skill_url("mine", "pdf").unwrap(),
            "https://github.com/me/skills/tree/main/skills/pdf"
        );
    }

    #[test]
    fn resolving_through_an_unknown_source_lists_the_saved_ones() {
        let err = parse(&format!("sources:\n  anthropic: {URL}\n"))
            .skill_url("nope", "pdf")
            .unwrap_err();

        assert!(format!("{err}").contains("unknown source 'nope'"), "{err}");
    }

    #[test]
    fn resolving_through_a_broken_saved_url_says_how_to_fix_it() {
        let err = parse("sources:\n  broken: not-a-url\n")
            .skill_url("broken", "pdf")
            .unwrap_err();

        assert!(
            format!("{err:#}").contains("source add broken <url> --force"),
            "{err:#}"
        );
    }

    #[test]
    fn a_url_is_classified_as_a_url_despite_its_colon() {
        assert_eq!(
            SkillReference::classify(URL).unwrap(),
            SkillReference::Url(URL.to_string())
        );
        // Not a GitHub URL, but still a URL: the error comes from parsing it as
        // one rather than from treating it as a missing local path.
        assert!(matches!(
            SkillReference::classify("https://gitlab.com/o/r/tree/main/s").unwrap(),
            SkillReference::Url(_)
        ));
    }

    #[test]
    fn a_colon_marks_a_source_reference() {
        assert_eq!(
            SkillReference::classify("anthropic:pdf").unwrap(),
            SkillReference::Source {
                source: "anthropic".to_string(),
                skill: "pdf".to_string(),
            }
        );
        assert_eq!(
            SkillReference::classify(" anthropic:docs/pdf ").unwrap(),
            SkillReference::Source {
                source: "anthropic".to_string(),
                skill: "docs/pdf".to_string(),
            }
        );
    }

    #[test]
    fn paths_and_bare_names_are_classified_as_local() {
        for input in ["./my-skill", "/tmp/my-skill", "my-skill", "../a/b"] {
            assert_eq!(
                SkillReference::classify(input).unwrap(),
                SkillReference::Local(input.to_string()),
                "{input}"
            );
        }
    }

    #[test]
    fn a_path_shaped_prefix_is_not_a_source_reference() {
        // The colon is inside a path, so there is no source name to look up.
        assert_eq!(
            SkillReference::classify("./odd:name").unwrap(),
            SkillReference::Local("./odd:name".to_string())
        );
    }

    #[test]
    fn an_existing_directory_wins_over_a_source_reference() {
        let root = temp_dir("colon-dir");
        let odd = root.join("odd:name");
        fs::create_dir_all(&odd).unwrap();
        let path = odd.to_string_lossy().to_string();

        // Absolute, so it is path-shaped anyway; the interesting case is a
        // relative name that exists in the current directory.
        assert_eq!(
            SkillReference::classify(&path).unwrap(),
            SkillReference::Local(path.clone())
        );

        let previous = std::env::current_dir().unwrap();
        std::env::set_current_dir(&root).unwrap();
        let classified = SkillReference::classify("odd:name").unwrap();
        std::env::set_current_dir(previous).unwrap();

        assert_eq!(classified, SkillReference::Local("odd:name".to_string()));
    }

    #[test]
    fn a_malformed_reference_is_rejected() {
        for (input, expected) in [
            (":pdf", "a source name cannot be empty"),
            ("anthropic:", "a skill name cannot be empty"),
            ("anthropic:/pdf", "cannot start or end with '/'"),
            ("anthropic:pdf/", "cannot start or end with '/'"),
            ("anthropic:docs//pdf", "empty path segment"),
            ("anthropic:../secrets", "cannot contain '.' or '..'"),
            ("anthropic:docs/../../etc", "cannot contain '.' or '..'"),
            ("anthropic:a:b", "cannot contain '\\' or ':'"),
            ("-bad:pdf", "cannot start with '-'"),
            ("", "no skill given"),
        ] {
            let err = SkillReference::classify(input).unwrap_err();
            assert!(
                format!("{err:#}").contains(expected),
                "{input:?} should be rejected with {expected:?}: {err:#}"
            );
        }
    }
}
