use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use anyhow::{Context, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use tokio::{process::Command, time::timeout};
use tracing::{info, warn};

use crate::{
    change_execution::{
        ChangeExecutionOutcome, ChangeExecutionRequest, ChangeExecutionStatus, execute_change_loop,
    },
    config::AppConfig,
    github::{GitHubClient, InstallationToken, IssueInfo, RepositoryInfo},
    model::ModelClient,
    repo_context::{RepoContext, collect_repository_context},
    types::{IssueTrigger, RepoRef, TriggerKind},
    verification::{
        VerificationCommandResult, VerificationOutcome, VerificationRequest, VerificationStatus,
        run_verification,
    },
};

#[derive(Debug, Serialize, Deserialize)]
pub struct RunState {
    pub run_id: String,
    pub repo: RepoRef,
    pub issue_number: u64,
    pub trigger: TriggerKind,
    pub actor: Option<String>,
    pub installation_id: u64,
    pub status: RunStatus,
    pub issue_title: Option<String>,
    pub issue_url: Option<String>,
    pub default_branch: Option<String>,
    pub branch_name: Option<String>,
    pub worktree_path: Option<PathBuf>,
    pub context_path: Option<PathBuf>,
    pub plan_path: Option<PathBuf>,
    pub transcript_path: Option<PathBuf>,
    pub diff_stat_path: Option<PathBuf>,
    pub changed_files: Vec<String>,
    pub implementation_summary: Option<String>,
    pub verification_path: Option<PathBuf>,
    pub verification_status: Option<VerificationStatus>,
    pub verification_summary: Option<String>,
    pub verification_results: Vec<VerificationCommandResult>,
    pub repair_attempted: bool,
    pub repair_summary: Option<String>,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Received,
    Authenticated,
    Prepared,
    Planned,
    Implementing,
    Verifying,
    Verified,
    Completed,
    Blocked,
    Failed,
}

pub async fn run_issue_triggered(config: AppConfig, trigger: IssueTrigger) -> anyhow::Result<()> {
    let max_run = Duration::from_secs(config.workspace.max_run_minutes * 60);
    timeout(max_run, run_issue_triggered_inner(config, trigger))
        .await
        .context("coding run exceeded configured time limit")?
}

async fn run_issue_triggered_inner(config: AppConfig, trigger: IssueTrigger) -> anyhow::Result<()> {
    let installation_id = trigger
        .installation_id
        .or(config.github.installation_id)
        .context("installation id must come from webhook payload, CLI, or config")?;
    let created_at = now_rfc3339();
    let run_dir = run_dir(&config, &trigger);

    tokio::fs::create_dir_all(&run_dir)
        .await
        .with_context(|| format!("failed to create {}", run_dir.display()))?;

    let mut state = RunState {
        run_id: trigger.run_id.clone(),
        repo: trigger.repo.clone(),
        issue_number: trigger.issue_number,
        trigger: trigger.trigger,
        actor: trigger.actor.clone(),
        installation_id,
        status: RunStatus::Received,
        issue_title: None,
        issue_url: None,
        default_branch: trigger.default_branch.clone(),
        branch_name: None,
        worktree_path: None,
        context_path: None,
        plan_path: None,
        transcript_path: None,
        diff_stat_path: None,
        changed_files: Vec::new(),
        implementation_summary: None,
        verification_path: None,
        verification_status: None,
        verification_summary: None,
        verification_results: Vec::new(),
        repair_attempted: false,
        repair_summary: None,
        error: None,
        created_at: created_at.clone(),
        updated_at: created_at,
    };
    write_state(&run_dir, &state).await?;

    let github = GitHubClient::new(&config.github)?;
    let model = ModelClient::new(&config.model)?;

    let token = github
        .create_installation_token(&config.github, installation_id, &trigger.repo)
        .await?;
    info!(
        expires_at = %token.expires_at,
        run_id = %trigger.run_id,
        "received GitHub App installation token"
    );
    state.status = RunStatus::Authenticated;
    state.updated_at = now_rfc3339();
    write_state(&run_dir, &state).await?;

    let repo = github.repository(&token, &trigger.repo).await?;
    let issue = github
        .issue(&token, &trigger.repo, trigger.issue_number)
        .await?;
    let issue_comments = github
        .issue_comments(&token, &trigger.repo, trigger.issue_number)
        .await?;

    if issue.pull_request.is_some() {
        bail!("issue #{} is a pull request, not an issue", issue.number);
    }

    post_comment_best_effort(
        &github,
        &token,
        &trigger.repo,
        trigger.issue_number,
        &format!(
            "Started a coding run for this issue.\n\nRun: `{}`\nTrigger: `{}`",
            trigger.run_id, trigger.trigger
        ),
    )
    .await;

    state.issue_title = Some(issue.title.clone());
    state.issue_url = Some(issue.html_url.clone());
    state.default_branch = Some(repo.default_branch.clone());

    let branch_name = branch_name_for_issue(&issue);
    let worktree_path =
        prepare_worktree(&config, &token, &trigger.repo, &repo, &branch_name).await?;

    state.status = RunStatus::Prepared;
    state.branch_name = Some(branch_name.clone());
    state.worktree_path = Some(worktree_path.clone());
    state.updated_at = now_rfc3339();
    write_state(&run_dir, &state).await?;

    let repo_context =
        collect_repository_context(&config, &repo, &issue, &issue_comments, &worktree_path).await?;
    let context_path = run_dir.join("context.json");
    let context_body =
        serde_json::to_vec_pretty(&repo_context).context("failed to serialize repo context")?;
    tokio::fs::write(&context_path, context_body)
        .await
        .with_context(|| format!("failed to write {}", context_path.display()))?;
    state.context_path = Some(context_path);
    state.updated_at = now_rfc3339();
    write_state(&run_dir, &state).await?;

    let plan = model
        .issue_plan(&repo, &issue, &branch_name, &repo_context)
        .await?;
    let plan_path = run_dir.join("plan.md");
    tokio::fs::write(&plan_path, &plan)
        .await
        .with_context(|| format!("failed to write {}", plan_path.display()))?;

    state.status = RunStatus::Planned;
    state.plan_path = Some(plan_path);
    state.updated_at = now_rfc3339();
    write_state(&run_dir, &state).await?;

    state.status = RunStatus::Implementing;
    state.transcript_path = Some(run_dir.join("transcript.jsonl"));
    state.updated_at = now_rfc3339();
    write_state(&run_dir, &state).await?;

    let outcome = match execute_change_loop(ChangeExecutionRequest {
        config: &config,
        model: &model,
        repo: &repo,
        issue: &issue,
        branch_name: &branch_name,
        repo_context: &repo_context,
        plan: &plan,
        checkout_path: &worktree_path,
        run_dir: &run_dir,
    })
    .await
    {
        Ok(outcome) => outcome,
        Err(err) => {
            state.status = RunStatus::Failed;
            state.error = Some(format!("change execution failed: {err}"));
            state.updated_at = now_rfc3339();
            write_state(&run_dir, &state).await?;
            return Err(err).context("change execution failed");
        }
    };

    state.transcript_path = Some(outcome.transcript_path.clone());
    state.diff_stat_path = outcome.diff_stat_path.clone();
    state.changed_files = outcome.changed_files.clone();
    state.implementation_summary = Some(outcome.summary.clone());
    state.updated_at = now_rfc3339();

    let run_context = WorkerRunContext {
        config: &config,
        github: &github,
        model: &model,
        token: &token,
        repo_ref: &trigger.repo,
        repo: &repo,
        issue: &issue,
        branch_name: &branch_name,
        repo_context: &repo_context,
        plan: &plan,
        worktree_path: &worktree_path,
        run_dir: &run_dir,
        issue_number: trigger.issue_number,
        run_id: &trigger.run_id,
    };

    if !handle_change_execution_outcome(&mut state, &run_context, &outcome).await? {
        return Ok(());
    }

    run_verification_phase(&mut state, &run_context).await
}

async fn handle_change_execution_outcome(
    state: &mut RunState,
    context: &WorkerRunContext<'_>,
    outcome: &ChangeExecutionOutcome,
) -> anyhow::Result<bool> {
    match outcome.status {
        ChangeExecutionStatus::Completed => Ok(true),
        ChangeExecutionStatus::Blocked => {
            state.status = RunStatus::Blocked;
            state.error = Some(outcome.summary.clone());
            state.updated_at = now_rfc3339();
            write_state(context.run_dir, state).await?;
            post_comment_best_effort(
                context.github,
                context.token,
                context.repo_ref,
                context.issue_number,
                &implementation_blocked_comment(
                    context.branch_name,
                    &outcome.summary,
                    outcome.question.as_deref(),
                ),
            )
            .await;
            Ok(false)
        }
        ChangeExecutionStatus::MaxIterations
        | ChangeExecutionStatus::MaxRuntime
        | ChangeExecutionStatus::UnsafeRequest => {
            state.status = RunStatus::Failed;
            state.error = Some(outcome.summary.clone());
            state.updated_at = now_rfc3339();
            write_state(context.run_dir, state).await?;
            bail!("change execution stopped: {}", outcome.summary);
        }
    }
}

async fn run_verification_phase(
    state: &mut RunState,
    context: &WorkerRunContext<'_>,
) -> anyhow::Result<()> {
    start_verification(state, context).await?;
    let verification =
        run_verification_once(state, context, Vec::new(), 1, "verification failed to run").await?;
    record_verification_outcome(state, &verification);

    if finish_successful_verification(state, context, &verification, false).await? {
        return Ok(());
    }
    if verification.failed_commands.is_empty() {
        return fail_verification(state, context, &verification, "verification failed").await;
    }

    let repair_outcome = run_repair_attempt(state, context, &verification).await?;
    if !handle_change_execution_outcome(state, context, &repair_outcome).await? {
        return Ok(());
    }

    start_verification(state, context).await?;
    let rerun = run_verification_once(
        state,
        context,
        verification.failed_commands.clone(),
        verification.next_index,
        "verification rerun failed to start",
    )
    .await?;
    record_verification_outcome(state, &rerun);

    if finish_successful_verification(state, context, &rerun, true).await? {
        return Ok(());
    }

    fail_verification(state, context, &rerun, "verification failed after repair").await
}

async fn start_verification(
    state: &mut RunState,
    context: &WorkerRunContext<'_>,
) -> anyhow::Result<()> {
    state.status = RunStatus::Verifying;
    state.updated_at = now_rfc3339();
    write_state(context.run_dir, state).await
}

async fn run_verification_once(
    state: &mut RunState,
    context: &WorkerRunContext<'_>,
    commands: Vec<String>,
    start_index: usize,
    error_context: &'static str,
) -> anyhow::Result<VerificationOutcome> {
    match run_verification(VerificationRequest {
        config: context.config,
        repo_context: context.repo_context,
        checkout_path: context.worktree_path,
        run_dir: context.run_dir,
        commands,
        start_index,
    })
    .await
    {
        Ok(verification) => Ok(verification),
        Err(err) => {
            state.status = RunStatus::Failed;
            state.error = Some(format!("{error_context}: {err}"));
            state.updated_at = now_rfc3339();
            write_state(context.run_dir, state).await?;
            Err(err).context(error_context)
        }
    }
}

async fn finish_successful_verification(
    state: &mut RunState,
    context: &WorkerRunContext<'_>,
    outcome: &VerificationOutcome,
    after_repair: bool,
) -> anyhow::Result<bool> {
    match outcome.status {
        VerificationStatus::Passed => {
            state.status = RunStatus::Verified;
            state.updated_at = now_rfc3339();
            write_state(context.run_dir, state).await?;
            let body = if after_repair {
                verification_passed_after_repair_comment(
                    context.branch_name,
                    &state.changed_files,
                    &outcome.summary,
                )
            } else {
                verification_passed_comment(
                    context.branch_name,
                    &state.changed_files,
                    &outcome.summary,
                )
            };
            post_comment_best_effort(
                context.github,
                context.token,
                context.repo_ref,
                context.issue_number,
                &body,
            )
            .await;
            info!(
                repo = %context.repo_ref,
                issue = context.issue_number,
                branch = %context.branch_name,
                run_id = %context.run_id,
                changed_files = ?state.changed_files,
                "completed local changes and verification"
            );
            Ok(true)
        }
        VerificationStatus::Skipped => {
            state.status = if after_repair {
                RunStatus::Verified
            } else {
                RunStatus::Completed
            };
            state.updated_at = now_rfc3339();
            write_state(context.run_dir, state).await?;
            post_comment_best_effort(
                context.github,
                context.token,
                context.repo_ref,
                context.issue_number,
                &verification_skipped_comment(context.branch_name, &state.changed_files),
            )
            .await;
            Ok(true)
        }
        VerificationStatus::Failed | VerificationStatus::TimedOut => Ok(false),
    }
}

async fn run_repair_attempt(
    state: &mut RunState,
    context: &WorkerRunContext<'_>,
    verification: &VerificationOutcome,
) -> anyhow::Result<ChangeExecutionOutcome> {
    state.status = RunStatus::Implementing;
    state.repair_attempted = true;
    state.repair_summary = Some("started bounded repair after verification failure".to_owned());
    state.updated_at = now_rfc3339();
    write_state(context.run_dir, state).await?;

    let repair_plan = repair_plan(context.plan, &verification.summary);
    let repair_outcome = match execute_change_loop(ChangeExecutionRequest {
        config: context.config,
        model: context.model,
        repo: context.repo,
        issue: context.issue,
        branch_name: context.branch_name,
        repo_context: context.repo_context,
        plan: &repair_plan,
        checkout_path: context.worktree_path,
        run_dir: context.run_dir,
    })
    .await
    {
        Ok(outcome) => outcome,
        Err(err) => {
            state.status = RunStatus::Failed;
            state.error = Some(format!("verification repair failed: {err}"));
            state.updated_at = now_rfc3339();
            write_state(context.run_dir, state).await?;
            return Err(err).context("verification repair failed");
        }
    };

    state.transcript_path = Some(repair_outcome.transcript_path.clone());
    state.diff_stat_path = repair_outcome.diff_stat_path.clone();
    state.changed_files = repair_outcome.changed_files.clone();
    state.repair_summary = Some(repair_outcome.summary.clone());
    state.updated_at = now_rfc3339();

    Ok(repair_outcome)
}

async fn fail_verification(
    state: &mut RunState,
    context: &WorkerRunContext<'_>,
    outcome: &VerificationOutcome,
    error_prefix: &str,
) -> anyhow::Result<()> {
    state.status = RunStatus::Failed;
    state.error = Some(outcome.summary.clone());
    state.updated_at = now_rfc3339();
    write_state(context.run_dir, state).await?;
    post_comment_best_effort(
        context.github,
        context.token,
        context.repo_ref,
        context.issue_number,
        &verification_failed_comment(context.branch_name, &outcome.summary),
    )
    .await;
    bail!("{error_prefix}: {}", outcome.summary)
}

struct WorkerRunContext<'a> {
    config: &'a AppConfig,
    github: &'a GitHubClient,
    model: &'a ModelClient,
    token: &'a InstallationToken,
    repo_ref: &'a RepoRef,
    repo: &'a RepositoryInfo,
    issue: &'a IssueInfo,
    branch_name: &'a str,
    repo_context: &'a RepoContext,
    plan: &'a str,
    worktree_path: &'a Path,
    run_dir: &'a Path,
    issue_number: u64,
    run_id: &'a str,
}

