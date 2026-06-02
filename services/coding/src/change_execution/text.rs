use super::limits::MAX_HISTORY_VALUE_CHARS;

pub(super) fn truncate_for_prompt(value: &str) -> String {
    truncate_text(value, MAX_HISTORY_VALUE_CHARS)
}

pub(super) fn truncate_text(value: &str, max_chars: usize) -> String {
    let mut truncated = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        truncated.push_str("\n...[truncated]");
    }
    truncated
}
