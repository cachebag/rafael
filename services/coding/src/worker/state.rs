use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::{
    types::{RepoRef, TriggerKind},
    verification::{VerificationCommandResult, VerificationStatus},
};

#[derive(Debug, Serialize, Deserialize)]
pub struct RunState {
    pub run_id: String,
    pub repo: RepoRef,
    pub issue_number: u64,
    pub trigger: TriggerKind,
    pub actor: Option<String>,
    pub installation_id: u64,
    pub status: RunStatus,
    pub issue_title: Option<String>,
    pub issue_url: Option<String>,
    pub default_branch: Option<String>,
    pub branch_name: Option<String>,
    pub worktree_path: Option<PathBuf>,
    pub context_path: Option<PathBuf>,
    pub plan_path: Option<PathBuf>,
    pub transcript_path: Option<PathBuf>,
    pub diff_stat_path: Option<PathBuf>,
    pub changed_files: Vec<String>,
    pub implementation_summary: Option<String>,
    pub verification_path: Option<PathBuf>,
    pub verification_status: Option<VerificationStatus>,
    pub verification_summary: Option<String>,
    pub verification_results: Vec<VerificationCommandResult>,
    pub repair_attempted: bool,
    pub repair_summary: Option<String>,
    pub commit_sha: Option<String>,
    pub pr_url: Option<String>,
    pub pr_number: Option<u64>,
    pub publish_summary: Option<String>,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Received,
    Authenticated,
    Prepared,
    Planned,
    Implementing,
    Verifying,
    Verified,
    Publishing,
    Published,
    Completed,
    Blocked,
    Failed,
}

pub(super) async fn write_state(run_dir: &Path, state: &RunState) -> anyhow::Result<()> {
    let state_path = run_dir.join("state.json");
    let body = serde_json::to_vec_pretty(state).context("failed to serialize run state")?;
    tokio::fs::write(&state_path, body)
        .await
        .with_context(|| format!("failed to write {}", state_path.display()))
}

pub(super) fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}
