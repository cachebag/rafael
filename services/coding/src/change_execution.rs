mod actions;
mod content;
mod git;
mod history;
mod limits;
mod parser;
mod prompt;
mod sandbox;
mod search;
mod text;
mod transcript;
mod types;
mod write;

use std::path::Path;

use anyhow::Context;
use tokio::time::{Instant, timeout};

use crate::{
    config::AppConfig,
    github::{IssueInfo, RepositoryInfo},
    model::ModelClient,
    repo_context::RepoContext,
};

use self::{
    actions::{execute_action, repeated_action_result},
    git::{git_changed_files, git_diff_stat},
    history::{
        consecutive_action_repetitions, prompt_history_entry,
        should_finish_after_repeated_write_edit,
    },
    limits::{ExecutionLimits, MAX_CONSECUTIVE_IDENTICAL_ACTIONS},
    parser::parse_action,
    prompt::build_action_prompt,
    sandbox::RepoSandbox,
    text::truncate_for_prompt,
    transcript::append_transcript,
    types::{ActionResult, LoopStop},
};

pub use self::types::{ChangeExecutionOutcome, ChangeExecutionStatus};

pub struct ChangeExecutionRequest<'a> {
    pub config: &'a AppConfig,
    pub model: &'a ModelClient,
    pub repo: &'a RepositoryInfo,
    pub issue: &'a IssueInfo,
    pub branch_name: &'a str,
    pub repo_context: &'a RepoContext,
    pub plan: &'a str,
    pub checkout_path: &'a Path,
    pub run_dir: &'a Path,
}

