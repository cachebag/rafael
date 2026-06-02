use std::{
    fmt,
    fs::OpenOptions,
    io::{Error, ErrorKind, Write},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use anyhow::{Context, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use tokio::{process::Command, task, time::timeout};
use tracing::{info, warn};

use crate::{
    config::AppConfig,
    github::{GitHubClient, InstallationToken, IssueInfo, RepositoryInfo},
    model::ModelClient,
    types::{IssueTrigger, RepoRef, TriggerKind},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunState {
    pub run_id: String,
    pub repo: RepoRef,
    pub issue_number: u64,
    pub trigger: TriggerKind,
    pub actor: Option<String>,
    #[serde(default)]
    pub installation_id: Option<u64>,
    pub status: RunStatus,
    pub issue_title: Option<String>,
    pub issue_url: Option<String>,
    pub default_branch: Option<String>,
    pub branch_name: Option<String>,
    pub worktree_path: Option<PathBuf>,
    pub plan_path: Option<PathBuf>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub pr_url: Option<String>,
    #[serde(default)]
    pub commit_sha: Option<String>,
    #[serde(default)]
    pub verification_summary: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Received,
    Claimed,
    Authenticated,
    Prepared,
    Planned,
    Implementing,
    Verifying,
    Published,
    Completed,
    Blocked,
    Cancelled,
    Failed,
}

impl RunStatus {
    fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Blocked | Self::Cancelled | Self::Failed
        )
    }
}

impl fmt::Display for RunStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Received => write!(f, "received"),
            Self::Claimed => write!(f, "claimed"),
            Self::Authenticated => write!(f, "authenticated"),
            Self::Prepared => write!(f, "prepared"),
            Self::Planned => write!(f, "planned"),
            Self::Implementing => write!(f, "implementing"),
            Self::Verifying => write!(f, "verifying"),
            Self::Published => write!(f, "published"),
            Self::Completed => write!(f, "completed"),
            Self::Blocked => write!(f, "blocked"),
            Self::Cancelled => write!(f, "cancelled"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveRun {
    pub run_id: String,
    pub repo: RepoRef,
    pub issue_number: u64,
    pub trigger: TriggerKind,
    pub actor: Option<String>,
    pub created_at: String,
}

#[derive(Debug)]
pub struct RunClaim {
    trigger: IssueTrigger,
    run_dir: PathBuf,
    issue_dir: PathBuf,
    lock_path: PathBuf,
}

#[derive(Debug)]
pub enum RunClaimDecision {
    Claimed(RunClaim),
    Duplicate {
        reason: String,
        active_run: Option<ActiveRun>,
    },
}

#[derive(Debug)]
pub struct IssueStatus {
    pub repo: RepoRef,
    pub issue_number: u64,
    pub issue_dir: PathBuf,
    pub active_run: Option<ActiveRun>,
    pub cancel_requested: bool,
    pub latest_state: Option<RunState>,
}

#[derive(Debug)]
pub struct CancelResult {
    pub active_run: ActiveRun,
    pub marker_path: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
struct CancelRequest {
    run_id: String,
    requested_at: String,
}

const STALE_LOCK_GRACE_SECS: u64 = 10 * 60;
const MAX_RUN_ID_LEN: usize = 128;

fn ensure_safe_run_id(run_id: &str) -> anyhow::Result<()> {
    if is_safe_run_id(run_id) {
        return Ok(());
    }

    bail!("run_id must be a single safe path component");
}

fn is_safe_run_id(run_id: &str) -> bool {
    !run_id.is_empty()
        && run_id.len() <= MAX_RUN_ID_LEN
        && run_id != "."
        && run_id != ".."
        && run_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

pub async fn claim_issue_run(
    config: &AppConfig,
    trigger: IssueTrigger,
    event_body: Option<&[u8]>,
) -> anyhow::Result<RunClaimDecision> {
    ensure_safe_run_id(&trigger.run_id)?;

    let issue_dir = issue_dir(config, &trigger.repo, trigger.issue_number);
    let run_dir = run_dir_for_issue_dir(&issue_dir, &trigger.run_id);
    let lock_path = active_lock_path_for_issue_dir(&issue_dir);

    tokio::fs::create_dir_all(&issue_dir)
        .await
        .with_context(|| format!("failed to create {}", issue_dir.display()))?;

    if tokio::fs::try_exists(&run_dir)
        .await
        .with_context(|| format!("failed to inspect {}", run_dir.display()))?
    {
        return Ok(RunClaimDecision::Duplicate {
            reason: format!("run `{}` already exists", trigger.run_id),
            active_run: read_active_run(&lock_path).await.ok().flatten(),
        });
    }

    let active_run = ActiveRun {
        run_id: trigger.run_id.clone(),
        repo: trigger.repo.clone(),
        issue_number: trigger.issue_number,
        trigger: trigger.trigger,
        actor: trigger.actor.clone(),
        created_at: now_rfc3339(),
    };

    let mut recovered_stale_lock = false;
    loop {
        match create_active_lock(&lock_path, &active_run).await {
            Ok(()) => break,
            Err(err) if err.kind() == ErrorKind::AlreadyExists && !recovered_stale_lock => {
                let active_run = match read_active_run(&lock_path).await {
                    Ok(active_run) => active_run,
                    Err(err) => {
                        warn!(
                            path = %lock_path.display(),
                            error = %err,
                            "removing unreadable active run lock"
                        );
                        remove_active_lock_best_effort(&lock_path).await;
                        recovered_stale_lock = true;
                        continue;
                    }
                };

                let Some(active_run) = active_run.as_ref() else {
                    warn!(
                        path = %lock_path.display(),
                        "removing empty active run lock"
                    );
                    remove_active_lock_best_effort(&lock_path).await;
                    recovered_stale_lock = true;
                    continue;
                };

                if recover_stale_active_lock(config, &issue_dir, &lock_path, active_run).await? {
                    recovered_stale_lock = true;
                    continue;
                }

                return Ok(RunClaimDecision::Duplicate {
                    reason: "active run already exists".to_owned(),
                    active_run: Some(active_run.clone()),
                });
            }
            Err(err) if err.kind() == ErrorKind::AlreadyExists => {
                return Ok(RunClaimDecision::Duplicate {
                    reason: "active run already exists".to_owned(),
                    active_run: read_active_run(&lock_path).await.ok().flatten(),
                });
            }
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("failed to create {}", lock_path.display()));
            }
        }
    }

    if let Err(err) = tokio::fs::create_dir_all(&run_dir).await {
        remove_active_lock_best_effort(&lock_path).await;
        remove_run_dir_best_effort(&run_dir).await;
        return Err(err).with_context(|| format!("failed to create {}", run_dir.display()));
    }

    if let Some(body) = event_body {
        let event_path = run_dir.join("event.json");
        if let Err(err) = tokio::fs::write(&event_path, body).await {
            remove_active_lock_best_effort(&lock_path).await;
            remove_run_dir_best_effort(&run_dir).await;
            return Err(err).with_context(|| format!("failed to write {}", event_path.display()));
        }
    }

    Ok(RunClaimDecision::Claimed(RunClaim {
        trigger,
        run_dir,
        issue_dir,
        lock_path,
    }))
}

pub async fn run_issue_triggered(config: AppConfig, trigger: IssueTrigger) -> anyhow::Result<()> {
    match claim_issue_run(&config, trigger, None).await? {
        RunClaimDecision::Claimed(claim) => run_claimed_issue(config, claim).await,
        RunClaimDecision::Duplicate { reason, active_run } => {
            if let Some(active_run) = active_run {
                info!(
                    repo = %active_run.repo,
                    issue = active_run.issue_number,
                    run_id = %active_run.run_id,
                    %reason,
                    "ignored duplicate coding run"
                );
            } else {
                info!(%reason, "ignored duplicate coding run");
            }
            Ok(())
        }
    }
}

pub async fn run_claimed_issue(config: AppConfig, claim: RunClaim) -> anyhow::Result<()> {
    let max_run = Duration::from_secs(config.workspace.max_run_minutes * 60);
    let run_id = claim.trigger.run_id.clone();
    let run_dir = claim.run_dir.clone();
    let issue_dir = claim.issue_dir.clone();
    let lock_path = claim.lock_path.clone();

    let result = match timeout(max_run, run_issue_triggered_inner(config, &claim)).await {
        Ok(result) => result,
        Err(_) => Err(anyhow::anyhow!("coding run exceeded configured time limit")),
    };

    if let Err(err) = &result {
        let status = match cancel_requested_for_issue_dir(&issue_dir).await {
            Ok(true) => RunStatus::Cancelled,
            Ok(false) => RunStatus::Failed,
            Err(status_err) => {
                warn!(
                    run_id = %run_id,
                    error = %status_err,
                    "failed to inspect cancel marker"
                );
                RunStatus::Failed
            }
        };
        if let Err(state_err) = mark_terminal(&run_dir, status, Some(err.to_string())).await {
            warn!(
                run_id = %run_id,
                error = %state_err,
                "failed to persist terminal run state"
            );
        }
    }

    if let Err(err) = release_active_lock(&lock_path, &run_id).await {
        warn!(run_id = %run_id, error = %err, "failed to release active run lock");
    }
    clear_cancel_marker_best_effort(&issue_dir, &run_id).await;

    result
}

pub async fn issue_status(
    config: &AppConfig,
    repo: &RepoRef,
    issue_number: u64,
) -> anyhow::Result<IssueStatus> {
    let issue_dir = issue_dir(config, repo, issue_number);
    let active_run = read_active_run(&active_lock_path_for_issue_dir(&issue_dir)).await?;
    let cancel_requested = cancel_requested_for_issue_dir(&issue_dir).await?;
    let latest_state = latest_run_state(&issue_dir).await?;

    Ok(IssueStatus {
        repo: repo.clone(),
        issue_number,
        issue_dir,
        active_run,
        cancel_requested,
        latest_state,
    })
}

pub async fn request_cancel(
    config: &AppConfig,
    repo: &RepoRef,
    issue_number: u64,
) -> anyhow::Result<CancelResult> {
    let issue_dir = issue_dir(config, repo, issue_number);
    let lock_path = active_lock_path_for_issue_dir(&issue_dir);
    let active_run = read_active_run(&lock_path)
        .await?
        .with_context(|| format!("no active run for {repo}#{issue_number}"))?;
    let marker_path = cancel_marker_path_for_issue_dir(&issue_dir);
    let request = CancelRequest {
        run_id: active_run.run_id.clone(),
        requested_at: now_rfc3339(),
    };
    let body = serde_json::to_vec_pretty(&request).context("failed to serialize cancel request")?;

    tokio::fs::write(&marker_path, body)
        .await
        .with_context(|| format!("failed to write {}", marker_path.display()))?;

    Ok(CancelResult {
        active_run,
        marker_path,
    })
}

async fn run_issue_triggered_inner(config: AppConfig, claim: &RunClaim) -> anyhow::Result<()> {
    let trigger = &claim.trigger;
    let created_at = now_rfc3339();
    let mut state = RunState {
        run_id: trigger.run_id.clone(),
        repo: trigger.repo.clone(),
        issue_number: trigger.issue_number,
        trigger: trigger.trigger,
        actor: trigger.actor.clone(),
        installation_id: trigger.installation_id.or(config.github.installation_id),
        status: RunStatus::Claimed,
        issue_title: None,
        issue_url: None,
        default_branch: trigger.default_branch.clone(),
        branch_name: None,
        worktree_path: None,
        plan_path: None,
        error: None,
        pr_url: None,
        commit_sha: None,
        verification_summary: None,
        created_at: created_at.clone(),
        updated_at: created_at,
    };
    write_state(&claim.run_dir, &state).await?;
    ensure_not_cancelled(claim).await?;

    let installation_id = state
        .installation_id
        .context("installation id must come from webhook payload, CLI, or config")?;
    let github = GitHubClient::new(&config.github)?;
    let model = ModelClient::new(&config.model)?;

    let token = github
        .create_installation_token(&config.github, installation_id, &trigger.repo)
        .await?;
    ensure_not_cancelled(claim).await?;
    info!(
        expires_at = %token.expires_at,
        run_id = %trigger.run_id,
        "received GitHub App installation token"
    );
    state.status = RunStatus::Authenticated;
    state.updated_at = now_rfc3339();
    write_state(&claim.run_dir, &state).await?;

    let repo = github.repository(&token, &trigger.repo).await?;
    let issue = github
        .issue(&token, &trigger.repo, trigger.issue_number)
        .await?;
    ensure_not_cancelled(claim).await?;

    if issue.pull_request.is_some() {
        bail!("issue #{} is a pull request, not an issue", issue.number);
    }

    post_comment_best_effort(
        &github,
        &token,
        &trigger.repo,
        trigger.issue_number,
        &format!(
            "Started a coding run for this issue.\n\nRun: `{}`\nTrigger: `{}`",
            trigger.run_id, trigger.trigger
        ),
    )
    .await;

    state.issue_title = Some(issue.title.clone());
    state.issue_url = Some(issue.html_url.clone());
    state.default_branch = Some(repo.default_branch.clone());

    let branch_name = branch_name_for_issue(&issue);
    let worktree_path =
        prepare_worktree(&config, &token, &trigger.repo, &repo, &branch_name).await?;
    ensure_not_cancelled(claim).await?;

    state.status = RunStatus::Prepared;
    state.branch_name = Some(branch_name.clone());
    state.worktree_path = Some(worktree_path);
    state.updated_at = now_rfc3339();
    write_state(&claim.run_dir, &state).await?;

    let plan = model.issue_plan(&repo, &issue, &branch_name).await?;
    ensure_not_cancelled(claim).await?;
    let plan_path = claim.run_dir.join("plan.md");
    tokio::fs::write(&plan_path, plan)
        .await
        .with_context(|| format!("failed to write {}", plan_path.display()))?;

    state.status = RunStatus::Planned;
    state.plan_path = Some(plan_path);
    state.updated_at = now_rfc3339();
    write_state(&claim.run_dir, &state).await?;

    post_comment_best_effort(
        &github,
        &token,
        &trigger.repo,
        trigger.issue_number,
        &format!(
            "Prepared branch `{branch_name}` and completed phase-one planning. Code editing and PR creation are not enabled in this build yet."
        ),
    )
    .await;

    ensure_not_cancelled(claim).await?;
    state.status = RunStatus::Completed;
    state.updated_at = now_rfc3339();
    write_state(&claim.run_dir, &state).await?;

    info!(
        repo = %trigger.repo,
        issue = trigger.issue_number,
        branch = %branch_name,
        run_id = %trigger.run_id,
        "prepared phase-one coding run"
    );

    Ok(())
}

async fn post_comment_best_effort(
    github: &GitHubClient,
    token: &InstallationToken,
    repo: &RepoRef,
    issue_number: u64,
    body: &str,
) {
    if let Err(err) = github
        .post_issue_comment(token, repo, issue_number, body)
        .await
    {
        warn!(repo = %repo, issue = issue_number, error = %err, "failed to post issue comment");
    }
}

async fn ensure_not_cancelled(claim: &RunClaim) -> anyhow::Result<()> {
    if cancel_requested_for_issue_dir(&claim.issue_dir).await? {
        bail!("run cancelled by operator");
    }
    Ok(())
}

fn issue_dir(config: &AppConfig, repo: &RepoRef, issue_number: u64) -> PathBuf {
    config
        .workspace
        .runs_dir
        .join(repo.safe_dir_name())
        .join(format!("issue-{issue_number}"))
}

fn run_dir_for_issue_dir(issue_dir: &Path, run_id: &str) -> PathBuf {
    issue_dir.join(run_id)
}

fn active_lock_path_for_issue_dir(issue_dir: &Path) -> PathBuf {
    issue_dir.join("active.lock")
}

fn cancel_marker_path_for_issue_dir(issue_dir: &Path) -> PathBuf {
    issue_dir.join("cancel.requested")
}

async fn cancel_requested_for_issue_dir(issue_dir: &Path) -> anyhow::Result<bool> {
    let marker_path = cancel_marker_path_for_issue_dir(issue_dir);
    tokio::fs::try_exists(&marker_path)
        .await
        .with_context(|| format!("failed to inspect {}", marker_path.display()))
}

async fn write_state(run_dir: &Path, state: &RunState) -> anyhow::Result<()> {
    let state_path = run_dir.join("state.json");
    let body = serde_json::to_vec_pretty(state).context("failed to serialize run state")?;
    tokio::fs::write(&state_path, body)
        .await
        .with_context(|| format!("failed to write {}", state_path.display()))
}

async fn mark_terminal(
    run_dir: &Path,
    status: RunStatus,
    error: Option<String>,
) -> anyhow::Result<()> {
    let state_path = run_dir.join("state.json");
    let body = tokio::fs::read(&state_path)
        .await
        .with_context(|| format!("failed to read {}", state_path.display()))?;
    let mut state: RunState = serde_json::from_slice(&body).context("failed to parse run state")?;
    state.status = status;
    state.error = error;
    state.updated_at = now_rfc3339();
    write_state(run_dir, &state).await
}

async fn read_run_state(run_dir: &Path) -> anyhow::Result<Option<RunState>> {
    let state_path = run_dir.join("state.json");
    let body = match tokio::fs::read(&state_path).await {
        Ok(body) => body,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(err).with_context(|| format!("failed to read {}", state_path.display()));
        }
    };

    serde_json::from_slice(&body)
        .map(Some)
        .with_context(|| format!("failed to parse {}", state_path.display()))
}

async fn latest_run_state(issue_dir: &Path) -> anyhow::Result<Option<RunState>> {
    let mut entries = match tokio::fs::read_dir(issue_dir).await {
        Ok(entries) => entries,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(err).with_context(|| format!("failed to read {}", issue_dir.display()));
        }
    };
    let mut latest: Option<RunState> = None;

    while let Some(entry) = entries
        .next_entry()
        .await
        .with_context(|| format!("failed to scan {}", issue_dir.display()))?
    {
        let file_type = entry
            .file_type()
            .await
            .with_context(|| format!("failed to inspect {}", entry.path().display()))?;
        if !file_type.is_dir() {
            continue;
        }

        let state = match read_run_state(&entry.path()).await {
            Ok(Some(state)) => state,
            Ok(None) => continue,
            Err(err) => {
                warn!(path = %entry.path().display(), error = %err, "ignored invalid run state");
                continue;
            }
        };

        match latest.as_ref() {
            Some(current) if current.created_at >= state.created_at => {}
            _ => latest = Some(state),
        }
    }

    Ok(latest)
}

