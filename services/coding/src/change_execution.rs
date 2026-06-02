use std::{
    collections::BTreeSet,
    fs::OpenOptions,
    io::Write,
    path::{Component, Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use tokio::{
    process::Command,
    time::{Instant, timeout},
};
use tracing::warn;

use crate::{
    config::AppConfig,
    github::{IssueInfo, RepositoryInfo},
    model::ModelClient,
    repo_context::RepoContext,
};

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
        let execution =
            execute_action(&action, &sandbox, run_dir, &diff_stat_path, &limits).await?;

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

async fn execute_action(
    action: &ChangeAction,
    sandbox: &RepoSandbox,
    run_dir: &Path,
    diff_stat_path: &Path,
    limits: &ExecutionLimits,
) -> anyhow::Result<ActionExecution> {
    match action {
        ChangeAction::ReadFile { path } => read_file_action(path, sandbox, limits).await,
        ChangeAction::Search { query, path } => {
            search_action(query, path.as_deref(), sandbox, limits).await
        }
        ChangeAction::WriteFile { path, content } => {
            write_file_action(path, content, sandbox, run_dir, diff_stat_path, limits).await
        }
        ChangeAction::EditFile {
            path,
            old_text,
            new_text,
        } => {
            edit_file_action(
                path,
                old_text,
                new_text,
                sandbox,
                run_dir,
                diff_stat_path,
                limits,
            )
            .await
        }
        ChangeAction::Done {
            status,
            summary,
            question,
        } => {
            let result = ActionResult::ok(summary.clone());
            let stop = match status {
                DoneStatus::Completed => LoopStop::Completed {
                    summary: summary.clone(),
                },
                DoneStatus::Blocked => LoopStop::Blocked {
                    summary: summary.clone(),
                    question: question.clone(),
                },
            };

            Ok(ActionExecution {
                result,
                stop: Some(stop),
            })
        }
    }
}

async fn repeated_action_result(
    action: &ChangeAction,
    sandbox: &RepoSandbox,
    limits: &ExecutionLimits,
) -> anyhow::Result<ActionResult> {
    let message = match action {
        ChangeAction::ReadFile { .. } | ChangeAction::Search { .. } => {
            "same read/search action was just attempted; use the prior result and choose a different action".to_owned()
        }
        ChangeAction::WriteFile { .. } | ChangeAction::EditFile { .. } => {
            "same write/edit action was just attempted. If the previous result was ok, that change is already applied; do not repeat it. Inspect the current file content below, address the next requested feedback item, or return done only after all feedback is complete.".to_owned()
        }
        ChangeAction::Done { .. } => {
            "same done action was just attempted; choose a different action only if more work remains".to_owned()
        }
    };

    let mut result = ActionResult::error(message);

    if let Some((content, bytes)) = action_file_content_for_prompt(action, sandbox, limits).await {
        result = result.with_content(content, bytes);
    }

    if let Ok(diff_stat) = git_diff_stat(&sandbox.checkout_path).await {
        if !diff_stat.trim().is_empty() {
            result = result.with_diff_stat(diff_stat);
        }
    }
    if let Ok(changed_files) = git_changed_files(&sandbox.checkout_path).await {
        if !changed_files.is_empty() {
            result = result.with_changed_files(changed_files);
        }
    }

    Ok(result)
}

async fn action_file_content_for_prompt(
    action: &ChangeAction,
    sandbox: &RepoSandbox,
    limits: &ExecutionLimits,
) -> Option<(String, u64)> {
    file_content_for_prompt(action.target_path()?, sandbox, limits).await
}

async fn file_content_for_prompt(
    path: &str,
    sandbox: &RepoSandbox,
    limits: &ExecutionLimits,
) -> Option<(String, u64)> {
    let absolute_path = sandbox.existing_file(path).await.ok()?;
    let metadata = tokio::fs::metadata(&absolute_path).await.ok()?;
    if metadata.len() > limits.max_read_bytes {
        return None;
    }

    let bytes = tokio::fs::read(&absolute_path).await.ok()?;
    if bytes.contains(&0) {
        return None;
    }

    String::from_utf8(bytes)
        .ok()
        .map(|content| (content, metadata.len()))
}

async fn read_file_action(
    path: &str,
    sandbox: &RepoSandbox,
    limits: &ExecutionLimits,
) -> anyhow::Result<ActionExecution> {
    let absolute_path = match sandbox.existing_file(path).await {
        Ok(path) => path,
        Err(PathRejection::Unsafe(message)) => return Ok(unsafe_execution(message)),
        Err(PathRejection::NotFound(message)) => {
            return Ok(ActionExecution::continue_with(ActionResult::error(message)));
        }
    };

    let metadata = tokio::fs::metadata(&absolute_path)
        .await
        .with_context(|| format!("failed to stat {}", absolute_path.display()))?;
    if metadata.len() > limits.max_read_bytes {
        return Ok(ActionExecution::continue_with(ActionResult::error(
            format!(
                "file is too large to read safely ({} bytes > {} bytes)",
                metadata.len(),
                limits.max_read_bytes
            ),
        )));
    }

    let bytes = tokio::fs::read(&absolute_path)
        .await
        .with_context(|| format!("failed to read {}", absolute_path.display()))?;
    if bytes.contains(&0) {
        return Ok(ActionExecution::continue_with(ActionResult::error(
            "binary file reads are not allowed".to_owned(),
        )));
    }

    let content = match String::from_utf8(bytes) {
        Ok(content) => content,
        Err(_) => {
            return Ok(ActionExecution::continue_with(ActionResult::error(
                "file is not valid UTF-8".to_owned(),
            )));
        }
    };

    Ok(ActionExecution::continue_with(
        ActionResult::ok(format!("read {path}")).with_content(content, metadata.len()),
    ))
}

async fn search_action(
    query: &str,
    path: Option<&str>,
    sandbox: &RepoSandbox,
    limits: &ExecutionLimits,
) -> anyhow::Result<ActionExecution> {
    if query.trim().is_empty() {
        return Ok(ActionExecution::continue_with(ActionResult::error(
            "search query must be non-empty".to_owned(),
        )));
    }

    let scope = match sandbox.optional_existing_path(path).await {
        Ok(scope) => scope,
        Err(PathRejection::Unsafe(message)) => return Ok(unsafe_execution(message)),
        Err(PathRejection::NotFound(message)) => {
            return Ok(ActionExecution::continue_with(ActionResult::error(message)));
        }
    };

    let files = git_ls_files(&sandbox.checkout_path).await?;
    let query_lower = query.to_ascii_lowercase();
    let mut matches = Vec::new();

    for file in files {
        if matches.len() >= limits.max_search_results {
            break;
        }
        if !scope.contains(&file) {
            continue;
        }
        if is_sensitive_path(&file) {
            continue;
        }

        let absolute_path = match sandbox.existing_file(&file).await {
            Ok(path) => path,
            Err(PathRejection::Unsafe(_)) | Err(PathRejection::NotFound(_)) => continue,
        };
        let metadata = match tokio::fs::metadata(&absolute_path).await {
            Ok(metadata) if metadata.is_file() && metadata.len() <= limits.max_read_bytes => {
                metadata
            }
            _ => continue,
        };
        if metadata.len() == 0 {
            continue;
        }

        let bytes = match tokio::fs::read(&absolute_path).await {
            Ok(bytes) if !bytes.contains(&0) => bytes,
            _ => continue,
        };
        let content = match String::from_utf8(bytes) {
            Ok(content) => content,
            Err(_) => continue,
        };

        for (line_index, line) in content.lines().enumerate() {
            if line.to_ascii_lowercase().contains(&query_lower) {
                matches.push(SearchMatch {
                    path: file.clone(),
                    line: line_index + 1,
                    preview: truncate_text(line.trim(), MAX_SEARCH_PREVIEW_CHARS),
                });
                if matches.len() >= limits.max_search_results {
                    break;
                }
            }
        }
    }

    let result = ActionResult::ok(format!(
        "found {} match(es) for literal query `{query}`",
        matches.len()
    ))
    .with_matches(matches);

    Ok(ActionExecution::continue_with(result))
}

async fn write_file_action(
    path: &str,
    content: &str,
    sandbox: &RepoSandbox,
    _run_dir: &Path,
    diff_stat_path: &Path,
    limits: &ExecutionLimits,
) -> anyhow::Result<ActionExecution> {
    if content.len() > limits.max_write_bytes {
        return Ok(ActionExecution::continue_with(ActionResult::error(
            format!(
                "write content is too large ({} bytes > {} bytes)",
                content.len(),
                limits.max_write_bytes
            ),
        )));
    }

    let (relative_path, absolute_path) = match sandbox.write_target(path).await {
        Ok(target) => target,
        Err(PathRejection::Unsafe(message)) => return Ok(unsafe_execution(message)),
        Err(PathRejection::NotFound(message)) => {
            return Ok(ActionExecution::continue_with(ActionResult::error(message)));
        }
    };

    if let Some(result) = changed_file_cap_error(&relative_path, sandbox, limits).await? {
        return Ok(ActionExecution::continue_with(result));
    }

    tokio::fs::write(&absolute_path, content)
        .await
        .with_context(|| format!("failed to write {}", absolute_path.display()))?;

    write_result_after_change(
        format!("wrote {path}"),
        path,
        sandbox,
        diff_stat_path,
        limits,
    )
    .await
}

async fn edit_file_action(
    path: &str,
    old_text: &str,
    new_text: &str,
    sandbox: &RepoSandbox,
    _run_dir: &Path,
    diff_stat_path: &Path,
    limits: &ExecutionLimits,
) -> anyhow::Result<ActionExecution> {
    if old_text.is_empty() {
        return Ok(ActionExecution::continue_with(ActionResult::error(
            "old_text must be non-empty for edit_file".to_owned(),
        )));
    }

    let absolute_path = match sandbox.existing_file(path).await {
        Ok(path) => path,
        Err(PathRejection::Unsafe(message)) => return Ok(unsafe_execution(message)),
        Err(PathRejection::NotFound(message)) => {
            return Ok(ActionExecution::continue_with(ActionResult::error(message)));
        }
    };
    let relative_path = sanitize_repo_path(path).map_err(|message| anyhow::anyhow!(message))?;

    let metadata = tokio::fs::metadata(&absolute_path)
        .await
        .with_context(|| format!("failed to stat {}", absolute_path.display()))?;
    if metadata.len() > limits.max_read_bytes {
        return Ok(ActionExecution::continue_with(ActionResult::error(
            format!(
                "file is too large to edit safely ({} bytes > {} bytes)",
                metadata.len(),
                limits.max_read_bytes
            ),
        )));
    }

    if let Some(result) = changed_file_cap_error(&relative_path, sandbox, limits).await? {
        return Ok(ActionExecution::continue_with(result));
    }

    let bytes = tokio::fs::read(&absolute_path)
        .await
        .with_context(|| format!("failed to read {}", absolute_path.display()))?;
    if bytes.contains(&0) {
        return Ok(ActionExecution::continue_with(ActionResult::error(
            "binary file edits are not allowed".to_owned(),
        )));
    }
    let content = match String::from_utf8(bytes) {
        Ok(content) => content,
        Err(_) => {
            return Ok(ActionExecution::continue_with(ActionResult::error(
                "file is not valid UTF-8".to_owned(),
            )));
        }
    };

    let occurrence_count = content.match_indices(old_text).count();
    if occurrence_count != 1 {
        return Ok(ActionExecution::continue_with(
            ActionResult::error(format!(
                "old_text must match exactly once; found {occurrence_count} matches. Use the current file content and choose a corrected edit, another feedback item, or done if all feedback is addressed."
            ))
            .with_content(content, metadata.len()),
        ));
    }

    let updated = content.replacen(old_text, new_text, 1);
    if updated.len() > limits.max_write_bytes {
        return Ok(ActionExecution::continue_with(ActionResult::error(
            format!(
                "edited file would exceed max write size ({} bytes > {} bytes)",
                updated.len(),
                limits.max_write_bytes
            ),
        )));
    }

    tokio::fs::write(&absolute_path, updated)
        .await
        .with_context(|| format!("failed to write {}", absolute_path.display()))?;

    write_result_after_change(
        format!("edited {path}"),
        path,
        sandbox,
        diff_stat_path,
        limits,
    )
    .await
}

async fn changed_file_cap_error(
    relative_path: &Path,
    sandbox: &RepoSandbox,
    limits: &ExecutionLimits,
) -> anyhow::Result<Option<ActionResult>> {
    let changed_files = git_changed_files(&sandbox.checkout_path).await?;
    let relative_path = path_to_repo_string(relative_path);
    if changed_files.len() >= limits.max_changed_files && !changed_files.contains(&relative_path) {
        return Ok(Some(ActionResult::error(format!(
            "changed file cap reached ({} files); refusing to modify another path",
            limits.max_changed_files
        ))));
    }

    Ok(None)
}

async fn write_result_after_change(
    message: String,
    changed_path: &str,
    sandbox: &RepoSandbox,
    diff_stat_path: &Path,
    limits: &ExecutionLimits,
) -> anyhow::Result<ActionExecution> {
    let diff_stat = git_diff_stat(&sandbox.checkout_path).await?;
    tokio::fs::write(diff_stat_path, &diff_stat)
        .await
        .with_context(|| format!("failed to write {}", diff_stat_path.display()))?;
    let changed_files = git_changed_files(&sandbox.checkout_path).await?;
    let mut result = ActionResult::ok(message)
        .with_diff_stat(diff_stat)
        .with_changed_files(changed_files);

    if let Some((content, bytes)) = file_content_for_prompt(changed_path, sandbox, limits).await {
        result.message.push_str(
            "; current file content is included below. If this addressed the last requested item, return done instead of repeating the edit.",
        );
        result = result.with_content(content, bytes);
    }

    Ok(ActionExecution::continue_with(result))
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

fn unsafe_execution(message: String) -> ActionExecution {
    ActionExecution {
        result: ActionResult::unsafe_request(message.clone()),
        stop: Some(LoopStop::Unsafe { message }),
    }
}

fn build_action_prompt(
    repo: &RepositoryInfo,
    issue: &IssueInfo,
    branch_name: &str,
    repo_context: &RepoContext,
    plan: &str,
    history: &[PromptHistoryEntry],
    limits: &ExecutionLimits,
) -> anyhow::Result<String> {
    let body = repo_context
        .issue
        .body
        .as_deref()
        .unwrap_or("(no issue body)");
    let labels = issue
        .labels
        .iter()
        .map(|label| label.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let context_json = serde_json::to_string(repo_context)
        .context("failed to serialize repository context for change prompt")?;
    let history_json = serde_json::to_string(&prompt_history_tail(history))
        .context("failed to serialize change execution history")?;

    Ok(format!(
        "Repository: {}\nDefault branch: {}\nTarget branch: {}\nIssue #{}: {}\nLabels: {}\nIssue body:\n{}\n\nImplementation plan:\n{}\n\nRepository context JSON:\n{}\n\nPrior tool history JSON:\n{}\n\nYou are editing the prepared local branch through a bounded tool loop. Choose exactly one next action and return exactly one JSON object with no Markdown, comments, or extra text. Available actions:\n- {{\"action\":\"read_file\",\"path\":\"relative/path\"}}\n- {{\"action\":\"search\",\"query\":\"case-insensitive literal text\",\"path\":\"optional/relative/scope\"}}\n- {{\"action\":\"write_file\",\"path\":\"relative/path\",\"content\":\"complete file contents\"}}\n- {{\"action\":\"edit_file\",\"path\":\"relative/path\",\"old_text\":\"exact text appearing once\",\"new_text\":\"replacement text\"}}\n- {{\"action\":\"done\",\"status\":\"completed\",\"summary\":\"what changed\"}}\n- {{\"action\":\"done\",\"status\":\"blocked\",\"summary\":\"why blocked\",\"question\":\"what you need clarified\"}}\n\nConstraints:\n- Paths must be repository-relative, inside the checkout, and must not use .git, parent traversal, absolute paths, or known secret files.\n- Do not request shell commands, commits, pushes, branch changes, package installs, or network calls.\n- Keep changes minimal and focused on the issue. Prefer read_file/search before edits when context is missing.\n- Treat trusted review feedback in the implementation plan as a checklist. After a successful write/edit, move to the next unaddressed feedback item. Do not repeat the same edit.\n- If old_text no longer matches, the file may already be changed; use the current file content from tool history, read the file, or choose the next file.\n- Return done with status completed only after every requested feedback item is addressed.\n- write_file content must be at most {} bytes; reads are capped at {} bytes; the run may change at most {} files.\n- If the requested change cannot be completed safely with these actions, return done with status blocked.",
        repo.full_name,
        repo.default_branch,
        branch_name,
        issue.number,
        issue.title,
        if labels.is_empty() { "(none)" } else { &labels },
        body,
        plan,
        context_json,
        history_json,
        limits.max_write_bytes,
        limits.max_read_bytes,
        limits.max_changed_files
    ))
}

fn parse_action(raw_response: &str) -> anyhow::Result<ChangeAction> {
    let candidate = json_candidate(raw_response);
    serde_json::from_str(candidate).context("failed to parse JSON action")
}

fn json_candidate(raw_response: &str) -> &str {
    let trimmed = raw_response.trim();
    if !trimmed.starts_with("```") {
        return trimmed;
    }

    let Some((_, rest)) = trimmed.split_once('\n') else {
        return trimmed;
    };
    let rest = rest.trim();
    rest.strip_suffix("```").unwrap_or(rest).trim()
}

async fn append_transcript<T>(
    path: &Path,
    iteration: usize,
    kind: &'static str,
    payload: &T,
) -> anyhow::Result<()>
where
    T: Serialize + ?Sized,
{
    let payload =
        serde_json::to_value(payload).context("failed to serialize transcript payload")?;
    let entry = serde_json::json!({
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "iteration": iteration,
        "kind": kind,
        "payload": payload,
    });
    let mut line = serde_json::to_vec(&entry).context("failed to serialize transcript entry")?;
    line.push(b'\n');

    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("failed to open {}", path.display()))?;
        file.write_all(&line)
            .with_context(|| format!("failed to append {}", path.display()))?;
        Ok(())
    })
    .await
    .context("transcript append task failed")??;

    Ok(())
}

fn prompt_history_entry(
    iteration: usize,
    action: serde_json::Value,
    result: &ActionResult,
) -> PromptHistoryEntry {
    PromptHistoryEntry {
        iteration,
        action,
        result: result.for_prompt(),
    }
}

async fn git_ls_files(checkout_path: &Path) -> anyhow::Result<Vec<String>> {
    let output = run_git_capture(
        checkout_path,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ],
    )
    .await?;

    let mut files = Vec::new();
    for entry in output.split('\0') {
        if entry.is_empty() {
            continue;
        }
        if sanitize_repo_path(entry).is_ok() {
            files.push(entry.to_owned());
        } else {
            warn!(path = %entry, "ignored unsafe path from git ls-files");
        }
    }

    Ok(files)
}

