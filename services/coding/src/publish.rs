use std::{path::Path, process::Stdio, time::Duration};

use anyhow::{Context, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use tokio::{process::Command, time::timeout};
use tracing::warn;

use crate::{
    config::AppConfig,
    github::{GitHubClient, InstallationToken, IssueInfo, RepositoryInfo},
    types::RepoRef,
    verification::VerificationStatus,
};

pub struct PublishCommitRequest<'a> {
    pub config: &'a AppConfig,
    pub repo: &'a RepositoryInfo,
    pub issue: &'a IssueInfo,
    pub branch_name: &'a str,
    pub checkout_path: &'a Path,
    pub verification_status: Option<VerificationStatus>,
}

pub async fn prepare_publish_commit(
    request: PublishCommitRequest<'_>,
) -> anyhow::Result<PublishCommit> {
    ensure_publish_preconditions(&request).await?;
    configure_git_author(request.config, request.checkout_path).await?;

    let commit_message = commit_message_for_issue(request.branch_name, request.issue);
    run_git(
        request.checkout_path,
        vec!["add".to_owned(), "-A".to_owned()],
    )
    .await?;
    run_git(
        request.checkout_path,
        vec![
            "commit".to_owned(),
            "-m".to_owned(),
            commit_message.subject.clone(),
            "-m".to_owned(),
            commit_message.footer.clone(),
        ],
    )
    .await?;
    let commit_sha = run_git_capture(
        request.checkout_path,
        vec!["rev-parse".to_owned(), "HEAD".to_owned()],
    )
    .await?
    .trim()
    .to_owned();

    Ok(PublishCommit {
        commit_sha,
        message: commit_message.full(),
    })
}

pub struct PublishPullRequestRequest<'a> {
    pub github: &'a GitHubClient,
    pub token: &'a InstallationToken,
    pub repo_ref: &'a RepoRef,
    pub repo: &'a RepositoryInfo,
    pub issue: &'a IssueInfo,
    pub branch_name: &'a str,
    pub checkout_path: &'a Path,
    pub commit_sha: &'a str,
    pub implementation_summary: Option<&'a str>,
    pub run_id: &'a str,
}

pub async fn publish_pull_request(
    request: PublishPullRequestRequest<'_>,
) -> anyhow::Result<PublishPullRequestOutcome> {
    push_branch(request.checkout_path, request.token, request.branch_name).await?;

    let title = pull_request_title(request.issue);
    let body = pull_request_body(request.issue, request.implementation_summary);

    if let Some(existing) = request
        .github
        .open_pull_request_for_branch(
            request.token,
            request.repo_ref,
            &request.repo_ref.owner,
            request.branch_name,
        )
        .await?
    {
        let updated = request
            .github
            .update_pull_request(
                request.token,
                request.repo_ref,
                existing.number,
                &title,
                &body,
            )
            .await?;
        request
            .github
            .post_issue_comment(
                request.token,
                request.repo_ref,
                existing.number,
                &format!(
                    "Updated this pull request from run `{}` at commit `{}`.",
                    request.run_id, request.commit_sha
                ),
            )
            .await?;
        return Ok(PublishPullRequestOutcome {
            pr_number: updated.number,
            pr_url: updated.html_url,
            created: false,
        });
    }

    let created = request
        .github
        .create_pull_request(
            request.token,
            request.repo_ref,
            &title,
            request.branch_name,
            &request.repo.default_branch,
            &body,
        )
        .await?;

    Ok(PublishPullRequestOutcome {
        pr_number: created.number,
        pr_url: created.html_url,
        created: true,
    })
}

async fn ensure_publish_preconditions(request: &PublishCommitRequest<'_>) -> anyhow::Result<()> {
    if request.branch_name == request.repo.default_branch {
        bail!(
            "refusing to publish from default branch `{}`",
            request.branch_name
        );
    }
    if !is_safe_branch_name(request.branch_name) {
        bail!(
            "refusing to publish unsafe branch name `{}`",
            request.branch_name
        );
    }
    if !verification_allows_publish(request.config, request.verification_status) {
        bail!("refusing to publish because verification did not pass");
    }
    if !working_tree_has_changes(request.checkout_path).await? {
        bail!("refusing to publish because the working tree has no changes");
    }

    Ok(())
}

