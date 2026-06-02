use std::path::Path;

use super::{
    content::{action_file_content_for_prompt, read_file_action, read_file_range_action},
    git::{git_changed_files, git_diff_stat},
    limits::ExecutionLimits,
    sandbox::RepoSandbox,
    search::search_action,
    types::{ActionExecution, ActionResult, ChangeAction, DoneStatus, LoopStop},
    write::{edit_file_action, write_file_action},
};

pub(super) async fn execute_action(
    action: &ChangeAction,
    sandbox: &RepoSandbox,
    diff_stat_path: &Path,
    limits: &ExecutionLimits,
) -> anyhow::Result<ActionExecution> {
    match action {
        ChangeAction::ReadFile { path } => read_file_action(path, sandbox, limits).await,
        ChangeAction::ReadFileRange {
            path,
            start_line,
            end_line,
        } => read_file_range_action(path, *start_line, *end_line, sandbox, limits).await,
        ChangeAction::Search { query, path } => {
            search_action(query, path.as_deref(), sandbox, limits).await
        }
        ChangeAction::WriteFile { path, content } => {
            write_file_action(path, content, sandbox, diff_stat_path, limits).await
        }
        ChangeAction::EditFile {
            path,
            old_text,
            new_text,
        } => edit_file_action(path, old_text, new_text, sandbox, diff_stat_path, limits).await,
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

pub(super) async fn repeated_action_result(
    action: &ChangeAction,
    sandbox: &RepoSandbox,
    limits: &ExecutionLimits,
) -> anyhow::Result<ActionResult> {
    let (status, message) = match action {
        ChangeAction::ReadFile { .. } => {
            (
                RepeatedActionStatus::Ok,
                "same read_file action was just attempted. The file content is included again below; do not read this same file again. Use this content, use read_file_range/search for a different target, or choose an edit_file/write_file/done action.".to_owned(),
            )
        }
        ChangeAction::ReadFileRange { .. } => {
            (
                RepeatedActionStatus::Ok,
                "same read_file_range action was just attempted. The requested line range is included again below; do not read this same range again. Use it now, or choose a different range/search/edit action.".to_owned(),
            )
        }
        ChangeAction::Search { .. } => {
            (
                RepeatedActionStatus::Ok,
                "same search action was just attempted; do not repeat this same search. Use the prior matches and choose a different action.".to_owned(),
            )
        }
        ChangeAction::WriteFile { .. } | ChangeAction::EditFile { .. } => {
            (
                RepeatedActionStatus::Error,
                "same write/edit action was just attempted. If the previous result was ok, that change is already applied; do not repeat it. Inspect the current file content below, address the next requested feedback item, or return done only after all feedback is complete.".to_owned(),
            )
        }
        ChangeAction::Done { .. } => {
            (
                RepeatedActionStatus::Error,
                "same done action was just attempted; choose a different action only if more work remains".to_owned(),
            )
        }
    };

    let mut result = match status {
        RepeatedActionStatus::Ok => ActionResult::ok(message),
        RepeatedActionStatus::Error => ActionResult::error(message),
    };

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

#[derive(Debug, Clone, Copy)]
enum RepeatedActionStatus {
    Ok,
    Error,
}
