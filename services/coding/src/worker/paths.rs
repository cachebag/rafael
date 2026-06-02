use std::path::PathBuf;

use crate::{
    config::AppConfig,
    types::{IssueTrigger, PullRequestRevisionTrigger},
};

pub(super) fn run_dir(config: &AppConfig, trigger: &IssueTrigger) -> PathBuf {
    config
        .workspace
        .runs_dir
        .join(trigger.repo.safe_dir_name())
        .join(format!("issue-{}", trigger.issue_number))
        .join(safe_path_component(&trigger.run_id))
}

pub(super) fn pull_request_run_dir(
    config: &AppConfig,
    trigger: &PullRequestRevisionTrigger,
) -> PathBuf {
    config
        .workspace
        .runs_dir
        .join(trigger.repo.safe_dir_name())
        .join(format!("pr-{}", trigger.pull_number))
        .join(safe_path_component(&trigger.run_id))
}

pub(super) fn safe_path_component(value: &str) -> String {
    let mut safe = String::new();
    let mut last_was_dash = false;

    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            safe.push(ch);
            last_was_dash = false;
        } else if !last_was_dash && !safe.is_empty() {
            safe.push('-');
            last_was_dash = true;
        }

        if safe.len() >= 96 {
            break;
        }
    }

    let safe = safe.trim_matches('-');
    if safe.is_empty() {
        "run".to_owned()
    } else {
        safe.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_path_component_strips_separators() {
        assert_eq!(safe_path_component("../abc/def"), "abc-def");
    }
}
