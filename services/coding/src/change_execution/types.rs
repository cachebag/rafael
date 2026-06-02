use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::{
    limits::MAX_HISTORY_ACTION_TEXT_CHARS,
    text::{truncate_for_prompt, truncate_text},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeExecutionOutcome {
    pub status: ChangeExecutionStatus,
    pub summary: String,
    pub question: Option<String>,
    pub transcript_path: PathBuf,
    pub diff_stat_path: Option<PathBuf>,
    pub changed_files: Vec<String>,
    pub iterations: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChangeExecutionStatus {
    Completed,
    Blocked,
    MaxIterations,
    MaxRuntime,
    UnsafeRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(super) enum ChangeAction {
    ReadFile {
        path: String,
    },
    ReadFileRange {
        path: String,
        start_line: usize,
        end_line: usize,
    },
    Search {
        #[serde(alias = "pattern")]
        query: String,
        #[serde(default)]
        path: Option<String>,
    },
    WriteFile {
        path: String,
        content: String,
    },
    EditFile {
        path: String,
        old_text: String,
        new_text: String,
    },
    Done {
        #[serde(default)]
        status: DoneStatus,
        summary: String,
        #[serde(default)]
        question: Option<String>,
    },
}

impl ChangeAction {
    pub(super) fn for_prompt(&self) -> serde_json::Value {
        match self {
            Self::ReadFile { path } => serde_json::json!({
                "action": "read_file",
                "path": path,
            }),
            Self::ReadFileRange {
                path,
                start_line,
                end_line,
            } => serde_json::json!({
                "action": "read_file_range",
                "path": path,
                "start_line": start_line,
                "end_line": end_line,
            }),
            Self::Search { query, path } => serde_json::json!({
                "action": "search",
                "query": query,
                "path": path,
            }),
            Self::WriteFile { path, content } => serde_json::json!({
                "action": "write_file",
                "path": path,
                "content_bytes": content.len(),
                "content_preview": truncate_text(content, MAX_HISTORY_ACTION_TEXT_CHARS),
            }),
            Self::EditFile {
                path,
                old_text,
                new_text,
            } => serde_json::json!({
                "action": "edit_file",
                "path": path,
                "old_text_bytes": old_text.len(),
                "old_text_preview": truncate_text(old_text, MAX_HISTORY_ACTION_TEXT_CHARS),
                "new_text_bytes": new_text.len(),
                "new_text_preview": truncate_text(new_text, MAX_HISTORY_ACTION_TEXT_CHARS),
            }),
            Self::Done {
                status,
                summary,
                question,
            } => serde_json::json!({
                "action": "done",
                "status": status,
                "summary": summary,
                "question": question,
            }),
        }
    }

    pub(super) fn target_path(&self) -> Option<&str> {
        match self {
            Self::ReadFile { path }
            | Self::ReadFileRange { path, .. }
            | Self::WriteFile { path, .. }
            | Self::EditFile { path, .. } => Some(path),
            Self::Search { path, .. } => path.as_deref(),
            Self::Done { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub(super) enum DoneStatus {
    #[default]
    Completed,
    Blocked,
}

#[derive(Debug)]
pub(super) struct ActionExecution {
    pub(super) result: ActionResult,
    pub(super) stop: Option<LoopStop>,
}

impl ActionExecution {
    pub(super) fn continue_with(result: ActionResult) -> Self {
        Self { result, stop: None }
    }

    pub(super) fn unsafe_request(message: String) -> Self {
        Self {
            result: ActionResult::unsafe_request(message.clone()),
            stop: Some(LoopStop::Unsafe { message }),
        }
    }
}

#[derive(Debug)]
pub(super) enum LoopStop {
    Completed {
        summary: String,
    },
    Blocked {
        summary: String,
        question: Option<String>,
    },
    Unsafe {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ActionResult {
    pub(super) status: ActionResultStatus,
    pub(super) message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) matches: Option<Vec<SearchMatch>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) diff_stat: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) changed_files: Option<Vec<String>>,
}

impl ActionResult {
    pub(super) fn ok(message: String) -> Self {
        Self {
            status: ActionResultStatus::Ok,
            message,
            content: None,
            bytes: None,
            matches: None,
            diff_stat: None,
            changed_files: None,
        }
    }

    pub(super) fn error(message: String) -> Self {
        Self {
            status: ActionResultStatus::Error,
            message,
            content: None,
            bytes: None,
            matches: None,
            diff_stat: None,
            changed_files: None,
        }
    }

    pub(super) fn unsafe_request(message: String) -> Self {
        Self {
            status: ActionResultStatus::Unsafe,
            message,
            content: None,
            bytes: None,
            matches: None,
            diff_stat: None,
            changed_files: None,
        }
    }

    pub(super) fn with_content(mut self, content: String, bytes: u64) -> Self {
        self.content = Some(content);
        self.bytes = Some(bytes);
        self
    }

    pub(super) fn with_matches(mut self, matches: Vec<SearchMatch>) -> Self {
        self.matches = Some(matches);
        self
    }

    pub(super) fn with_diff_stat(mut self, diff_stat: String) -> Self {
        self.diff_stat = Some(diff_stat);
        self
    }

    pub(super) fn with_changed_files(mut self, changed_files: Vec<String>) -> Self {
        self.changed_files = Some(changed_files);
        self
    }

    pub(super) fn for_prompt(&self) -> serde_json::Value {
        let mut result = self.clone();
        if let Some(content) = result.content.as_deref() {
            result.content = Some(truncate_for_prompt(content));
        }
        if let Some(diff_stat) = result.diff_stat.as_deref() {
            result.diff_stat = Some(truncate_for_prompt(diff_stat));
        }

        serde_json::to_value(result).unwrap_or_else(|_| {
            serde_json::json!({
                "status": "error",
                "message": "failed to serialize action result for prompt"
            })
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ActionResultStatus {
    Ok,
    Error,
    Unsafe,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SearchMatch {
    pub(super) path: String,
    pub(super) line: usize,
    pub(super) preview: String,
    pub(super) read_start_line: usize,
    pub(super) read_end_line: usize,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct PromptHistoryEntry {
    pub(super) iteration: usize,
    pub(super) action: serde_json::Value,
    pub(super) result: serde_json::Value,
}