fn verification_passed_comment(
    branch_name: &str,
    changed_files: &[String],
    summary: &str,
) -> String {
    format!(
        "Prepared branch `{branch_name}`, completed bounded local code changes, and verification passed.\n\nChanged files:\n{}\n\nVerification:\n{summary}\n\nNo commit, push, or PR creation was performed in this slice.",
        changed_files_list(changed_files)
    )
}

fn verification_passed_after_repair_comment(
    branch_name: &str,
    changed_files: &[String],
    summary: &str,
) -> String {
    format!(
        "Prepared branch `{branch_name}`, repaired verification failures once, and verification passed on rerun.\n\nChanged files:\n{}\n\nVerification rerun:\n{summary}\n\nNo commit, push, or PR creation was performed in this slice.",
        changed_files_list(changed_files)
    )
}

fn verification_skipped_comment(branch_name: &str, changed_files: &[String]) -> String {
    format!(
        "Prepared branch `{branch_name}` and completed bounded local code changes.\n\nChanged files:\n{}\n\nVerification was skipped because no commands were selected. No commit, push, or PR creation was performed in this slice.",
        changed_files_list(changed_files)
    )
}

fn verification_failed_comment(branch_name: &str, summary: &str) -> String {
    format!(
        "Prepared branch `{branch_name}`, but verification failed.\n\nSummary:\n{summary}\n\nCommand logs were saved under the run's `verification/` artifact directory."
    )
}