async fn git_changed_files(checkout_path: &Path) -> anyhow::Result<Vec<String>> {
    let output = run_git_capture(
        checkout_path,
        &["status", "--porcelain", "--untracked-files=normal"],
    )
    .await?;
    let mut files = BTreeSet::new();

    for line in output.lines() {
        if line.len() < 4 {
            continue;
        }
        let path = line[3..].trim();
        let path = path
            .split_once(" -> ")
            .map(|(_, new_path)| new_path)
            .unwrap_or(path)
            .trim_matches('"');
        if sanitize_repo_path(path).is_ok() {
            files.insert(path.to_owned());
        }
    }

    Ok(files.into_iter().collect())
}

async fn git_diff_stat(checkout_path: &Path) -> anyhow::Result<String> {
    run_git_capture(checkout_path, &["diff", "--stat"]).await
}

async fn run_git_capture(checkout_path: &Path, args: &[&str]) -> anyhow::Result<String> {
    let output = timeout(
        Duration::from_secs(GIT_COMMAND_TIMEOUT_SECS),
        Command::new("git")
            .current_dir(checkout_path)
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null())
            .args(args)
            .output(),
    )
    .await
    .context("git command timed out")?
    .context("failed to execute git")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git command failed with status {}: {stderr}", output.status);
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[derive(Debug)]
struct RepoSandbox {
    checkout_path: PathBuf,
    canonical_checkout_path: PathBuf,
}

