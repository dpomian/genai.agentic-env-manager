use anyhow::{bail, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;
use tokio::fs;
use tokio::task::JoinSet;

#[derive(Debug, Clone)]
pub struct GitHubPath {
    pub owner: String,
    pub repo: String,
    pub branch: String,
    pub path: String,
}

#[derive(Debug, Deserialize)]
struct GitHubContent {
    name: String,
    path: String,
    #[serde(rename = "type")]
    content_type: String,
    download_url: Option<String>,
}

impl GitHubPath {
    pub fn parse(url: &str) -> Result<Self> {
        let url = url.trim();

        // Handle URLs like: https://github.com/owner/repo/tree/branch/path/to/skill
        if let Some(rest) = url.strip_prefix("https://github.com/") {
            let parts: Vec<&str> = rest.splitn(5, '/').collect();

            if parts.len() < 4 {
                bail!("Invalid GitHub URL format. Expected: https://github.com/owner/repo/tree/branch/path");
            }

            let owner = parts[0].to_string();
            let repo = parts[1].to_string();

            // parts[2] should be "tree"
            if parts[2] != "tree" {
                bail!("Invalid GitHub URL format. Expected '/tree/' in URL");
            }

            let branch = parts[3].to_string();
            let path = if parts.len() > 4 {
                parts[4].to_string()
            } else {
                String::new()
            };

            return Ok(GitHubPath {
                owner,
                repo,
                branch,
                path,
            });
        }

        bail!("URL must start with https://github.com/");
    }

    pub fn skill_name(&self) -> Result<String> {
        let name = self
            .path
            .split('/')
            .last()
            .filter(|s| !s.is_empty())
            .context("Could not determine skill name from path")?;
        Ok(name.to_string())
    }

    fn api_url(&self) -> String {
        format!(
            "https://api.github.com/repos/{}/{}/contents/{}?ref={}",
            self.owner, self.repo, self.path, self.branch
        )
    }

    /// The same repository and branch at another path, for walking into a
    /// subdirectory of this one.
    fn child(&self, path: &str) -> Self {
        Self {
            owner: self.owner.clone(),
            repo: self.repo.clone(),
            branch: self.branch.clone(),
            path: path.to_string(),
        }
    }
}

async fn get_gh_token() -> Result<String> {
    let output = tokio::process::Command::new("gh")
        .args(["auth", "token"])
        .output()
        .await
        .context(
            "Failed to run 'gh auth token'. Please install GitHub CLI and run 'gh auth login'",
        )?;

    if !output.status.success() {
        bail!("GitHub CLI is not authenticated. Please run 'gh auth login' to authenticate.");
    }

    let token = String::from_utf8(output.stdout)
        .context("Invalid token output from gh CLI")?
        .trim()
        .to_string();

    if token.is_empty() {
        bail!("No GitHub token found. Please run 'gh auth login' to authenticate.");
    }

    Ok(token)
}

pub async fn download_skill(github_path: &GitHubPath, dest: &Path) -> Result<()> {
    let client = client().await?;

    download_directory(&client, github_path, dest).await
}

/// Builds an HTTP client authenticated with the `gh` CLI's token.
async fn client() -> Result<Client> {
    let token = get_gh_token().await?;

    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::AUTHORIZATION,
        format!("Bearer {}", token)
            .parse()
            .context("Invalid GitHub token format")?,
    );

    Client::builder()
        .user_agent("agentic-env-manager")
        .default_headers(headers)
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .context("Failed to create HTTP client")
}

/// One skill directory found inside a source.
#[derive(Debug, Clone, Serialize)]
pub struct RemoteSkill {
    /// Directory name, which is also what `install <source>:<name>` takes.
    pub name: String,
    /// Path of the skill within the repository.
    pub path: String,
    /// Frontmatter of the skill's marker file, when requested and present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frontmatter: Option<String>,
}

/// What a source directory turned out to contain.
#[derive(Debug, Clone, Serialize)]
pub struct SourceListing {
    /// Immediate subdirectories that hold a skill marker, sorted by name.
    pub skills: Vec<RemoteSkill>,
    /// True when the source directory is itself a skill, rather than a directory
    /// of skills.
    pub is_skill: bool,
    /// Subdirectories with no skill marker, sorted by name. Reported so a source
    /// pointing somewhere unexpected is visible rather than silently empty.
    pub skipped: Vec<String>,
}

/// How many directories to inspect at once. Listing a source costs one request
/// per subdirectory (two with frontmatter), so this keeps a large source quick
/// without hammering the API.
const MAX_CONCURRENCY: usize = 8;

