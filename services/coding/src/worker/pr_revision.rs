use serde::Serialize;
use tracing::warn;

use crate::{
    config::AppConfig,
    github::{
        GitHubClient, InstallationToken, IssueComment, PullRequestDetails, PullRequestFile,
        PullRequestReview, PullRequestReviewComment,
    },
    types::RepoRef,
};

#[derive(Debug, Serialize)]
pub(super) struct PullRequestFeedback {
    reviews: Vec<PullRequestReview>,
    review_comments: Vec<PullRequestReviewComment>,
    conversation_comments: Vec<IssueComment>,
}

#[derive(Debug, Serialize)]
pub(super) struct PullRequestState {
    number: u64,
    title: String,
    body: Option<String>,
    html_url: String,
    base_ref: String,
    head_ref: String,
    head_sha: String,
    changed_files: Vec<PullRequestStateFile>,
}

impl PullRequestState {
    pub(super) fn from_github(
        pull_request: &PullRequestDetails,
        files: Vec<PullRequestFile>,
    ) -> Self {
        Self {
            number: pull_request.number,
            title: pull_request.title.clone(),
            body: pull_request.body.clone(),
            html_url: pull_request.html_url.clone(),
            base_ref: pull_request.base.ref_name.clone(),
            head_ref: pull_request.head.ref_name.clone(),
            head_sha: pull_request.head.sha.clone(),
            changed_files: files
                .into_iter()
                .take(MAX_PULL_REQUEST_FILES)
                .map(PullRequestStateFile::from_github)
                .collect(),
        }
    }
}

#[derive(Debug, Serialize)]
struct PullRequestStateFile {
    path: String,
    status: String,
    additions: u64,
    deletions: u64,
    changes: u64,
    patch: Option<String>,
    patch_truncated: bool,
}

impl PullRequestStateFile {
    fn from_github(file: PullRequestFile) -> Self {
        let (patch, patch_truncated) = truncate_optional_patch(file.patch);
        Self {
            path: file.filename,
            status: file.status,
            additions: file.additions,
            deletions: file.deletions,
            changes: file.changes,
            patch,
            patch_truncated,
        }
    }
}

pub(super) async fn collect_pull_request_feedback(
    config: &AppConfig,
    github: &GitHubClient,
    token: &InstallationToken,
    repo: &RepoRef,
    pull_number: u64,
    issue_comments: Vec<IssueComment>,
) -> anyhow::Result<PullRequestFeedback> {
    let reviews = github
        .pull_request_reviews(token, repo, pull_number)
        .await?;
    let review_comments = match github
        .unresolved_pull_request_review_comments(token, repo, pull_number)
        .await
    {
        Ok(comments) => comments,
        Err(err) => {
            warn!(
                repo = %repo,
                pull = pull_number,
                error = %err,
                "failed to fetch unresolved pull request review threads; falling back to review comments"
            );
            github
                .pull_request_review_comments(token, repo, pull_number)
                .await?
        }
    };

    Ok(PullRequestFeedback {
        reviews: reviews
            .into_iter()
            .rev()
            .filter(|review| config.github.is_trusted_user(&review.user.login))
            .filter(|review| !review.state.eq_ignore_ascii_case("approved"))
            .filter(|review| {
                review
                    .body
                    .as_deref()
                    .is_some_and(|body| !body.trim().is_empty())
            })
            .take(MAX_FEEDBACK_ITEMS)
            .collect(),
        review_comments: review_comments
            .into_iter()
            .rev()
            .filter(|comment| config.github.is_trusted_user(&comment.user.login))
            .filter(|comment| !comment.body.trim().is_empty())
            .take(MAX_FEEDBACK_ITEMS)
            .collect(),
        conversation_comments: issue_comments
            .into_iter()
            .rev()
            .filter(|comment| config.github.is_trusted_user(&comment.user.login))
            .filter(|comment| !comment.body.trim().is_empty())
            .take(MAX_FEEDBACK_ITEMS)
            .collect(),
    })
}

pub(super) fn pull_request_revision_plan(
    state: &PullRequestState,
    feedback: &PullRequestFeedback,
) -> String {
    format!(
        "Revise existing pull request #{} on branch `{}`.\n\nDo not create a new branch or a new pull request. The current GitHub PR state below is the source of truth at the start of this run; do not infer PR state from previous failed local run artifacts. Address the trusted unresolved PR review feedback against that PR state, keep the change scoped, and update the existing branch. If feedback asks to revert a file, remove that file's changes from the PR diff. If the feedback is missing, ambiguous, or conflicts with repository safety, return done with status blocked and ask a concrete question.\n\nPull request title:\n{}\n\nPull request body:\n{}\n\nCurrent GitHub PR state:\n{}\n\nTrusted unresolved review feedback:\n{}",
        state.number,
        state.head_ref,
        state.title,
        truncate_feedback_text(state.body.as_deref().unwrap_or("(no pull request body)")),
        pull_request_state_for_plan(state),
        feedback_for_plan(feedback)
    )
}