impl RepoSandbox {
    async fn new(checkout_path: &Path) -> anyhow::Result<Self> {
        let canonical_checkout_path = tokio::fs::canonicalize(checkout_path)
            .await
            .with_context(|| format!("failed to canonicalize {}", checkout_path.display()))?;

        Ok(Self {
            checkout_path: checkout_path.to_path_buf(),
            canonical_checkout_path,
        })
    }

    async fn existing_file(&self, path: &str) -> Result<PathBuf, PathRejection> {
        let relative_path = sanitize_repo_path(path).map_err(PathRejection::Unsafe)?;
        let absolute_path = self.checkout_path.join(&relative_path);
        let canonical_path = self
            .canonical_existing_path(&absolute_path)
            .await
            .map_err(path_rejection_for_canonical_error)?;

        let metadata = tokio::fs::metadata(&canonical_path)
            .await
            .map_err(|err| PathRejection::NotFound(format!("failed to stat {path}: {err}")))?;
        if !metadata.is_file() {
            return Err(PathRejection::NotFound(format!(
                "path is not a file: {path}"
            )));
        }

        Ok(canonical_path)
    }

    async fn optional_existing_path(
        &self,
        path: Option<&str>,
    ) -> Result<SearchScope, PathRejection> {
        let Some(path) = path else {
            return Ok(SearchScope::All);
        };
        let relative_path = sanitize_repo_path(path).map_err(PathRejection::Unsafe)?;
        let absolute_path = self.checkout_path.join(&relative_path);
        self.canonical_existing_path(&absolute_path)
            .await
            .map_err(path_rejection_for_canonical_error)?;

        Ok(SearchScope::Path(path_to_repo_string(&relative_path)))
    }

