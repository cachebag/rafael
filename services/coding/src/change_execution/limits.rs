use std::time::Duration;

use serde::Serialize;

use crate::config::AppConfig;

#[derive(Debug, Clone, Copy, Serialize)]
pub(super) struct ExecutionLimits {
    pub(super) max_iterations: usize,
    #[serde(skip)]
    pub(super) max_runtime: Duration,
    pub(super) max_runtime_seconds: u64,
    pub(super) max_read_bytes: u64,
    pub(super) max_read_lines: usize,
    pub(super) max_write_bytes: usize,
    pub(super) max_changed_files: usize,
    pub(super) max_search_results: usize,
}

impl ExecutionLimits {
    pub(super) fn from_config(config: &AppConfig) -> Self {
        Self {
            max_iterations: config.workspace.max_tool_iterations,
            max_runtime: Duration::from_secs(config.workspace.max_tool_runtime_seconds),
            max_runtime_seconds: config.workspace.max_tool_runtime_seconds,
            max_read_bytes: config.workspace.max_file_read_bytes,
            max_read_lines: MAX_RANGE_READ_LINES,
            max_write_bytes: config.workspace.max_write_bytes,
            max_changed_files: config.workspace.max_changed_files,
            max_search_results: MAX_SEARCH_RESULTS,
        }
    }
}

pub(super) const MAX_SEARCH_RESULTS: usize = 40;
pub(super) const SEARCH_MATCH_CANDIDATE_MULTIPLIER: usize = 4;
pub(super) const MAX_SEARCH_PREVIEW_CHARS: usize = 240;
pub(super) const MAX_RANGE_READ_LINES: usize = 160;
pub(super) const MAX_HISTORY_ACTION_TEXT_CHARS: usize = 2_000;
pub(super) const MAX_HISTORY_VALUE_CHARS: usize = 4_000;
pub(super) const MAX_PROMPT_HISTORY_ENTRIES: usize = 8;
pub(super) const MAX_CONSECUTIVE_IDENTICAL_ACTIONS: usize = 3;
