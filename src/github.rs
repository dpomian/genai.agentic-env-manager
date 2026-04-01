use anyhow::{bail, Context, Result};
use reqwest::Client;
use serde::Deserialize;
use std::path::Path;
use tokio::fs;

#[derive(Debug)]
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
}

fn get_gh_token() -> Result<String> {
    let output = std::process::Command::new("gh")
        .args(["auth", "token"])
        .output()
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
    let token = get_gh_token()?;

    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::AUTHORIZATION,
        format!("Bearer {}", token)
            .parse()
            .context("Invalid GitHub token format")?,
    );

    let client = Client::builder()
        .user_agent("skill-installer")
        .default_headers(headers)
        .build()
        .context("Failed to create HTTP client")?;

    download_directory(&client, github_path, dest).await
}

async fn download_directory(client: &Client, github_path: &GitHubPath, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)
        .await
        .with_context(|| format!("Failed to create directory '{}'", dest.display()))?;

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

    let contents: Vec<GitHubContent> = response
        .json()
        .await
        .context("Failed to parse GitHub API response")?;

    for item in contents {
        let item_dest = dest.join(&item.name);

        if item.content_type == "dir" {
            let sub_path = GitHubPath {
                owner: github_path.owner.clone(),
                repo: github_path.repo.clone(),
                branch: github_path.branch.clone(),
                path: item.path,
            };
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