    async fn write_target(&self, path: &str) -> Result<(PathBuf, PathBuf), PathRejection> {
        let relative_path = sanitize_repo_path(path).map_err(PathRejection::Unsafe)?;
        let absolute_path = self.checkout_path.join(&relative_path);
        let Some(parent) = absolute_path.parent() else {
            return Err(PathRejection::Unsafe(format!(
                "write target has no parent directory: {path}"
            )));
        };
        let Some(file_name) = absolute_path.file_name() else {
            return Err(PathRejection::Unsafe(format!(
                "write target has no file name: {path}"
            )));
        };

        let canonical_parent = self
            .canonical_existing_path(parent)
            .await
            .map_err(path_rejection_for_canonical_error)?;
        if !canonical_parent.is_dir() {
            return Err(PathRejection::NotFound(format!(
                "write target parent is not a directory: {}",
                parent.display()
            )));
        }

        if let Ok(metadata) = tokio::fs::symlink_metadata(&absolute_path).await {
            if metadata.file_type().is_symlink() {
                return Err(PathRejection::Unsafe(
                    "write targets must not be symlinks".to_owned(),
                ));
            }
            let canonical_target = self
                .canonical_existing_path(&absolute_path)
                .await
                .map_err(path_rejection_for_canonical_error)?;
            return Ok((relative_path, canonical_target));
        }

        Ok((relative_path, canonical_parent.join(Path::new(file_name))))
    }