pub async fn execute_change_loop(
    request: ChangeExecutionRequest<'_>,
) -> anyhow::Result<ChangeExecutionOutcome> {
    let ChangeExecutionRequest {
        config,
        model,
        repo,
        issue,
        branch_name,
        repo_context,
        plan,
        checkout_path,
        run_dir,
    } = request;
    let limits = ExecutionLimits::from_config(config);
    let sandbox = RepoSandbox::new(checkout_path).await?;
    let transcript_path = run_dir.join("transcript.jsonl");
    let diff_stat_path = run_dir.join("diff.stat");
    let mut history = Vec::new();
    let started_at = Instant::now();

    append_transcript(
        &transcript_path,
        0,
        "start",
        &serde_json::json!({
            "limits": limits,
            "checkout_path": checkout_path,
        }),
    )
    .await?;

    for iteration in 1..=limits.max_iterations {
        let elapsed = started_at.elapsed();
        if elapsed >= limits.max_runtime {
            return finish_outcome(
                ChangeExecutionStatus::MaxRuntime,
                "change execution loop exceeded its configured runtime".to_owned(),
                None,
                iteration - 1,
                &transcript_path,
                &diff_stat_path,
                &sandbox,
            )
            .await;
        }

        let prompt = build_action_prompt(
            repo,
            issue,
            branch_name,
            repo_context,
            plan,
            &history,
            &limits,
        )?;
        let remaining = limits.max_runtime.saturating_sub(elapsed);
        let raw_response = match timeout(remaining, model.change_action(&prompt)).await {
            Ok(Ok(response)) => response,
            Ok(Err(err)) => return Err(err).context("model failed while choosing change action"),
            Err(_) => {
                return finish_outcome(
                    ChangeExecutionStatus::MaxRuntime,
                    "model action request exceeded change execution runtime".to_owned(),
                    None,
                    iteration - 1,
                    &transcript_path,
                    &diff_stat_path,
                    &sandbox,
                )
                .await;
            }
        };

        append_transcript(
            &transcript_path,
            iteration,
            "model_response",
            &serde_json::json!({ "content": raw_response }),
        )
        .await?;

        let action = match parse_action(&raw_response) {
            Ok(action) => action,
            Err(err) => {
                let result = ActionResult::error(format!(
                    "model response was not a valid JSON action: {err}"
                ));
                append_transcript(&transcript_path, iteration, "result", &result).await?;
                history.push(prompt_history_entry(
                    iteration,
                    serde_json::json!({
                        "invalid_model_response": truncate_for_prompt(&raw_response)
                    }),
                    &result,
                ));
                continue;
            }
        };

        append_transcript(&transcript_path, iteration, "action", &action).await?;

        let prompt_action = action.for_prompt();
        let repeat_count = consecutive_action_repetitions(&history, &prompt_action);
        if repeat_count >= MAX_CONSECUTIVE_IDENTICAL_ACTIONS {
            if should_finish_after_repeated_write_edit(&history, &action, &prompt_action) {
                return finish_outcome(
                    ChangeExecutionStatus::Completed,
                    "applied current changes, then stopped because the model repeated an already-applied edit".to_owned(),
                    None,
                    iteration,
                    &transcript_path,
                    &diff_stat_path,
                    &sandbox,
                )
                .await;
            }

            return finish_outcome(
                ChangeExecutionStatus::Blocked,
                "model repeated the same action without making progress".to_owned(),
                Some("Please clarify the requested change or provide more specific implementation details.".to_owned()),
                iteration,
                &transcript_path,
                &diff_stat_path,
                &sandbox,
            )
            .await;
        }
        if repeat_count > 0 {
            let result = repeated_action_result(&action, &sandbox, &limits).await?;
            append_transcript(&transcript_path, iteration, "result", &result).await?;
            history.push(prompt_history_entry(iteration, prompt_action, &result));
            continue;
        }
        let execution = execute_action(&action, &sandbox, &diff_stat_path, &limits).await?;

        append_transcript(&transcript_path, iteration, "result", &execution.result).await?;
        history.push(prompt_history_entry(
            iteration,
            prompt_action,
            &execution.result,
        ));

        if let Some(stop) = execution.stop {
            return match stop {
                LoopStop::Completed { summary } => {
                    finish_outcome(
                        ChangeExecutionStatus::Completed,
                        summary,
                        None,
                        iteration,
                        &transcript_path,
                        &diff_stat_path,
                        &sandbox,
                    )
                    .await
                }
                LoopStop::Blocked { summary, question } => {
                    finish_outcome(
                        ChangeExecutionStatus::Blocked,
                        summary,
                        question,
                        iteration,
                        &transcript_path,
                        &diff_stat_path,
                        &sandbox,
                    )
                    .await
                }
                LoopStop::Unsafe { message } => {
                    finish_outcome(
                        ChangeExecutionStatus::UnsafeRequest,
                        message,
                        None,
                        iteration,
                        &transcript_path,
                        &diff_stat_path,
                        &sandbox,
                    )
                    .await
                }
            };
        }
    }

    finish_outcome(
        ChangeExecutionStatus::MaxIterations,
        format!(
            "change execution loop reached the configured iteration limit ({})",
            limits.max_iterations
        ),
        None,
        limits.max_iterations,
        &transcript_path,
        &diff_stat_path,
        &sandbox,
    )
    .await
}

async fn finish_outcome(
    status: ChangeExecutionStatus,
    summary: String,
    question: Option<String>,
    iterations: usize,
    transcript_path: &Path,
    diff_stat_path: &Path,
    sandbox: &RepoSandbox,
) -> anyhow::Result<ChangeExecutionOutcome> {
    let changed_files = git_changed_files(&sandbox.checkout_path).await?;
    let diff_stat = git_diff_stat(&sandbox.checkout_path).await?;
    if !diff_stat.trim().is_empty() {
        tokio::fs::write(diff_stat_path, &diff_stat)
            .await
            .with_context(|| format!("failed to write {}", diff_stat_path.display()))?;
    }

    let diff_stat_path = if diff_stat.trim().is_empty() {
        None
    } else {
        Some(diff_stat_path.to_path_buf())
    };

    Ok(ChangeExecutionOutcome {
        status,
        summary,
        question,
        transcript_path: transcript_path.to_path_buf(),
        diff_stat_path,
        changed_files,
        iterations,
    })
}
