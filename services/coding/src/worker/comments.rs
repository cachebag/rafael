use tracing::warn;

use crate::{
    github::{GitHubClient, InstallationToken},
    types::RepoRef,
};

pub(super) fn publish_completed_comment(pr_url: &str, commit_sha: &str, created: bool) -> String {
    let action = if created { "Created" } else { "Updated" };
    format!("{action} pull request: {pr_url}\n\nCommit: `{commit_sha}`")
}

pub(super) fn verification_passed_comment(
    branch_name: &str,
    changed_files: &[String],
    summary: &str,
) -> String {
    format!(
        "Prepared branch `{branch_name}`, completed bounded local code changes, and verification passed.\n\nChanged files:\n{}\n\nVerification:\n{summary}\n\nPublishing will continue automatically.",
        changed_files_list(changed_files)
    )
}

pub(super) fn verification_passed_after_repair_comment(
    branch_name: &str,
    changed_files: &[String],
    summary: &str,
) -> String {
    format!(
        "Prepared branch `{branch_name}`, repaired verification failures once, and verification passed on rerun.\n\nChanged files:\n{}\n\nVerification rerun:\n{summary}\n\nPublishing will continue automatically.",
        changed_files_list(changed_files)
    )
}

pub(super) fn verification_skipped_comment(branch_name: &str, changed_files: &[String]) -> String {
    format!(
        "Prepared branch `{branch_name}` and completed bounded local code changes.\n\nChanged files:\n{}\n\nVerification was skipped because no commands were selected. Publishing is disabled unless unverified publishing is explicitly allowed.",
        changed_files_list(changed_files)
    )
}

pub(super) fn verification_failed_comment(branch_name: &str, summary: &str) -> String {
    format!(
        "Prepared branch `{branch_name}`, but verification failed.\n\nSummary:\n{summary}\n\nCommand logs were saved under the run's `verification/` artifact directory."
    )
}

pub(super) fn implementation_blocked_comment(
    branch_name: &str,
    summary: &str,
    question: Option<&str>,
) -> String {
    let question = question
        .map(|question| format!("\n\nQuestion: {question}"))
        .unwrap_or_default();

    format!(
        "Prepared branch `{branch_name}`, but the bounded implementation loop is blocked.\n\nReason: {summary}{question}"
    )
}

pub(super) async fn post_comment_best_effort(
    github: &GitHubClient,
    token: &InstallationToken,
    repo: &RepoRef,
    issue_number: u64,
    body: &str,
) {
    if let Err(err) = github
        .post_issue_comment(token, repo, issue_number, body)
        .await
    {
        warn!(repo = %repo, issue = issue_number, error = %err, "failed to post issue comment");
    }
}

fn changed_files_list(changed_files: &[String]) -> String {
    if changed_files.is_empty() {
        "(none reported)".to_owned()
    } else {
        changed_files
            .iter()
            .map(|path| format!("- `{path}`"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}
