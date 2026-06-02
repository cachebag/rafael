use std::{collections::BTreeSet, path::Path, process::Stdio, time::Duration};

use anyhow::{Context, bail};
use tokio::{process::Command, time::timeout};
use tracing::warn;

use super::sandbox::sanitize_repo_path;

pub(super) async fn git_ls_files(checkout_path: &Path) -> anyhow::Result<Vec<String>> {
    let output = run_git_capture(
        checkout_path,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ],
    )
    .await?;

    let mut files = Vec::new();
    for entry in output.split('\0') {
        if entry.is_empty() {
            continue;
        }
        if sanitize_repo_path(entry).is_ok() {
            files.push(entry.to_owned());
        } else {
            warn!(path = %entry, "ignored unsafe path from git ls-files");
        }
    }

    Ok(files)
}

pub(super) async fn git_changed_files(checkout_path: &Path) -> anyhow::Result<Vec<String>> {
    let output = run_git_capture(
        checkout_path,
        &["status", "--porcelain", "--untracked-files=normal"],
    )
    .await?;
    let mut files = BTreeSet::new();

    for line in output.lines() {
        if line.len() < 4 {
            continue;
        }
        let path = line[3..].trim();
        let path = path
            .split_once(" -> ")
            .map(|(_, new_path)| new_path)
            .unwrap_or(path)
            .trim_matches('"');
        if sanitize_repo_path(path).is_ok() {
            files.insert(path.to_owned());
        }
    }

    Ok(files.into_iter().collect())
}

pub(super) async fn git_diff_stat(checkout_path: &Path) -> anyhow::Result<String> {
    run_git_capture(checkout_path, &["diff", "--stat"]).await
}

async fn run_git_capture(checkout_path: &Path, args: &[&str]) -> anyhow::Result<String> {
    let output = timeout(
        Duration::from_secs(GIT_COMMAND_TIMEOUT_SECS),
        Command::new("git")
            .current_dir(checkout_path)
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null())
            .args(args)
            .output(),
    )
    .await
    .context("git command timed out")?
    .context("failed to execute git")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git command failed with status {}: {stderr}", output.status);
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

const GIT_COMMAND_TIMEOUT_SECS: u64 = 30;