    async fn canonical_existing_path(&self, path: &Path) -> Result<PathBuf, String> {
        let canonical_path = tokio::fs::canonicalize(path)
            .await
            .map_err(|err| format!("path does not exist or cannot be accessed: {err}"))?;
        if !canonical_path.starts_with(&self.canonical_checkout_path) {
            return Err(PATH_ESCAPES_REPOSITORY_CHECKOUT.to_owned());
        }

        Ok(canonical_path)
    }
}

#[derive(Debug)]
enum PathRejection {
    Unsafe(String),
    NotFound(String),
}

fn path_rejection_for_canonical_error(message: String) -> PathRejection {
    if message == PATH_ESCAPES_REPOSITORY_CHECKOUT {
        PathRejection::Unsafe(message)
    } else {
        PathRejection::NotFound(message)
    }
}

#[derive(Debug)]
enum SearchScope {
    All,
    Path(String),
}

impl SearchScope {
    fn contains(&self, path: &str) -> bool {
        match self {
            SearchScope::All => true,
            SearchScope::Path(scope) => path == scope || path.starts_with(&format!("{scope}/")),
        }
    }
}

fn sanitize_repo_path(path: &str) -> Result<PathBuf, String> {
    if path.is_empty() || path.contains('\0') {
        return Err("path must be non-empty and must not contain NUL bytes".to_owned());
    }

    let path = Path::new(path);
    if path.is_absolute() {
        return Err("absolute paths are not allowed".to_owned());
    }

    let mut clean_path = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                if part == ".git" {
                    return Err("paths under .git are not allowed".to_owned());
                }
                clean_path.push(part);
            }
            Component::CurDir => {}
            Component::ParentDir => return Err("parent traversal is not allowed".to_owned()),
            Component::RootDir | Component::Prefix(_) => {
                return Err("absolute paths are not allowed".to_owned());
            }
        }
    }

    if clean_path.as_os_str().is_empty() {
        return Err("path must name a repository file".to_owned());
    }

    let clean = path_to_repo_string(&clean_path);
    if is_sensitive_path(&clean) {
        return Err("known secret-bearing paths are not allowed".to_owned());
    }

    Ok(clean_path)
}