async fn recover_stale_active_lock(
    config: &AppConfig,
    issue_dir: &Path,
    lock_path: &Path,
    active_run: &ActiveRun,
) -> anyhow::Result<bool> {
    let Some(reason) = stale_active_lock_reason(config, issue_dir, active_run).await? else {
        return Ok(false);
    };

    warn!(
        repo = %active_run.repo,
        issue = active_run.issue_number,
        run_id = %active_run.run_id,
        %reason,
        "recovering stale active run lock"
    );
    release_active_lock(lock_path, &active_run.run_id).await?;
    Ok(true)
}

async fn stale_active_lock_reason(
    config: &AppConfig,
    issue_dir: &Path,
    active_run: &ActiveRun,
) -> anyhow::Result<Option<String>> {
    if let Err(err) = ensure_safe_run_id(&active_run.run_id) {
        return Ok(Some(format!("active lock contains unsafe run_id: {err}")));
    }

    let run_dir = run_dir_for_issue_dir(issue_dir, &active_run.run_id);

    match read_run_state(&run_dir).await {
        Ok(Some(state)) if state.status.is_terminal() => {
            return Ok(Some(format!(
                "active lock references terminal run status `{}`",
                state.status
            )));
        }
        Ok(_) => {}
        Err(err) => {
            warn!(
                run_id = %active_run.run_id,
                path = %run_dir.display(),
                error = %err,
                "failed to inspect active run state for stale lock recovery"
            );
        }
    }

    let stale_after = stale_lock_after(config);
    let Some(age) = active_lock_age(active_run) else {
        return Ok(Some("active lock has invalid created_at".to_owned()));
    };

    if age > stale_after {
        return Ok(Some(format!(
            "active lock age {}s exceeds stale threshold {}s",
            age.as_secs(),
            stale_after.as_secs()
        )));
    }

    Ok(None)
}