async fn configure_git_author(config: &AppConfig, checkout_path: &Path) -> anyhow::Result<()> {
    let name = clean_git_identity(&config.github.git_author_name, "git author name")?;
    let email = clean_git_identity(&config.github.git_author_email, "git author email")?;

    run_git(
        checkout_path,
        vec!["config".to_owned(), "user.name".to_owned(), name],
    )
    .await?;
    run_git(
        checkout_path,
        vec!["config".to_owned(), "user.email".to_owned(), email],
    )
    .await
}

async fn working_tree_has_changes(checkout_path: &Path) -> anyhow::Result<bool> {
    let status = run_git_capture(
        checkout_path,
        vec![
            "status".to_owned(),
            "--porcelain".to_owned(),
            "--untracked-files=normal".to_owned(),
        ],
    )
    .await?;
    Ok(!status.trim().is_empty())
}

async fn push_branch(
    checkout_path: &Path,
    token: &InstallationToken,
    branch_name: &str,
) -> anyhow::Result<()> {
    let remote_ref = format!("refs/heads/{branch_name}");
    let remote_branch_exists = fetch_remote_branch(checkout_path, token, branch_name).await?;

    let mut args = Vec::new();
    args.push("push".to_owned());
    if remote_branch_exists {
        args.push(format!("--force-with-lease={remote_ref}"));
    } else {
        args.push("-u".to_owned());
    }
    args.push("origin".to_owned());
    args.push(format!("HEAD:{remote_ref}"));

    run_git_authenticated(checkout_path, token, args).await
}

async fn fetch_remote_branch(
    checkout_path: &Path,
    token: &InstallationToken,
    branch_name: &str,
) -> anyhow::Result<bool> {
    let remote_ref = format!("refs/heads/{branch_name}");
    let tracking_ref = format!("refs/remotes/origin/{branch_name}");
    let output = run_git_authenticated_result(
        checkout_path,
        token,
        vec![
            "fetch".to_owned(),
            "origin".to_owned(),
            format!("+{remote_ref}:{tracking_ref}"),
        ],
    )
    .await?;

    Ok(output.status.success())
}

async fn run_git(checkout_path: &Path, args: Vec<String>) -> anyhow::Result<()> {
    let output = run_git_result(checkout_path, args).await?;
    ensure_git_success(output, "git command failed")
}

async fn run_git_capture(checkout_path: &Path, args: Vec<String>) -> anyhow::Result<String> {
    let output = run_git_result(checkout_path, args).await?;
    ensure_git_success_with_output(output, "git command failed")
}

async fn run_git_authenticated(
    checkout_path: &Path,
    token: &InstallationToken,
    args: Vec<String>,
) -> anyhow::Result<()> {
    let output = run_git_authenticated_result(checkout_path, token, args).await?;
    ensure_git_success(output, "authenticated git command failed")
}

async fn run_git_result(
    checkout_path: &Path,
    args: Vec<String>,
) -> anyhow::Result<std::process::Output> {
    let mut command = Command::new("git");
    command
        .current_dir(checkout_path)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .args(&args);
    run_command(command, args).await
}

async fn run_git_authenticated_result(
    checkout_path: &Path,
    token: &InstallationToken,
    args: Vec<String>,
) -> anyhow::Result<std::process::Output> {
    let mut command = Command::new("git");
    let auth_header = git_auth_header(&token.token);
    command
        .current_dir(checkout_path)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .arg("-c")
        .arg(format!(
            "http.https://github.com/.extraheader={auth_header}"
        ))
        .args(&args);
    run_command(command, args).await
}

async fn run_command(
    mut command: Command,
    args_for_log: Vec<String>,
) -> anyhow::Result<std::process::Output> {
    timeout(
        Duration::from_secs(GIT_COMMAND_TIMEOUT_SECS),
        command.output(),
    )
    .await
    .context("git command timed out")?
    .with_context(|| format!("failed to execute git with args {args_for_log:?}"))
}

fn ensure_git_success(output: std::process::Output, message: &str) -> anyhow::Result<()> {
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    warn!(status = ?output.status, stderr = %stderr, message = %message, "git command failed");
    bail!("{message} with status {}", output.status)
}

fn ensure_git_success_with_output(
    output: std::process::Output,
    message: &str,
) -> anyhow::Result<String> {
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).to_string());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    warn!(status = ?output.status, stderr = %stderr, message = %message, "git command failed");
    bail!("{message} with status {}", output.status)
}

fn verification_allows_publish(
    config: &AppConfig,
    verification_status: Option<VerificationStatus>,
) -> bool {
    matches!(verification_status, Some(VerificationStatus::Passed))
        || (config.workspace.allow_unverified_publish
            && matches!(
                verification_status,
                None | Some(VerificationStatus::Skipped)
            ))
}