/// Lists the skills in a source directory.
///
/// Every immediate subdirectory holding a `skill.md` / `SKILL.md` counts as a
/// skill. With `include_frontmatter`, each marker file is fetched and its
/// frontmatter block returned.
pub async fn list_skills(base: &GitHubPath, include_frontmatter: bool) -> Result<SourceListing> {
    let client = client().await?;
    let entries = fetch_contents(&client, base).await?;

    // A source may point straight at one skill instead of a directory of them.
    let is_skill = entries
        .iter()
        .any(|entry| entry.content_type == "file" && crate::skill::is_marker(&entry.name));

    let dirs: Vec<&GitHubContent> = entries
        .iter()
        .filter(|entry| entry.content_type == "dir")
        .collect();

    let mut skills = Vec::new();
    let mut skipped = Vec::new();

    for chunk in dirs.chunks(MAX_CONCURRENCY) {
        let mut tasks = JoinSet::new();

        for dir in chunk {
            let client = client.clone();
            let name = dir.name.clone();
            let path = base.child(&dir.path);

            tasks
                .spawn(async move { inspect_dir(&client, &path, name, include_frontmatter).await });
        }

        while let Some(joined) = tasks.join_next().await {
            match joined.context("failed to inspect a source directory")?? {
                Inspected::Skill(skill) => skills.push(skill),
                Inspected::NotASkill(name) => skipped.push(name),
            }
        }
    }

    // Concurrent inspection finishes out of order.
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skipped.sort();

    Ok(SourceListing {
        skills,
        is_skill,
        skipped,
    })
}

/// The outcome of looking inside one subdirectory of a source.
enum Inspected {
    Skill(RemoteSkill),
    NotASkill(String),
}

/// Checks one subdirectory for a skill marker, reading its frontmatter when asked.
async fn inspect_dir(
    client: &Client,
    path: &GitHubPath,
    name: String,
    include_frontmatter: bool,
) -> Result<Inspected> {
    let entries = fetch_contents(client, path).await?;

    let marker = entries
        .iter()
        .find(|entry| entry.content_type == "file" && crate::skill::is_marker(&entry.name));

    let Some(marker) = marker else {
        return Ok(Inspected::NotASkill(name));
    };

    let frontmatter = match (include_frontmatter, &marker.download_url) {
        (true, Some(url)) => {
            let content = fetch_text(client, url).await?;
            crate::skill::parse_frontmatter(&content)
        }
        _ => None,
    };

    Ok(Inspected::Skill(RemoteSkill {
        name,
        path: path.path.clone(),
        frontmatter,
    }))
}

/// Fetches one directory listing from the contents API.
async fn fetch_contents(client: &Client, github_path: &GitHubPath) -> Result<Vec<GitHubContent>> {
    let url = github_path.api_url();
    let response = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("Failed to fetch '{}'", url))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        bail!("GitHub API request failed with status {}: {}", status, text);
    }

    response
        .json()
        .await
        .context("Failed to parse GitHub API response")
}

/// Fetches a file's contents as text.
async fn fetch_text(client: &Client, url: &str) -> Result<String> {
    let response = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("Failed to download '{}'", url))?;

    if !response.status().is_success() {
        bail!("Failed to download file: {}", response.status());
    }

    response
        .text()
        .await
        .context("Failed to read response body")
}

async fn download_directory(client: &Client, github_path: &GitHubPath, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)
        .await
        .with_context(|| format!("Failed to create directory '{}'", dest.display()))?;

    let contents = fetch_contents(client, github_path).await?;

    for item in contents {
        let item_dest = dest.join(&item.name);

        if item.content_type == "dir" {
            let sub_path = github_path.child(&item.path);
            Box::pin(download_directory(client, &sub_path, &item_dest)).await?;
        } else if item.content_type == "file" {
            if let Some(download_url) = item.download_url {
                download_file(client, &download_url, &item_dest).await?;
            }
        }
    }

    Ok(())
}

async fn download_file(client: &Client, url: &str, dest: &Path) -> Result<()> {
    let response = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("Failed to download '{}'", url))?;

    if !response.status().is_success() {
        bail!("Failed to download file: {}", response.status());
    }

    let bytes = response
        .bytes()
        .await
        .context("Failed to read response body")?;

    fs::write(dest, &bytes)
        .await
        .with_context(|| format!("Failed to write file '{}'", dest.display()))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_github_url() {
        let url = "https://github.com/anthropics/skills/tree/main/skills/computer-use";
        let parsed = GitHubPath::parse(url).unwrap();
        assert_eq!(parsed.owner, "anthropics");
        assert_eq!(parsed.repo, "skills");
        assert_eq!(parsed.branch, "main");
        assert_eq!(parsed.path, "skills/computer-use");
    }

    #[test]
    fn test_skill_name() {
        let url = "https://github.com/anthropics/skills/tree/main/skills/computer-use";
        let parsed = GitHubPath::parse(url).unwrap();
        assert_eq!(parsed.skill_name().unwrap(), "computer-use");
    }
}