fn stale_lock_after(config: &AppConfig) -> Duration {
    Duration::from_secs(
        config
            .workspace
            .max_run_minutes
            .saturating_mul(60)
            .saturating_add(STALE_LOCK_GRACE_SECS),
    )
}

fn active_lock_age(active_run: &ActiveRun) -> Option<Duration> {
    let created_at = chrono::DateTime::parse_from_rfc3339(&active_run.created_at)
        .ok()?
        .with_timezone(&chrono::Utc);
    chrono::Utc::now()
        .signed_duration_since(created_at)
        .to_std()
        .ok()
}

async fn create_active_lock(lock_path: &Path, active_run: &ActiveRun) -> std::io::Result<()> {
    let lock_path = lock_path.to_owned();
    let active_run = active_run.clone();
    task::spawn_blocking(move || create_active_lock_sync(&lock_path, &active_run))
        .await
        .map_err(Error::other)?
}

fn create_active_lock_sync(lock_path: &Path, active_run: &ActiveRun) -> std::io::Result<()> {
    let body = serde_json::to_vec_pretty(active_run)
        .map_err(|err| std::io::Error::new(ErrorKind::InvalidData, err))?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(lock_path)?;
    file.write_all(&body)?;
    file.write_all(b"\n")
}

async fn read_active_run(lock_path: &Path) -> anyhow::Result<Option<ActiveRun>> {
    let body = match tokio::fs::read(lock_path).await {
        Ok(body) => body,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(err).with_context(|| format!("failed to read {}", lock_path.display()));
        }
    };

    if body.is_empty() {
        return Ok(None);
    }

    serde_json::from_slice(&body)
        .map(Some)
        .with_context(|| format!("failed to parse {}", lock_path.display()))
}