fn pull_request_state_for_plan(state: &PullRequestState) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "- Base: `{}`; head: `{}` at `{}`",
        state.base_ref, state.head_ref, state.head_sha
    ));
    if state.changed_files.is_empty() {
        lines.push("- Changed files: (none reported by GitHub)".to_owned());
        return lines.join("\n");
    }

    lines.push("- Changed files:".to_owned());
    for file in &state.changed_files {
        lines.push(format!(
            "  - `{}` status={} +{} -{} changes={}",
            file.path, file.status, file.additions, file.deletions, file.changes
        ));
        if let Some(patch) = file.patch.as_deref() {
            lines.push(format!("    Patch:\n{}", indent_patch_for_plan(patch)));
            if file.patch_truncated {
                lines.push("    [patch truncated]".to_owned());
            }
        }
    }

    lines.join("\n")
}

fn feedback_for_plan(feedback: &PullRequestFeedback) -> String {
    let mut lines = Vec::new();

    for review in &feedback.reviews {
        let body = review
            .body
            .as_deref()
            .map(truncate_feedback_text)
            .filter(|body| !body.trim().is_empty())
            .unwrap_or_else(|| "(no review body)".to_owned());
        lines.push(format!(
            "- Review by `{}` with state `{}`: {}",
            review.user.login, review.state, body
        ));
    }

    for comment in &feedback.review_comments {
        let line = comment
            .line
            .or(comment.original_line)
            .map(|line| format!(":{line}"))
            .unwrap_or_default();
        lines.push(format!(
            "- Inline comment by `{}` on `{}`{}: {}",
            comment.user.login,
            comment.path,
            line,
            truncate_feedback_text(&comment.body)
        ));
    }

    for comment in &feedback.conversation_comments {
        lines.push(format!(
            "- PR conversation comment by `{}`: {}",
            comment.user.login,
            truncate_feedback_text(&comment.body)
        ));
    }

    if lines.is_empty() {
        "(no trusted PR review feedback found)".to_owned()
    } else {
        lines.join("\n")
    }
}

fn truncate_feedback_text(value: &str) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= MAX_FEEDBACK_CHARS {
        compact
    } else {
        let mut truncated = compact.chars().take(MAX_FEEDBACK_CHARS).collect::<String>();
        truncated.push_str("...");
        truncated
    }
}

fn truncate_optional_patch(value: Option<String>) -> (Option<String>, bool) {
    let Some(value) = value else {
        return (None, false);
    };
    if value.chars().count() <= MAX_PULL_REQUEST_PATCH_CHARS {
        return (Some(value), false);
    }

    (
        Some(value.chars().take(MAX_PULL_REQUEST_PATCH_CHARS).collect()),
        true,
    )
}

fn indent_patch_for_plan(patch: &str) -> String {
    patch
        .lines()
        .map(|line| format!("    {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

const MAX_FEEDBACK_ITEMS: usize = 30;
const MAX_FEEDBACK_CHARS: usize = 2_000;
const MAX_PULL_REQUEST_FILES: usize = 40;
const MAX_PULL_REQUEST_PATCH_CHARS: usize = 4_000;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revision_plan_includes_current_pull_request_state() {
        let state = PullRequestState {
            number: 12,
            title: "services/coding: operations setup".to_owned(),
            body: Some("Issue: https://example.test/issues/10".to_owned()),
            html_url: "https://example.test/pulls/12".to_owned(),
            base_ref: "master".to_owned(),
            head_ref: "work/issue-10-services-coding-operations-setup".to_owned(),
            head_sha: "abc123".to_owned(),
            changed_files: vec![PullRequestStateFile {
                path: "README.md".to_owned(),
                status: "modified".to_owned(),
                additions: 22,
                deletions: 0,
                changes: 22,
                patch: Some("@@ -1 +1 @@\n+Systemd docs".to_owned()),
                patch_truncated: false,
            }],
        };
        let feedback = PullRequestFeedback {
            reviews: Vec::new(),
            review_comments: Vec::new(),
            conversation_comments: Vec::new(),
        };

        let plan = pull_request_revision_plan(&state, &feedback);

        assert!(plan.contains("Current GitHub PR state"));
        assert!(
            plan.contains("Base: `master`; head: `work/issue-10-services-coding-operations-setup`")
        );
        assert!(plan.contains("`README.md` status=modified +22 -0 changes=22"));
        assert!(plan.contains("+Systemd docs"));
    }
}
