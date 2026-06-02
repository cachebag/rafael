use super::{
    git::git_ls_files,
    limits::{ExecutionLimits, MAX_SEARCH_PREVIEW_CHARS, SEARCH_MATCH_CANDIDATE_MULTIPLIER},
    sandbox::{PathRejection, RepoSandbox, is_sensitive_path},
    text::truncate_text,
    types::{ActionExecution, ActionResult, SearchMatch},
};

pub(super) async fn search_action(
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
        Err(PathRejection::Unsafe(message)) => {
            return Ok(ActionExecution::unsafe_request(message));
        }
        Err(PathRejection::NotFound(message)) => {
            return Ok(ActionExecution::continue_with(ActionResult::error(message)));
        }
    };

    let files = git_ls_files(&sandbox.checkout_path).await?;
    let query_lower = query.to_ascii_lowercase();
    let mut matches = Vec::new();
    let max_candidates = limits
        .max_search_results
        .saturating_mul(SEARCH_MATCH_CANDIDATE_MULTIPLIER);

    for file in files {
        if matches.len() >= max_candidates {
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
                let line_number = line_index + 1;
                let (read_start_line, read_end_line) = suggested_read_range(line_number);
                matches.push(SearchMatch {
                    path: file.clone(),
                    line: line_number,
                    preview: truncate_text(line.trim(), MAX_SEARCH_PREVIEW_CHARS),
                    read_start_line,
                    read_end_line,
                });
                if matches.len() >= max_candidates {
                    break;
                }
            }
        }
    }

    matches.sort_by(|left, right| search_match_rank(left).cmp(&search_match_rank(right)));
    matches.truncate(limits.max_search_results);

    let result = ActionResult::ok(format!(
        "found {} match(es) for literal query `{query}`. To inspect a match, call read_file_range with that match's path, read_start_line, and read_end_line.",
        matches.len()
    ))
    .with_matches(matches);

    Ok(ActionExecution::continue_with(result))
}

fn suggested_read_range(line_number: usize) -> (usize, usize) {
    (
        line_number.saturating_sub(20).max(1),
        line_number.saturating_add(80),
    )
}

fn search_match_rank(search_match: &SearchMatch) -> (u8, &str, usize) {
    let preview = search_match.preview.trim_start();
    let rank = if preview.starts_with("fn ")
        || preview.starts_with("async fn ")
        || preview.starts_with("pub fn ")
        || preview.starts_with("pub async fn ")
        || preview.starts_with("struct ")
        || preview.starts_with("pub struct ")
        || preview.starts_with("enum ")
        || preview.starts_with("pub enum ")
        || preview.starts_with("impl ")
    {
        0
    } else {
        1
    };

    (rank, search_match.path.as_str(), search_match.line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggested_read_ranges_include_context_before_match() {
        assert_eq!(suggested_read_range(10), (1, 90));
        assert_eq!(suggested_read_range(299), (279, 379));
    }

    #[test]
    fn search_match_rank_prefers_definitions() {
        let mut matches = [
            SearchMatch {
                path: "src/main.rs".to_owned(),
                line: 20,
                preview: "let result = repeated_action_result();".to_owned(),
                read_start_line: 1,
                read_end_line: 100,
            },
            SearchMatch {
                path: "src/main.rs".to_owned(),
                line: 299,
                preview: "async fn repeated_action_result(".to_owned(),
                read_start_line: 279,
                read_end_line: 379,
            },
        ];

        matches.sort_by(|left, right| search_match_rank(left).cmp(&search_match_rank(right)));

        assert_eq!(matches[0].line, 299);
    }
}