async fn release_active_lock(lock_path: &Path, run_id: &str) -> anyhow::Result<()> {
    match read_active_run(lock_path).await {
        Ok(Some(active_run)) if active_run.run_id != run_id => {
            bail!(
                "active lock belongs to run `{}`, not `{run_id}`",
                active_run.run_id
            );
        }
        Ok(_) => {}
        Err(err) => {
            warn!(
                path = %lock_path.display(),
                error = %err,
                "active lock is unreadable; removing it"
            );
        }
    }

    match tokio::fs::remove_file(lock_path).await {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("failed to remove {}", lock_path.display())),
    }
}

async fn remove_active_lock_best_effort(lock_path: &Path) {
    if let Err(err) = tokio::fs::remove_file(lock_path).await
        && err.kind() != ErrorKind::NotFound
    {
        warn!(path = %lock_path.display(), error = %err, "failed to clean up active lock");
    }
}

async fn remove_run_dir_best_effort(run_dir: &Path) {
    if let Err(err) = tokio::fs::remove_dir_all(run_dir).await
        && err.kind() != ErrorKind::NotFound
    {
        warn!(path = %run_dir.display(), error = %err, "failed to clean up run directory");
    }
}

async fn clear_cancel_marker_best_effort(issue_dir: &Path, run_id: &str) {
    let marker_path = cancel_marker_path_for_issue_dir(issue_dir);
    if let Err(err) = tokio::fs::remove_file(&marker_path).await
        && err.kind() != ErrorKind::NotFound
    {
        warn!(
            run_id = %run_id,
            path = %marker_path.display(),
            error = %err,
            "failed to clear cancel marker"
        );
    }
}

