use std::path::Path;

use anyhow::Context;

use super::{
    content::file_content_for_prompt,
    git::{git_changed_files, git_diff_stat},
    limits::ExecutionLimits,
    sandbox::{PathRejection, RepoSandbox, path_to_repo_string, sanitize_repo_path},
    types::{ActionExecution, ActionResult},
};

pub(super) async fn write_file_action(
    path: &str,
    content: &str,
    sandbox: &RepoSandbox,
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
        Err(PathRejection::Unsafe(message)) => {
            return Ok(ActionExecution::unsafe_request(message));
        }
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

pub(super) async fn edit_file_action(
    path: &str,
    old_text: &str,
    new_text: &str,
    sandbox: &RepoSandbox,
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
        Err(PathRejection::Unsafe(message)) => {
            return Ok(ActionExecution::unsafe_request(message));
        }
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
