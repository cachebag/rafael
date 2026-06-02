use anyhow::{Context, bail};
use tracing::info;

use crate::{
    change_execution::{
        ChangeExecutionOutcome, ChangeExecutionRequest, ChangeExecutionStatus, execute_change_loop,
    },
    config::AppConfig,
    publish::{
        ExistingPullRequest, PublishCommitRequest, PublishPullRequestRequest,
        prepare_publish_commit, publish_pull_request,
    },
    verification::{
        VerificationOutcome, VerificationRequest, VerificationStatus, run_verification,
    },
};

use super::{
    WorkerRunContext,
    comments::{
        implementation_blocked_comment, post_comment_best_effort, publish_completed_comment,
        verification_failed_comment, verification_passed_after_repair_comment,
        verification_passed_comment, verification_skipped_comment,
    },
    state::{RunState, RunStatus, now_rfc3339, write_state},
};

pub(super) async fn handle_change_execution_outcome(
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

pub(super) async fn run_verification_phase(
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

pub(super) async fn run_publish_phase(
    state: &mut RunState,
    context: &WorkerRunContext<'_>,
) -> anyhow::Result<()> {
    if !should_attempt_publish(context.config, state) {
        return Ok(());
    }

    state.status = RunStatus::Publishing;
    state.updated_at = now_rfc3339();
    write_state(context.run_dir, state).await?;

    let commit = match prepare_publish_commit(PublishCommitRequest {
        config: context.config,
        repo: context.repo,
        issue: context.issue,
        branch_name: context.branch_name,
        checkout_path: context.worktree_path,
        verification_status: state.verification_status,
    })
    .await
    {
        Ok(commit) => commit,
        Err(err) => {
            state.status = RunStatus::Failed;
            state.error = Some(format!("publish commit failed: {err}"));
            state.updated_at = now_rfc3339();
            write_state(context.run_dir, state).await?;
            return Err(err).context("publish commit failed");
        }
    };

    state.commit_sha = Some(commit.commit_sha.clone());
    state.publish_summary = Some(format!("created local commit {}", commit.commit_sha));
    state.updated_at = now_rfc3339();
    write_state(context.run_dir, state).await?;

    let published = match publish_pull_request(PublishPullRequestRequest {
        github: context.github,
        token: context.token,
        repo_ref: context.repo_ref,
        repo: context.repo,
        issue: context.issue,
        branch_name: context.branch_name,
        checkout_path: context.worktree_path,
        implementation_summary: state.implementation_summary.as_deref(),
        existing_pull_request: context.existing_pull_request.as_ref().map(|pull| {
            ExistingPullRequest {
                number: pull.number,
                html_url: pull.html_url.as_str(),
            }
        }),
    })
    .await
    {
        Ok(published) => published,
        Err(err) => {
            state.status = RunStatus::Failed;
            state.error = Some(format!("publish PR failed: {err}"));
            state.updated_at = now_rfc3339();
            write_state(context.run_dir, state).await?;
            return Err(err).context("publish PR failed");
        }
    };

    state.status = RunStatus::Published;
    state.pr_url = Some(published.pr_url.clone());
    state.pr_number = Some(published.pr_number);
    state.publish_summary = Some(if published.created {
        format!("created pull request {}", published.pr_url)
    } else {
        format!("updated pull request {}", published.pr_url)
    });
    state.updated_at = now_rfc3339();
    write_state(context.run_dir, state).await?;

    post_comment_best_effort(
        context.github,
        context.token,
        context.repo_ref,
        context.issue_number,
        &publish_completed_comment(&published.pr_url, &commit.commit_sha, published.created),
    )
    .await;

    Ok(())
}

fn should_attempt_publish(config: &AppConfig, state: &RunState) -> bool {
    matches!(state.status, RunStatus::Verified)
        || (config.workspace.allow_unverified_publish
            && matches!(state.status, RunStatus::Completed))
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
            if !context.config.github.quiet_comments {
                post_comment_best_effort(
                    context.github,
                    context.token,
                    context.repo_ref,
                    context.issue_number,
                    &body,
                )
                .await;
            }
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
            let publish_will_continue =
                after_repair || context.config.workspace.allow_unverified_publish;
            if !context.config.github.quiet_comments || !publish_will_continue {
                post_comment_best_effort(
                    context.github,
                    context.token,
                    context.repo_ref,
                    context.issue_number,
                    &verification_skipped_comment(context.branch_name, &state.changed_files),
                )
                .await;
            }
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
