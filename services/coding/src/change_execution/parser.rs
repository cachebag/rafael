use anyhow::Context;

use super::types::ChangeAction;

pub(super) fn parse_action(raw_response: &str) -> anyhow::Result<ChangeAction> {
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
    fn parses_read_file_range_action() {
        let action = parse_action(
            r#"{"action":"read_file_range","path":"src/main.rs","start_line":10,"end_line":20}"#,
        )
        .unwrap();

        assert!(matches!(
            action,
            ChangeAction::ReadFileRange {
                path,
                start_line: 10,
                end_line: 20,
            } if path == "src/main.rs"
        ));
    }
}
