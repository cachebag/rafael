use std::{path::Path, time::Duration};

use anyhow::{Context, bail};
use tokio::time::timeout;
use tracing::info;

use crate::{
    change_execution::{ChangeExecutionRequest, execute_change_loop},
    config::AppConfig,
    github::{GitHubClient, InstallationToken, IssueInfo, RepositoryInfo},
    model::ModelClient,
    repo_context::{RepoContext, collect_repository_context},
    types::{IssueTrigger, PullRequestRevisionTrigger, RepoRef},
};

mod comments;
mod git;
mod paths;
mod phases;
mod pr_revision;
mod state;

use comments::post_comment_best_effort;
use git::{branch_name_for_issue, prepare_pull_request_worktree, prepare_worktree};
use paths::{pull_request_run_dir, run_dir};
use phases::{handle_change_execution_outcome, run_publish_phase, run_verification_phase};
use pr_revision::{PullRequestState, collect_pull_request_feedback, pull_request_revision_plan};
pub use state::{RunState, RunStatus};
use state::{now_rfc3339, write_state};

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
        commit_sha: None,
        pr_url: None,
        pr_number: None,
        publish_summary: None,
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

    if !config.github.quiet_comments {
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
    }

    state.issue_title = Some(issue.title.clone());
    state.issue_url = Some(issue.html_url.clone());
    state.default_branch = Some(repo.default_branch.clone());

    let branch_name = branch_name_for_issue(&issue);
    let worktree_path = prepare_worktree(
        &config,
        &token,
        &trigger.repo,
        &repo,
        trigger.issue_number,
        &trigger.run_id,
        &branch_name,
    )
    .await?;

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
        existing_pull_request: None,
    };

    if !handle_change_execution_outcome(&mut state, &run_context, &outcome).await? {
        return Ok(());
    }

    run_verification_phase(&mut state, &run_context).await?;
    run_publish_phase(&mut state, &run_context).await
}

pub async fn run_pull_request_revision(
    config: AppConfig,
    trigger: PullRequestRevisionTrigger,
) -> anyhow::Result<()> {
    let max_run = Duration::from_secs(config.workspace.max_run_minutes * 60);
    timeout(max_run, run_pull_request_revision_inner(config, trigger))
        .await
        .context("pull request revision run exceeded configured time limit")?
}

async fn run_pull_request_revision_inner(
    config: AppConfig,
    trigger: PullRequestRevisionTrigger,
) -> anyhow::Result<()> {
    let installation_id = trigger
        .installation_id
        .or(config.github.installation_id)
        .context("installation id must come from webhook payload or config")?;
    let created_at = now_rfc3339();
    let run_dir = pull_request_run_dir(&config, &trigger);

    tokio::fs::create_dir_all(&run_dir)
        .await
        .with_context(|| format!("failed to create {}", run_dir.display()))?;

    let mut state = RunState {
        run_id: trigger.run_id.clone(),
        repo: trigger.repo.clone(),
        issue_number: trigger.pull_number,
        trigger: trigger.trigger,
        actor: trigger.actor.clone(),
        installation_id,
        status: RunStatus::Received,
        issue_title: None,
        issue_url: None,
        default_branch: trigger.default_branch.clone(),
        branch_name: trigger.head_branch.clone(),
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
        commit_sha: None,
        pr_url: None,
        pr_number: Some(trigger.pull_number),
        publish_summary: None,
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
    let pull_request = github
        .pull_request(&token, &trigger.repo, trigger.pull_number)
        .await?;

    if !pull_request.state.eq_ignore_ascii_case("open") {
        bail!("pull request #{} is not open", pull_request.number);
    }

    let head_repo = pull_request
        .head
        .repo
        .as_ref()
        .context("pull request head repository is unavailable")?;
    if !head_repo
        .full_name
        .eq_ignore_ascii_case(&trigger.repo.full_name())
    {
        bail!(
            "refusing to revise pull request #{} from head repository `{}`",
            pull_request.number,
            head_repo.full_name
        );
    }

    let issue = github
        .issue(&token, &trigger.repo, trigger.pull_number)
        .await?;
    let issue_comments = github
        .issue_comments(&token, &trigger.repo, trigger.pull_number)
        .await?;
    let pull_request_files = github
        .pull_request_files(&token, &trigger.repo, trigger.pull_number)
        .await?;
    let pull_request_state = PullRequestState::from_github(&pull_request, pull_request_files);
    let pull_request_state_path = run_dir.join("pr-state.json");
    let pull_request_state_body =
        serde_json::to_vec_pretty(&pull_request_state).context("failed to serialize PR state")?;
    tokio::fs::write(&pull_request_state_path, pull_request_state_body)
        .await
        .with_context(|| format!("failed to write {}", pull_request_state_path.display()))?;

    let feedback = collect_pull_request_feedback(
        &config,
        &github,
        &token,
        &trigger.repo,
        trigger.pull_number,
        issue_comments.clone(),
    )
    .await?;
    let feedback_path = run_dir.join("review-feedback.json");
    let feedback_body =
        serde_json::to_vec_pretty(&feedback).context("failed to serialize PR feedback")?;
    tokio::fs::write(&feedback_path, feedback_body)
        .await
        .with_context(|| format!("failed to write {}", feedback_path.display()))?;

    if !config.github.quiet_comments {
        post_comment_best_effort(
            &github,
            &token,
            &trigger.repo,
            trigger.pull_number,
            &format!(
                "Started a pull request revision run.\n\nRun: `{}`\nTrigger: `{}`",
                trigger.run_id, trigger.trigger
            ),
        )
        .await;
    }

    let branch_name = pull_request.head.ref_name.clone();
    let worktree_path = prepare_pull_request_worktree(
        &config,
        &token,
        &trigger.repo,
        trigger.pull_number,
        &trigger.run_id,
        &branch_name,
    )
    .await?;

    state.status = RunStatus::Prepared;
    state.issue_title = Some(issue.title.clone());
    state.issue_url = Some(issue.html_url.clone());
    state.default_branch = Some(repo.default_branch.clone());
    state.branch_name = Some(branch_name.clone());
    state.worktree_path = Some(worktree_path.clone());
    state.pr_url = Some(pull_request.html_url.clone());
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

    let plan = pull_request_revision_plan(&pull_request_state, &feedback);
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
        issue_number: trigger.pull_number,
        run_id: &trigger.run_id,
        existing_pull_request: Some(ExistingPullRequestTarget {
            number: pull_request.number,
            html_url: pull_request.html_url.clone(),
        }),
    };

    if !handle_change_execution_outcome(&mut state, &run_context, &outcome).await? {
        return Ok(());
    }

    run_verification_phase(&mut state, &run_context).await?;
    run_publish_phase(&mut state, &run_context).await
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
    existing_pull_request: Option<ExistingPullRequestTarget>,
}

struct ExistingPullRequestTarget {
    number: u64,
    html_url: String,
}
