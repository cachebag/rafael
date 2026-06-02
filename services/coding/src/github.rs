use std::{fs, path::Path};

use anyhow::{Context, bail};
use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};

use crate::{config::GitHubConfig, types::RepoRef};

#[derive(Clone)]
pub struct GitHubClient {
    http: Client,
    api_base_url: String,
}

#[derive(Debug, Clone)]
pub struct InstallationToken {
    pub token: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RepositoryInfo {
    pub full_name: String,
    pub default_branch: String,
    pub clone_url: String,
    pub html_url: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IssueInfo {
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub html_url: String,
    pub state: String,
    #[serde(default)]
    pub labels: Vec<IssueLabel>,
    #[serde(default)]
    pub pull_request: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IssueLabel {
    pub name: String,
}

impl GitHubClient {
    pub fn new(config: &GitHubConfig) -> anyhow::Result<Self> {
        Ok(Self {
            http: Client::builder()
                .user_agent(format!("rafael-coding/{}", env!("CARGO_PKG_VERSION")))
                .build()
                .context("failed to build GitHub HTTP client")?,
            api_base_url: config.api_base_url.trim_end_matches('/').to_owned(),
        })
    }

    pub async fn create_installation_token(
        &self,
        config: &GitHubConfig,
        installation_id: u64,
        repo: &RepoRef,
    ) -> anyhow::Result<InstallationToken> {
        let jwt = create_app_jwt(config.app_id, &config.private_key_path)?;
        let url = format!(
            "{}/app/installations/{installation_id}/access_tokens",
            self.api_base_url
        );

        let response = self
            .http
            .post(url)
            .bearer_auth(jwt)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
            .json(&CreateInstallationTokenRequest {
                repositories: vec![repo.name.clone()],
                permissions: InstallationTokenPermissions {
                    contents: "write",
                    issues: "write",
                    pull_requests: "write",
                },
            })
            .send()
            .await
            .context("failed to request GitHub App installation token")?;

        if response.status() == StatusCode::UNAUTHORIZED {
            bail!("GitHub App authentication failed; check app id and private key");
        }

        let response = response
            .error_for_status()
            .context("GitHub installation token request failed")?;
        let token = response
            .json::<CreateInstallationTokenResponse>()
            .await
            .context("failed to parse GitHub installation token response")?;

        Ok(InstallationToken {
            token: token.token,
            expires_at: token.expires_at,
        })
    }

    pub async fn repository(
        &self,
        token: &InstallationToken,
        repo: &RepoRef,
    ) -> anyhow::Result<RepositoryInfo> {
        let url = format!("{}/repos/{}", self.api_base_url, repo);
        self.get_json(token, url, "repository").await
    }

    pub async fn issue(
        &self,
        token: &InstallationToken,
        repo: &RepoRef,
        issue_number: u64,
    ) -> anyhow::Result<IssueInfo> {
        let url = format!("{}/repos/{repo}/issues/{issue_number}", self.api_base_url);
        self.get_json(token, url, "issue").await
    }

    pub async fn post_issue_comment(
        &self,
        token: &InstallationToken,
        repo: &RepoRef,
        issue_number: u64,
        body: &str,
    ) -> anyhow::Result<()> {
        let url = format!(
            "{}/repos/{repo}/issues/{issue_number}/comments",
            self.api_base_url
        );

        self.http
            .post(url)
            .bearer_auth(&token.token)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
            .json(&CreateIssueCommentRequest { body })
            .send()
            .await
            .context("failed to post GitHub issue comment")?
            .error_for_status()
            .context("GitHub issue comment request failed")?;

        Ok(())
    }

    async fn get_json<T>(
        &self,
        token: &InstallationToken,
        url: String,
        resource_name: &str,
    ) -> anyhow::Result<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        self.http
            .get(url)
            .bearer_auth(&token.token)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
            .send()
            .await
            .with_context(|| format!("failed to fetch GitHub {resource_name}"))?
            .error_for_status()
            .with_context(|| format!("GitHub {resource_name} request failed"))?
            .json::<T>()
            .await
            .with_context(|| format!("failed to parse GitHub {resource_name} response"))
    }
}

fn create_app_jwt(app_id: u64, private_key_path: &Path) -> anyhow::Result<String> {
    let private_key = fs::read(private_key_path)
        .with_context(|| format!("failed to read {}", private_key_path.display()))?;
    let now = Utc::now().timestamp();

    encode(
        &Header::new(Algorithm::RS256),
        &AppClaims {
            iat: now - 60,
            exp: now + 540,
            iss: app_id.to_string(),
        },
        &EncodingKey::from_rsa_pem(&private_key).context("invalid GitHub App private key")?,
    )
    .context("failed to sign GitHub App JWT")
}

#[derive(Debug, Serialize)]
struct AppClaims {
    iat: i64,
    exp: i64,
    iss: String,
}

#[derive(Debug, Serialize)]
struct CreateInstallationTokenRequest {
    repositories: Vec<String>,
    permissions: InstallationTokenPermissions,
}

#[derive(Debug, Serialize)]
struct InstallationTokenPermissions {
    contents: &'static str,
    issues: &'static str,
    pull_requests: &'static str,
}

#[derive(Debug, Deserialize)]
struct CreateInstallationTokenResponse {
    token: String,
    expires_at: String,
}

#[derive(Debug, Serialize)]
struct CreateIssueCommentRequest<'a> {
    body: &'a str,
}

const GITHUB_API_VERSION: &str = "2026-03-10";