fn path_to_repo_string(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn is_sensitive_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let file_name = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();

    if is_allowed_env_example(&file_name) {
        return false;
    }

    lower == ".env"
        || lower.starts_with(".env.")
        || lower.contains("/.env")
        || lower.ends_with(".pem")
        || lower.ends_with(".key")
        || lower.contains("secret")
        || lower.contains("token")
        || lower.contains("credential")
        || matches!(
            file_name.as_str(),
            "id_rsa" | "id_ed25519" | "credentials" | "netrc" | ".netrc"
        )
}

fn is_allowed_env_example(file_name: &str) -> bool {
    matches!(file_name, ".env.example" | ".env.sample" | ".env.template")
}

fn truncate_for_prompt(value: &str) -> String {
    truncate_text(value, MAX_HISTORY_VALUE_CHARS)
}

fn truncate_text(value: &str, max_chars: usize) -> String {
    let mut truncated = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        truncated.push_str("\n...[truncated]");
    }
    truncated
}

#[derive(Debug, Clone, Copy, Serialize)]
struct ExecutionLimits {
    max_iterations: usize,
    #[serde(skip)]
    max_runtime: Duration,
    max_runtime_seconds: u64,
    max_read_bytes: u64,
    max_write_bytes: usize,
    max_changed_files: usize,
    max_search_results: usize,
}

