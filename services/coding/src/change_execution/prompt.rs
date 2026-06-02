use anyhow::Context;

use crate::{
    github::{IssueInfo, RepositoryInfo},
    repo_context::RepoContext,
};

use super::{history::prompt_history_tail, limits::ExecutionLimits, types::PromptHistoryEntry};

pub(super) fn build_action_prompt(
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
        "Repository: {}\nDefault branch: {}\nTarget branch: {}\nIssue #{}: {}\nLabels: {}\nIssue body:\n{}\n\nImplementation plan:\n{}\n\nRepository context JSON:\n{}\n\nPrior tool history JSON:\n{}\n\nYou are editing the prepared local branch through a bounded tool loop. Choose exactly one next action and return exactly one JSON object with no Markdown, comments, or extra text. Available actions:\n- {{\"action\":\"read_file\",\"path\":\"relative/path\"}}\n- {{\"action\":\"read_file_range\",\"path\":\"relative/path\",\"start_line\":1,\"end_line\":80}}\n- {{\"action\":\"search\",\"query\":\"case-insensitive literal text\",\"path\":\"optional/relative/scope\"}}\n- {{\"action\":\"write_file\",\"path\":\"relative/path\",\"content\":\"complete file contents\"}}\n- {{\"action\":\"edit_file\",\"path\":\"relative/path\",\"old_text\":\"exact text appearing once\",\"new_text\":\"replacement text\"}}\n- {{\"action\":\"done\",\"status\":\"completed\",\"summary\":\"what changed\"}}\n- {{\"action\":\"done\",\"status\":\"blocked\",\"summary\":\"why blocked\",\"question\":\"what you need clarified\"}}\n\nConstraints:\n- Paths must be repository-relative, inside the checkout, and must not use .git, parent traversal, absolute paths, or known secret files.\n- Do not request shell commands, commits, pushes, branch changes, package installs, or network calls.\n- Keep changes minimal and focused on the issue. Prefer search/read_file_range for large files or known symbols; use read_file only for small files.\n- Never repeat an identical read_file, read_file_range, or search action after the prior result was ok. Use the returned content/matches, choose a different range/search, edit a file, or finish.\n- Treat trusted review feedback in the implementation plan as a checklist. After a successful write/edit, move to the next unaddressed feedback item. Do not repeat the same edit.\n- If old_text no longer matches, the file may already be changed; use the current file content from tool history, read the file, or choose the next file.\n- Return done with status completed only after every requested feedback item is addressed.\n- write_file content must be at most {} bytes; whole-file reads are capped at {} bytes; range reads are capped at {} lines; the run may change at most {} files.\n- If the requested change cannot be completed safely with these actions, return done with status blocked.",
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
        limits.max_read_lines,
        limits.max_changed_files
    ))
}
