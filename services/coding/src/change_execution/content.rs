use anyhow::Context;

use super::{
    limits::ExecutionLimits,
    sandbox::{PathRejection, RepoSandbox},
    types::{ActionExecution, ActionResult, ChangeAction},
};

pub(super) async fn action_file_content_for_prompt(
    action: &ChangeAction,
    sandbox: &RepoSandbox,
    limits: &ExecutionLimits,
) -> Option<(String, u64)> {
    match action {
        ChangeAction::ReadFileRange {
            path,
            start_line,
            end_line,
        } => range_file_content_for_prompt(path, *start_line, *end_line, sandbox, limits)
            .await
            .map(|content| {
                let bytes = content.len() as u64;
                (content, bytes)
            }),
        _ => file_content_for_prompt(action.target_path()?, sandbox, limits).await,
    }
}

pub(super) async fn file_content_for_prompt(
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

pub(super) async fn read_file_action(
    path: &str,
    sandbox: &RepoSandbox,
    limits: &ExecutionLimits,
) -> anyhow::Result<ActionExecution> {
    let absolute_path = match sandbox.existing_file(path).await {
        Ok(path) => path,
        Err(PathRejection::Unsafe(message)) => {
            return Ok(ActionExecution::unsafe_request(message));
        }
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
                "file is too large to read safely ({} bytes > {} bytes). Use read_file_range with line numbers or search for a specific symbol/text instead.",
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

pub(super) async fn read_file_range_action(
    path: &str,
    start_line: usize,
    end_line: usize,
    sandbox: &RepoSandbox,
    limits: &ExecutionLimits,
) -> anyhow::Result<ActionExecution> {
    if start_line == 0 || end_line < start_line {
        return Ok(ActionExecution::continue_with(ActionResult::error(
            "read_file_range requires start_line >= 1 and end_line >= start_line".to_owned(),
        )));
    }

    let Some(content) =
        range_file_content_for_prompt(path, start_line, end_line, sandbox, limits).await
    else {
        return Ok(ActionExecution::continue_with(ActionResult::error(
            "could not read requested range; verify the path and use a smaller line range"
                .to_owned(),
        )));
    };

    Ok(ActionExecution::continue_with(
        ActionResult::ok(format!("read {path}:{start_line}-{end_line}"))
            .with_content(content.clone(), content.len() as u64),
    ))
}

async fn range_file_content_for_prompt(
    path: &str,
    start_line: usize,
    end_line: usize,
    sandbox: &RepoSandbox,
    limits: &ExecutionLimits,
) -> Option<String> {
    if start_line == 0 || end_line < start_line {
        return None;
    }

    let (content, _) = file_content_for_prompt(path, sandbox, limits).await?;
    format_line_range(&content, start_line, end_line, limits.max_read_lines)
}

fn format_line_range(
    content: &str,
    start_line: usize,
    end_line: usize,
    max_lines: usize,
) -> Option<String> {
    if start_line == 0 || end_line < start_line || max_lines == 0 {
        return None;
    }

    let selected = content
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let line_number = index + 1;
            (line_number >= start_line && line_number <= end_line)
                .then(|| format!("{line_number}: {line}"))
        })
        .take(max_lines)
        .collect::<Vec<_>>();

    if selected.is_empty() {
        return None;
    }

    let mut content = selected.join("\n");
    if end_line.saturating_sub(start_line) + 1 > max_lines {
        content.push_str("\n...[range truncated]");
    }
    Some(content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_line_ranges_with_line_numbers() {
        let content = "one\ntwo\nthree\nfour\n";

        let range = format_line_range(content, 2, 3, 10).unwrap();

        assert_eq!(range, "2: two\n3: three");
    }

    #[test]
    fn line_range_formatting_honors_max_lines() {
        let content = "one\ntwo\nthree\nfour\n";

        let range = format_line_range(content, 1, 4, 2).unwrap();

        assert_eq!(range, "1: one\n2: two\n...[range truncated]");
    }
}