impl ExecutionLimits {
    fn from_config(config: &AppConfig) -> Self {
        Self {
            max_iterations: config.workspace.max_tool_iterations,
            max_runtime: Duration::from_secs(config.workspace.max_tool_runtime_seconds),
            max_runtime_seconds: config.workspace.max_tool_runtime_seconds,
            max_read_bytes: config.workspace.max_file_read_bytes,
            max_write_bytes: config.workspace.max_write_bytes,
            max_changed_files: config.workspace.max_changed_files,
            max_search_results: MAX_SEARCH_RESULTS,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeExecutionOutcome {
    pub status: ChangeExecutionStatus,
    pub summary: String,
    pub question: Option<String>,
    pub transcript_path: PathBuf,
    pub diff_stat_path: Option<PathBuf>,
    pub changed_files: Vec<String>,
    pub iterations: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChangeExecutionStatus {
    Completed,
    Blocked,
    MaxIterations,
    MaxRuntime,
    UnsafeRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum ChangeAction {
    ReadFile {
        path: String,
    },
    Search {
        #[serde(alias = "pattern")]
        query: String,
        #[serde(default)]
        path: Option<String>,
    },
    WriteFile {
        path: String,
        content: String,
    },
    EditFile {
        path: String,
        old_text: String,
        new_text: String,
    },
    Done {
        #[serde(default)]
        status: DoneStatus,
        summary: String,
        #[serde(default)]
        question: Option<String>,
    },
}

impl ChangeAction {
    fn for_prompt(&self) -> serde_json::Value {
        match self {
            Self::ReadFile { path } => serde_json::json!({
                "action": "read_file",
                "path": path,
            }),
            Self::Search { query, path } => serde_json::json!({
                "action": "search",
                "query": query,
                "path": path,
            }),
            Self::WriteFile { path, content } => serde_json::json!({
                "action": "write_file",
                "path": path,
                "content_bytes": content.len(),
                "content_preview": truncate_text(content, MAX_HISTORY_ACTION_TEXT_CHARS),
            }),
            Self::EditFile {
                path,
                old_text,
                new_text,
            } => serde_json::json!({
                "action": "edit_file",
                "path": path,
                "old_text_bytes": old_text.len(),
                "old_text_preview": truncate_text(old_text, MAX_HISTORY_ACTION_TEXT_CHARS),
                "new_text_bytes": new_text.len(),
                "new_text_preview": truncate_text(new_text, MAX_HISTORY_ACTION_TEXT_CHARS),
            }),
            Self::Done {
                status,
                summary,
                question,
            } => serde_json::json!({
                "action": "done",
                "status": status,
                "summary": summary,
                "question": question,
            }),
        }
    }

    fn target_path(&self) -> Option<&str> {
        match self {
            Self::ReadFile { path }
            | Self::WriteFile { path, .. }
            | Self::EditFile { path, .. } => Some(path),
            Self::Search { path, .. } => path.as_deref(),
            Self::Done { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
enum DoneStatus {
    #[default]
    Completed,
    Blocked,
}

#[derive(Debug)]
struct ActionExecution {
    result: ActionResult,
    stop: Option<LoopStop>,
}

impl ActionExecution {
    fn continue_with(result: ActionResult) -> Self {
        Self { result, stop: None }
    }
}

#[derive(Debug)]
enum LoopStop {
    Completed {
        summary: String,
    },
    Blocked {
        summary: String,
        question: Option<String>,
    },
    Unsafe {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ActionResult {
    status: ActionResultStatus,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    matches: Option<Vec<SearchMatch>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    diff_stat: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    changed_files: Option<Vec<String>>,
}

impl ActionResult {
    fn ok(message: String) -> Self {
        Self {
            status: ActionResultStatus::Ok,
            message,
            content: None,
            bytes: None,
            matches: None,
            diff_stat: None,
            changed_files: None,
        }
    }

    fn error(message: String) -> Self {
        Self {
            status: ActionResultStatus::Error,
            message,
            content: None,
            bytes: None,
            matches: None,
            diff_stat: None,
            changed_files: None,
        }
    }

    fn unsafe_request(message: String) -> Self {
        Self {
            status: ActionResultStatus::Unsafe,
            message,
            content: None,
            bytes: None,
            matches: None,
            diff_stat: None,
            changed_files: None,
        }
    }

    fn with_content(mut self, content: String, bytes: u64) -> Self {
        self.content = Some(content);
        self.bytes = Some(bytes);
        self
    }

    fn with_matches(mut self, matches: Vec<SearchMatch>) -> Self {
        self.matches = Some(matches);
        self
    }

    fn with_diff_stat(mut self, diff_stat: String) -> Self {
        self.diff_stat = Some(diff_stat);
        self
    }

    fn with_changed_files(mut self, changed_files: Vec<String>) -> Self {
        self.changed_files = Some(changed_files);
        self
    }

    fn for_prompt(&self) -> serde_json::Value {
        let mut result = self.clone();
        if let Some(content) = result.content.as_deref() {
            result.content = Some(truncate_for_prompt(content));
        }
        if let Some(diff_stat) = result.diff_stat.as_deref() {
            result.diff_stat = Some(truncate_for_prompt(diff_stat));
        }

        serde_json::to_value(result).unwrap_or_else(|_| {
            serde_json::json!({
                "status": "error",
                "message": "failed to serialize action result for prompt"
            })
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ActionResultStatus {
    Ok,
    Error,
    Unsafe,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SearchMatch {
    path: String,
    line: usize,
    preview: String,
}

#[derive(Debug, Clone, Serialize)]
struct PromptHistoryEntry {
    iteration: usize,
    action: serde_json::Value,
    result: serde_json::Value,
}

fn prompt_history_tail(history: &[PromptHistoryEntry]) -> &[PromptHistoryEntry] {
    if history.len() <= MAX_PROMPT_HISTORY_ENTRIES {
        history
    } else {
        &history[history.len() - MAX_PROMPT_HISTORY_ENTRIES..]
    }
}

fn consecutive_action_repetitions(
    history: &[PromptHistoryEntry],
    action: &serde_json::Value,
) -> usize {
    history
        .iter()
        .rev()
        .take_while(|entry| entry.action == *action)
        .count()
}

fn should_finish_after_repeated_write_edit(
    history: &[PromptHistoryEntry],
    action: &ChangeAction,
    prompt_action: &serde_json::Value,
) -> bool {
    if !matches!(
        action,
        ChangeAction::WriteFile { .. } | ChangeAction::EditFile { .. }
    ) {
        return false;
    }

    history
        .iter()
        .rev()
        .take_while(|entry| entry.action == *prompt_action)
        .any(|entry| {
            let status_is_ok = entry
                .result
                .get("status")
                .and_then(serde_json::Value::as_str)
                == Some("ok");
            let has_changed_files = entry
                .result
                .get("changed_files")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|files| !files.is_empty());

            status_is_ok && has_changed_files
        })
}

const GIT_COMMAND_TIMEOUT_SECS: u64 = 30;
const MAX_SEARCH_RESULTS: usize = 40;
const MAX_SEARCH_PREVIEW_CHARS: usize = 240;
const MAX_HISTORY_ACTION_TEXT_CHARS: usize = 2_000;
const MAX_HISTORY_VALUE_CHARS: usize = 4_000;
const MAX_PROMPT_HISTORY_ENTRIES: usize = 8;
const MAX_CONSECUTIVE_IDENTICAL_ACTIONS: usize = 3;
const PATH_ESCAPES_REPOSITORY_CHECKOUT: &str = "path escapes repository checkout";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_edit_action() {
        let action = parse_action(
            r#"{"action":"edit_file","path":"src/main.rs","old_text":"old","new_text":"new"}"#,
        )
        .unwrap();

        assert!(matches!(action, ChangeAction::EditFile { .. }));
    }

    #[test]
    fn strips_json_fences() {
        let action = parse_action(
            "```json\n{\"action\":\"done\",\"status\":\"completed\",\"summary\":\"ok\"}\n```",
        )
        .unwrap();

        assert!(matches!(action, ChangeAction::Done { .. }));
    }

    #[test]
    fn rejects_unsafe_paths() {
        assert!(sanitize_repo_path("../Cargo.toml").is_err());
        assert!(sanitize_repo_path(".git/config").is_err());
        assert!(sanitize_repo_path("/tmp/file").is_err());
        assert!(sanitize_repo_path(".env").is_err());
        assert!(sanitize_repo_path(".env.example").is_ok());
        assert!(sanitize_repo_path("services/coding/.env.example").is_ok());
        assert!(sanitize_repo_path("services/coding/.env.local").is_err());
        assert!(sanitize_repo_path("src/main.rs").is_ok());
    }

    #[test]
    fn prompt_history_tail_keeps_recent_entries() {
        let history = (1..=10)
            .map(|iteration| PromptHistoryEntry {
                iteration,
                action: serde_json::json!({"action":"read_file","path":iteration.to_string()}),
                result: serde_json::json!({"status":"ok"}),
            })
            .collect::<Vec<_>>();

        let tail = prompt_history_tail(&history);

        assert_eq!(tail.len(), MAX_PROMPT_HISTORY_ENTRIES);
        assert_eq!(tail.first().unwrap().iteration, 3);
        assert_eq!(tail.last().unwrap().iteration, 10);
    }

    #[test]
    fn repeated_action_count_is_consecutive_only() {
        let repeated = serde_json::json!({"action":"read_file","path":"README.md"});
        let history = vec![
            PromptHistoryEntry {
                iteration: 1,
                action: repeated.clone(),
                result: serde_json::json!({"status":"ok"}),
            },
            PromptHistoryEntry {
                iteration: 2,
                action: serde_json::json!({"action":"read_file","path":"Cargo.toml"}),
                result: serde_json::json!({"status":"ok"}),
            },
            PromptHistoryEntry {
                iteration: 3,
                action: repeated.clone(),
                result: serde_json::json!({"status":"ok"}),
            },
        ];

        assert_eq!(consecutive_action_repetitions(&history, &repeated), 1);
    }

    #[test]
    fn repeated_successful_edit_can_finish_with_progress() {
        let action = ChangeAction::EditFile {
            path: "README.md".to_owned(),
            old_text: "old".to_owned(),
            new_text: "new".to_owned(),
        };
        let prompt_action = action.for_prompt();
        let history = vec![
            PromptHistoryEntry {
                iteration: 1,
                action: prompt_action.clone(),
                result: serde_json::json!({
                    "status": "ok",
                    "changed_files": ["README.md"],
                }),
            },
            PromptHistoryEntry {
                iteration: 2,
                action: prompt_action.clone(),
                result: serde_json::json!({"status": "error"}),
            },
        ];

        assert!(should_finish_after_repeated_write_edit(
            &history,
            &action,
            &prompt_action
        ));
    }

    #[test]
    fn repeated_read_does_not_finish_with_progress() {
        let action = ChangeAction::ReadFile {
            path: "README.md".to_owned(),
        };
        let prompt_action = action.for_prompt();
        let history = vec![PromptHistoryEntry {
            iteration: 1,
            action: prompt_action.clone(),
            result: serde_json::json!({
                "status": "ok",
                "changed_files": ["README.md"],
            }),
        }];

        assert!(!should_finish_after_repeated_write_edit(
            &history,
            &action,
            &prompt_action
        ));
    }
}