async fn prepare_worktree(
    config: &AppConfig,
    token: &InstallationToken,
    repo: &RepoRef,
    repo_info: &RepositoryInfo,
    branch_name: &str,
) -> anyhow::Result<PathBuf> {
    let repo_dir = config.workspace.workdir.join(repo.safe_dir_name());
    tokio::fs::create_dir_all(&config.workspace.workdir)
        .await
        .with_context(|| format!("failed to create {}", config.workspace.workdir.display()))?;

    if !repo_dir.starts_with(&config.workspace.workdir) {
        bail!("computed repo workdir escaped configured workdir");
    }

    if tokio::fs::try_exists(repo_dir.join(".git"))
        .await
        .with_context(|| format!("failed to inspect {}", repo_dir.join(".git").display()))?
    {
        run_git(
            &repo_dir,
            token,
            vec![
                "fetch".to_owned(),
                "origin".to_owned(),
                repo_info.default_branch.clone(),
                "--prune".to_owned(),
            ],
        )
        .await?;
    } else {
        run_git_clone(&config.workspace.workdir, token, repo, &repo_dir).await?;
    }

    run_git(
        &repo_dir,
        token,
        vec![
            "switch".to_owned(),
            "-C".to_owned(),
            branch_name.to_owned(),
            format!("origin/{}", repo_info.default_branch),
        ],
    )
    .await?;

    Ok(repo_dir)
}

