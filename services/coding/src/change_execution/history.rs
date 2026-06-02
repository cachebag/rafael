use super::{
    limits::MAX_PROMPT_HISTORY_ENTRIES,
    types::{ActionResult, ChangeAction, PromptHistoryEntry},
};

pub(super) fn prompt_history_entry(
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

pub(super) fn prompt_history_tail(history: &[PromptHistoryEntry]) -> &[PromptHistoryEntry] {
    if history.len() <= MAX_PROMPT_HISTORY_ENTRIES {
        history
    } else {
        &history[history.len() - MAX_PROMPT_HISTORY_ENTRIES..]
    }
}

pub(super) fn consecutive_action_repetitions(
    history: &[PromptHistoryEntry],
    action: &serde_json::Value,
) -> usize {
    history
        .iter()
        .rev()
        .take_while(|entry| entry.action == *action)
        .count()
}

pub(super) fn should_finish_after_repeated_write_edit(
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

#[cfg(test)]
mod tests {
    use super::*;

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
