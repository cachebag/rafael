use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use anyhow::{Context, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use tokio::{process::Command, time::timeout};
use tracing::warn;

use crate::{
    config::AppConfig,
    github::{InstallationToken, IssueInfo, RepositoryInfo},
    types::RepoRef,
};

use super::paths::safe_path_component;

pub(super) async fn prepare_worktree(
    config: &AppConfig,
    token: &InstallationToken,
    repo: &RepoRef,
    repo_info: &RepositoryInfo,
    issue_number: u64,
    run_id: &str,
    branch_name: &str,
) -> anyhow::Result<PathBuf> {
    let repo_dir = isolated_checkout_path(config, repo, issue_number, run_id);
    let parent = repo_dir
        .parent()
        .context("isolated checkout path must have a parent directory")?;
    tokio::fs::create_dir_all(parent)
        .await
        .with_context(|| format!("failed to create {}", parent.display()))?;

    if !repo_dir.starts_with(&config.workspace.workdir) {
        bail!("computed repo workdir escaped configured workdir");
    }

    if repo_dir.join(".git").exists() {
        run_git(
            &repo_dir,
            token,
            vec![
                "fetch".to_owned(),
                "origin".to_owned(),
                repo_info.default_branch.clone(),
                "--prune".to_owned(),
            ],
        )
        .await?;
    } else {
        run_git_clone(&config.workspace.workdir, token, repo, &repo_dir).await?;
    }

    run_git(
        &repo_dir,
        token,
        vec![
            "switch".to_owned(),
            "-C".to_owned(),
            branch_name.to_owned(),
            format!("origin/{}", repo_info.default_branch),
        ],
    )
    .await?;

    Ok(repo_dir)
}

pub(super) async fn prepare_pull_request_worktree(
    config: &AppConfig,
    token: &InstallationToken,
    repo: &RepoRef,
    pull_number: u64,
    run_id: &str,
    branch_name: &str,
) -> anyhow::Result<PathBuf> {
    if !is_safe_branch_name(branch_name) {
        bail!("refusing to check out unsafe pull request branch `{branch_name}`");
    }

    let repo_dir = pull_request_checkout_path(config, repo, pull_number, run_id);
    let parent = repo_dir
        .parent()
        .context("pull request checkout path must have a parent directory")?;
    tokio::fs::create_dir_all(parent)
        .await
        .with_context(|| format!("failed to create {}", parent.display()))?;

    if !repo_dir.starts_with(&config.workspace.workdir) {
        bail!("computed pull request workdir escaped configured workdir");
    }

    if !repo_dir.join(".git").exists() {
        run_git_clone(&config.workspace.workdir, token, repo, &repo_dir).await?;
    }

    run_git(
        &repo_dir,
        token,
        vec![
            "fetch".to_owned(),
            "origin".to_owned(),
            format!("+refs/heads/{branch_name}:refs/remotes/origin/{branch_name}"),
        ],
    )
    .await?;
    run_git(
        &repo_dir,
        token,
        vec![
            "switch".to_owned(),
            "-C".to_owned(),
            branch_name.to_owned(),
            format!("origin/{branch_name}"),
        ],
    )
    .await?;

    Ok(repo_dir)
}

pub(super) fn branch_name_for_issue(issue: &IssueInfo) -> String {
    let change_type = change_type_for_issue(issue);
    let slug = slugify(&issue.title);
    format!("{change_type}/issue-{}-{slug}", issue.number)
}

fn isolated_checkout_path(
    config: &AppConfig,
    repo: &RepoRef,
    issue_number: u64,
    run_id: &str,
) -> PathBuf {
    config
        .workspace
        .workdir
        .join(repo.safe_dir_name())
        .join(format!("issue-{issue_number}"))
        .join(safe_path_component(run_id))
        .join("checkout")
}

fn pull_request_checkout_path(
    config: &AppConfig,
    repo: &RepoRef,
    pull_number: u64,
    run_id: &str,
) -> PathBuf {
    config
        .workspace
        .workdir
        .join(repo.safe_dir_name())
        .join(format!("pr-{pull_number}"))
        .join(safe_path_component(run_id))
        .join("checkout")
}

async fn run_git_clone(
    cwd: &Path,
    token: &InstallationToken,
    repo: &RepoRef,
    repo_dir: &Path,
) -> anyhow::Result<()> {
    let url = format!("https://github.com/{repo}.git");
    run_git_at(
        cwd,
        token,
        vec![
            "clone".to_owned(),
            url,
            repo_dir.to_string_lossy().into_owned(),
        ],
    )
    .await
}

async fn run_git(cwd: &Path, token: &InstallationToken, args: Vec<String>) -> anyhow::Result<()> {
    run_git_at(cwd, token, args).await
}

async fn run_git_at(
    cwd: &Path,
    token: &InstallationToken,
    args: Vec<String>,
) -> anyhow::Result<()> {
    let mut command = Command::new("git");
    let auth_header = git_auth_header(&token.token);
    command
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .arg("-c")
        .arg(format!(
            "http.https://github.com/.extraheader={auth_header}"
        ))
        .args(&args);

    let output = timeout(Duration::from_secs(300), command.output())
        .await
        .context("git command timed out")?
        .context("failed to execute git")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        warn!(?args, status = ?output.status, stderr = %stderr, "git command failed");
        bail!("git command failed with status {}", output.status);
    }

    Ok(())
}

fn git_auth_header(token: &str) -> String {
    let encoded = STANDARD.encode(format!("x-access-token:{token}"));
    format!("AUTHORIZATION: basic {encoded}")
}

fn change_type_for_issue(issue: &IssueInfo) -> &'static str {
    let labels = issue
        .labels
        .iter()
        .map(|label| label.name.to_ascii_lowercase())
        .collect::<Vec<_>>();

    if labels.iter().any(|label| label == "bug" || label == "fix") {
        "fix"
    } else if labels
        .iter()
        .any(|label| label == "documentation" || label == "docs")
    {
        "docs"
    } else if labels
        .iter()
        .any(|label| label == "feature" || label == "enhancement" || label == "feat")
    {
        "feat"
    } else {
        "work"
    }
}

fn slugify(value: &str) -> String {
    let mut slug = String::new();
    let mut last_was_dash = false;

    for ch in value.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            last_was_dash = false;
        } else if !last_was_dash && !slug.is_empty() {
            slug.push('-');
            last_was_dash = true;
        }

        if slug.len() >= 48 {
            break;
        }
    }

    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "issue".to_owned()
    } else {
        slug.to_owned()
    }
}

fn is_safe_branch_name(branch_name: &str) -> bool {
    !branch_name.is_empty()
        && !branch_name.starts_with('-')
        && !branch_name.contains("..")
        && !branch_name.contains('\\')
        && !branch_name.contains(' ')
        && !branch_name.contains('\n')
        && !branch_name.contains('\r')
        && !branch_name.contains('\0')
        && !branch_name.ends_with('/')
        && !branch_name.ends_with(".lock")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::IssueLabel;

    #[test]
    fn branch_name_uses_type_and_slug() {
        let issue = IssueInfo {
            number: 12,
            title: "Add GitHub App webhook receiver!".to_owned(),
            body: None,
            html_url: "https://example.test".to_owned(),
            state: "open".to_owned(),
            labels: vec![IssueLabel {
                name: "feature".to_owned(),
            }],
            pull_request: None,
        };

        assert_eq!(
            branch_name_for_issue(&issue),
            "feat/issue-12-add-github-app-webhook-receiver"
        );
    }

    #[test]
    fn slugify_falls_back_for_empty_title() {
        assert_eq!(slugify("?!"), "issue");
    }
}