async fn run_git_clone(
    cwd: &Path,
    token: &InstallationToken,
    repo: &RepoRef,
    repo_dir: &Path,
) -> anyhow::Result<()> {
    let url = format!("https://github.com/{repo}.git");
    run_git_at(
        cwd,
        token,
        vec![
            "clone".to_owned(),
            url,
            repo_dir.to_string_lossy().into_owned(),
        ],
    )
    .await
}

async fn run_git(cwd: &Path, token: &InstallationToken, args: Vec<String>) -> anyhow::Result<()> {
    run_git_at(cwd, token, args).await
}

async fn run_git_at(
    cwd: &Path,
    token: &InstallationToken,
    args: Vec<String>,
) -> anyhow::Result<()> {
    let mut command = Command::new("git");
    let auth_header = git_auth_header(&token.token);
    command
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .arg("-c")
        .arg(format!(
            "http.https://github.com/.extraheader={auth_header}"
        ))
        .args(&args);

    let output = timeout(Duration::from_secs(300), command.output())
        .await
        .context("git command timed out")?
        .context("failed to execute git")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        warn!(?args, status = ?output.status, stderr = %stderr, "git command failed");
        bail!("git command failed with status {}", output.status);
    }

    Ok(())
}

fn git_auth_header(token: &str) -> String {
    let encoded = STANDARD.encode(format!("x-access-token:{token}"));
    format!("AUTHORIZATION: basic {encoded}")
}

fn branch_name_for_issue(issue: &IssueInfo) -> String {
    let change_type = change_type_for_issue(issue);
    let slug = slugify(&issue.title);
    format!("{change_type}/{slug}")
}

fn change_type_for_issue(issue: &IssueInfo) -> &'static str {
    let labels = issue
        .labels
        .iter()
        .map(|label| label.name.to_ascii_lowercase())
        .collect::<Vec<_>>();

    if labels.iter().any(|label| label == "bug" || label == "fix") {
        "fix"
    } else if labels
        .iter()
        .any(|label| label == "documentation" || label == "docs")
    {
        "docs"
    } else if labels
        .iter()
        .any(|label| label == "feature" || label == "enhancement" || label == "feat")
    {
        "feat"
    } else {
        "work"
    }
}