fn record_verification_outcome(state: &mut RunState, outcome: &VerificationOutcome) {
    state.verification_path = Some(outcome.verification_path.clone());
    state.verification_status = Some(outcome.status);
    state.verification_summary = Some(outcome.summary.clone());
    state.verification_results.extend(outcome.results.clone());
    state.updated_at = now_rfc3339();
}

fn repair_plan(plan: &str, verification_summary: &str) -> String {
    format!(
        "{plan}\n\nVerification failed after the initial implementation. Make one minimal repair attempt only for the failures below. If the failure cannot be fixed safely with the bounded file actions, return done with status blocked.\n\nVerification failure summary:\n{verification_summary}"
    )
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

fn implementation_blocked_comment(
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

async fn post_comment_best_effort(
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

fn run_dir(config: &AppConfig, trigger: &IssueTrigger) -> PathBuf {
    config
        .workspace
        .runs_dir
        .join(trigger.repo.safe_dir_name())
        .join(format!("issue-{}", trigger.issue_number))
        .join(&trigger.run_id)
}

async fn write_state(run_dir: &std::path::Path, state: &RunState) -> anyhow::Result<()> {
    let state_path = run_dir.join("state.json");
    let body = serde_json::to_vec_pretty(state).context("failed to serialize run state")?;
    tokio::fs::write(&state_path, body)
        .await
        .with_context(|| format!("failed to write {}", state_path.display()))
}

async fn prepare_worktree(
    config: &AppConfig,
    token: &InstallationToken,
    repo: &RepoRef,
    repo_info: &RepositoryInfo,
    branch_name: &str,
) -> anyhow::Result<PathBuf> {
    let repo_dir = config.workspace.workdir.join(repo.safe_dir_name());
    tokio::fs::create_dir_all(&config.workspace.workdir)
        .await
        .with_context(|| format!("failed to create {}", config.workspace.workdir.display()))?;

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

async fn run_git_clone(
    cwd: &std::path::Path,
    token: &InstallationToken,
    repo: &RepoRef,
    repo_dir: &std::path::Path,
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

async fn run_git(
    cwd: &std::path::Path,
    token: &InstallationToken,
    args: Vec<String>,
) -> anyhow::Result<()> {
    run_git_at(cwd, token, args).await
}

async fn run_git_at(
    cwd: &std::path::Path,
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

fn branch_name_for_issue(issue: &IssueInfo) -> String {
    let change_type = change_type_for_issue(issue);
    let slug = slugify(&issue.title);
    format!("{change_type}/{slug}")
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

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
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
            "feat/add-github-app-webhook-receiver"
        );
    }

    #[test]
    fn slugify_falls_back_for_empty_title() {
        assert_eq!(slugify("?!"), "issue");
    }
}