fn commit_message_for_issue(branch_name: &str, issue: &IssueInfo) -> CommitMessage {
    let prefix = conventional_prefix_for_branch(branch_name);
    let title = single_line(&issue.title);
    CommitMessage {
        subject: format!("{prefix}: {title}"),
        footer: format!("Refs #{}", issue.number),
    }
}

fn conventional_prefix_for_branch(branch_name: &str) -> &'static str {
    match branch_name.split_once('/').map(|(kind, _)| kind) {
        Some("feat") => "feat",
        Some("fix") => "fix",
        Some("docs") => "docs",
        Some("test") => "test",
        Some("refactor") => "refactor",
        _ => "chore",
    }
}

fn pull_request_title(issue: &IssueInfo) -> String {
    single_line(&issue.title)
}

fn pull_request_body(issue: &IssueInfo, implementation_summary: Option<&str>) -> String {
    let summary = implementation_summary
        .map(single_line)
        .filter(|summary| !summary.is_empty())
        .unwrap_or_else(|| format!("This change addresses {}.", single_line(&issue.title)));

    format!("Issue: {}\n\nSummary: {}", issue.html_url, summary)
}

fn single_line(value: &str) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_chars(&compact, MAX_SUMMARY_CHARS)
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut truncated = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        truncated.push('…');
    }
    truncated
}

fn clean_git_identity(value: &str, name: &str) -> anyhow::Result<String> {
    let value = value.trim();
    if value.is_empty() || value.contains('\n') || value.contains('\r') || value.contains('\0') {
        bail!("{name} must be non-empty and single-line");
    }

    Ok(value.to_owned())
}

fn is_safe_branch_name(branch_name: &str) -> bool {
    !branch_name.is_empty()
        && !branch_name.starts_with('-')
        && !branch_name.starts_with('/')
        && !branch_name.ends_with('/')
        && !branch_name.contains("..")
        && !branch_name.contains('@')
        && !branch_name.contains('\\')
        && !branch_name.contains(char::is_whitespace)
        && !branch_name.chars().any(char::is_control)
}

fn git_auth_header(token: &str) -> String {
    let encoded = STANDARD.encode(format!("x-access-token:{token}"));
    format!("AUTHORIZATION: basic {encoded}")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishCommit {
    pub commit_sha: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishPullRequestOutcome {
    pub pr_number: u64,
    pub pr_url: String,
    pub created: bool,
}

#[derive(Debug, Clone)]
struct CommitMessage {
    subject: String,
    footer: String,
}

impl CommitMessage {
    fn full(&self) -> String {
        format!("{}\n\n{}", self.subject, self.footer)
    }
}

const GIT_COMMAND_TIMEOUT_SECS: u64 = 300;
const MAX_SUMMARY_CHARS: usize = 240;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_message_uses_conventional_prefix_and_refs_footer() {
        let issue = test_issue("Add publish support");

        let message = commit_message_for_issue("feat/add-publish-support", &issue);

        assert_eq!(message.subject, "feat: Add publish support");
        assert_eq!(message.footer, "Refs #42");
    }

    #[test]
    fn work_branch_uses_chore_prefix() {
        let issue = test_issue("Do maintenance");

        let message = commit_message_for_issue("work/do-maintenance", &issue);

        assert_eq!(message.subject, "chore: Do maintenance");
    }

    #[test]
    fn pr_body_includes_issue_link_and_plain_summary() {
        let issue = test_issue("Add publish support");

        let body = pull_request_body(&issue, Some("Changed the worker flow."));

        assert!(body.contains("Issue: https://example.test/issues/42"));
        assert!(body.contains("Summary: Changed the worker flow."));
        assert!(!body.contains("\n- "));
    }

    #[test]
    fn unsafe_branch_names_are_rejected() {
        assert!(is_safe_branch_name("feat/add-publish-support"));
        assert!(!is_safe_branch_name("main..evil"));
        assert!(!is_safe_branch_name("-bad"));
        assert!(!is_safe_branch_name("bad branch"));
    }

    fn test_issue(title: &str) -> IssueInfo {
        IssueInfo {
            number: 42,
            title: title.to_owned(),
            body: None,
            html_url: "https://example.test/issues/42".to_owned(),
            state: "open".to_owned(),
            labels: Vec::new(),
            pull_request: None,
        }
    }
}