fn slugify(value: &str) -> String {
    let mut slug = String::new();
    let mut last_was_dash = false;

    for ch in value.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            last_was_dash = false;
        } else if !last_was_dash && !slug.is_empty() {
            slug.push('-');
            last_was_dash = true;
        }

        if slug.len() >= 48 {
            break;
        }
    }

    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "issue".to_owned()
    } else {
        slug.to_owned()
    }
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use std::{
        net::SocketAddr,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;
    use crate::{
        config::{GitHubConfig, ModelConfig, ServerConfig, WorkspaceConfig},
        github::IssueLabel,
    };

    #[test]
    fn branch_name_uses_type_and_slug() {
        let issue = IssueInfo {
            number: 12,
            title: "Add GitHub App webhook receiver!".to_owned(),
            body: None,
            html_url: "https://example.test".to_owned(),
            state: "open".to_owned(),
            labels: vec![IssueLabel {
                name: "feature".to_owned(),
            }],
            pull_request: None,
        };

        assert_eq!(
            branch_name_for_issue(&issue),
            "feat/add-github-app-webhook-receiver"
        );
    }

    #[test]
    fn slugify_falls_back_for_empty_title() {
        assert_eq!(slugify("?!"), "issue");
    }

    #[tokio::test]
    async fn claim_persists_event_and_blocks_duplicate() {
        let root = temp_dir("claim");
        let config = test_config(root.join("runs"));
        let trigger = test_trigger("delivery-1");
        let event = br#"{"action":"labeled"}"#;

        let decision = claim_issue_run(&config, trigger.clone(), Some(event))
            .await
            .unwrap();
        let RunClaimDecision::Claimed(claim) = decision else {
            panic!("expected claimed run");
        };

        let persisted_event = tokio::fs::read(claim.run_dir.join("event.json"))
            .await
            .unwrap();
        assert_eq!(persisted_event, event);
        assert!(claim.lock_path.exists());

        let duplicate = claim_issue_run(&config, trigger, Some(event))
            .await
            .unwrap();
        let RunClaimDecision::Duplicate { reason, active_run } = duplicate else {
            panic!("expected duplicate run");
        };
        assert!(reason.contains("already exists") || reason.contains("active run"));
        assert_eq!(active_run.unwrap().run_id, "delivery-1");

        release_active_lock(&claim.lock_path, claim.trigger.run_id.as_str())
            .await
            .unwrap();
        cleanup(root);
    }

    #[tokio::test]
    async fn claim_rejects_unsafe_run_id() {
        let root = temp_dir("unsafe-run-id");
        let config = test_config(root.join("runs"));
        let mut trigger = test_trigger("../evil");

        let err = claim_issue_run(&config, trigger.clone(), None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("run_id"));

        trigger.run_id = "nested/evil".to_owned();
        let err = claim_issue_run(&config, trigger, None).await.unwrap_err();
        assert!(err.to_string().contains("run_id"));

        cleanup(root);
    }

    #[tokio::test]
    async fn claim_recovers_unreadable_active_lock() {
        let root = temp_dir("corrupt-lock");
        let config = test_config(root.join("runs"));
        let trigger = test_trigger("delivery-new");
        let issue_dir = issue_dir(&config, &trigger.repo, trigger.issue_number);
        tokio::fs::create_dir_all(&issue_dir).await.unwrap();
        tokio::fs::write(active_lock_path_for_issue_dir(&issue_dir), b"not json")
            .await
            .unwrap();

        let decision = claim_issue_run(&config, trigger, None).await.unwrap();
        let RunClaimDecision::Claimed(claim) = decision else {
            panic!("expected corrupt lock recovery and new run claim");
        };
        let active_run = read_active_run(&claim.lock_path).await.unwrap().unwrap();
        assert_eq!(active_run.run_id, "delivery-new");

        release_active_lock(&claim.lock_path, claim.trigger.run_id.as_str())
            .await
            .unwrap();
        cleanup(root);
    }

    #[tokio::test]
    async fn claim_recovers_lock_for_terminal_run() {
        let root = temp_dir("stale-lock");
        let config = test_config(root.join("runs"));
        let repo = RepoRef::parse("cachebag/rafael").unwrap();
        let old_trigger = test_trigger("delivery-old");
        let new_trigger = test_trigger("delivery-new");

        let old_decision = claim_issue_run(&config, old_trigger, None).await.unwrap();
        let RunClaimDecision::Claimed(old_claim) = old_decision else {
            panic!("expected old run claim");
        };
        write_state(
            &old_claim.run_dir,
            &test_state("delivery-old", &repo, "2026-01-01T00:00:00+00:00"),
        )
        .await
        .unwrap();

        let new_decision = claim_issue_run(&config, new_trigger, None).await.unwrap();
        let RunClaimDecision::Claimed(new_claim) = new_decision else {
            panic!("expected stale lock recovery and new run claim");
        };
        assert_eq!(new_claim.trigger.run_id, "delivery-new");

        let active_run = read_active_run(&new_claim.lock_path)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(active_run.run_id, "delivery-new");

        release_active_lock(&new_claim.lock_path, new_claim.trigger.run_id.as_str())
            .await
            .unwrap();
        cleanup(root);
    }

    #[tokio::test]
    async fn status_returns_latest_run_state() {
        let root = temp_dir("status");
        let config = test_config(root.join("runs"));
        let repo = RepoRef::parse("cachebag/rafael").unwrap();
        let issue_dir = issue_dir(&config, &repo, 7);
        let older_dir = issue_dir.join("run-old");
        let newer_dir = issue_dir.join("run-new");
        tokio::fs::create_dir_all(&older_dir).await.unwrap();
        tokio::fs::create_dir_all(&newer_dir).await.unwrap();

        write_state(
            &older_dir,
            &test_state("run-old", &repo, "2026-01-01T00:00:00+00:00"),
        )
        .await
        .unwrap();
        write_state(
            &newer_dir,
            &test_state("run-new", &repo, "2026-01-02T00:00:00+00:00"),
        )
        .await
        .unwrap();

        let status = issue_status(&config, &repo, 7).await.unwrap();
        assert_eq!(status.latest_state.unwrap().run_id, "run-new");
        assert!(status.active_run.is_none());
        assert!(!status.cancel_requested);

        cleanup(root);
    }

    #[tokio::test]
    async fn clear_cancel_marker_removes_non_sticky_cancel() {
        let root = temp_dir("clear-cancel");
        let config = test_config(root.join("runs"));
        let repo = RepoRef::parse("cachebag/rafael").unwrap();
        let issue_dir = issue_dir(&config, &repo, 7);
        tokio::fs::create_dir_all(&issue_dir).await.unwrap();
        let marker_path = cancel_marker_path_for_issue_dir(&issue_dir);
        tokio::fs::write(&marker_path, b"{}").await.unwrap();

        clear_cancel_marker_best_effort(&issue_dir, "run-test").await;
        assert!(!tokio::fs::try_exists(&marker_path).await.unwrap());

        cleanup(root);
    }

    #[tokio::test]
    async fn cancel_writes_marker_for_active_run() {
        let root = temp_dir("cancel");
        let config = test_config(root.join("runs"));
        let trigger = test_trigger("delivery-cancel");
        let repo = trigger.repo.clone();
        let issue_number = trigger.issue_number;

        let decision = claim_issue_run(&config, trigger, None).await.unwrap();
        let RunClaimDecision::Claimed(claim) = decision else {
            panic!("expected claimed run");
        };

        let result = request_cancel(&config, &repo, issue_number).await.unwrap();
        assert_eq!(result.active_run.run_id, "delivery-cancel");
        assert!(result.marker_path.exists());

        let status = issue_status(&config, &repo, issue_number).await.unwrap();
        assert!(status.cancel_requested);

        release_active_lock(&claim.lock_path, claim.trigger.run_id.as_str())
            .await
            .unwrap();
        cleanup(root);
    }

    fn test_config(runs_dir: PathBuf) -> AppConfig {
        AppConfig {
            model: ModelConfig {
                base_url: "http://localhost:8080/v1".to_owned(),
                name: "test-model".to_owned(),
            },
            github: GitHubConfig {
                app_id: 1,
                installation_id: Some(2),
                private_key_path: PathBuf::from("/tmp/key.pem"),
                webhook_secret: Some("secret".to_owned()),
                app_slug: "netshared".to_owned(),
                collaborator_login: "netshared[bot]".to_owned(),
                allowed_repos: vec!["cachebag/rafael".to_owned()],
                implementation_label: "netshared:implement".to_owned(),
                command_mention: "@netshared".to_owned(),
                trusted_users: vec!["cachebag".to_owned()],
                blocking_labels: vec!["blocked".to_owned()],
                enable_assignment_trigger: false,
                api_base_url: "https://api.github.com".to_owned(),
            },
            workspace: WorkspaceConfig {
                workdir: runs_dir.join("../worktrees"),
                runs_dir,
                max_run_minutes: 45,
            },
            server: ServerConfig {
                bind: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
            },
        }
    }

    fn test_trigger(run_id: &str) -> IssueTrigger {
        IssueTrigger {
            repo: RepoRef::parse("cachebag/rafael").unwrap(),
            issue_number: 7,
            trigger: TriggerKind::Label,
            actor: Some("cachebag".to_owned()),
            installation_id: Some(42),
            run_id: run_id.to_owned(),
            default_branch: Some("master".to_owned()),
        }
    }

    fn test_state(run_id: &str, repo: &RepoRef, created_at: &str) -> RunState {
        RunState {
            run_id: run_id.to_owned(),
            repo: repo.clone(),
            issue_number: 7,
            trigger: TriggerKind::Label,
            actor: Some("cachebag".to_owned()),
            installation_id: Some(42),
            status: RunStatus::Completed,
            issue_title: Some("Test issue".to_owned()),
            issue_url: Some("https://example.test/issues/7".to_owned()),
            default_branch: Some("master".to_owned()),
            branch_name: Some("work/test-issue".to_owned()),
            worktree_path: None,
            plan_path: None,
            error: None,
            pr_url: None,
            commit_sha: None,
            verification_summary: None,
            created_at: created_at.to_owned(),
            updated_at: created_at.to_owned(),
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "rafael-coding-{name}-{}-{nanos}",
            std::process::id()
        ))
    }

    fn cleanup(path: PathBuf) {
        if let Err(err) = std::fs::remove_dir_all(&path)
            && err.kind() != ErrorKind::NotFound
        {
            panic!("failed to clean up {}: {err}", path.display());
        }
    }
}
