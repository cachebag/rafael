use std::{
    collections::{BTreeSet, HashSet},
    path::{Component, Path},
    process::Stdio,
    time::Duration,
};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use tokio::{process::Command, time::timeout};
use tracing::warn;

use crate::{
    config::AppConfig,
    github::{IssueComment, IssueInfo, RepositoryInfo},
};

pub async fn collect_repository_context(
    config: &AppConfig,
    repo: &RepositoryInfo,
    issue: &IssueInfo,
    comments: &[IssueComment],
    checkout_path: &Path,
) -> anyhow::Result<RepoContext> {
    let tracked_files = git_ls_files(checkout_path).await?;
    let selected_files =
        select_context_files(checkout_path, &tracked_files, issue, comments).await?;
    let project = detect_project(checkout_path, &tracked_files).await?;
    let verification_commands = verification_commands(config, &project);

    Ok(RepoContext {
        generated_at: chrono::Utc::now().to_rfc3339(),
        repository: RepoContextRepository {
            full_name: repo.full_name.clone(),
            default_branch: repo.default_branch.clone(),
            html_url: repo.html_url.clone(),
        },
        issue: RepoContextIssue {
            number: issue.number,
            title: issue.title.clone(),
            body: truncate_optional_text(issue.body.as_deref(), MAX_ISSUE_BODY_CHARS),
            html_url: issue.html_url.clone(),
            labels: issue
                .labels
                .iter()
                .map(|label| label.name.clone())
                .collect(),
            comments: comments
                .iter()
                .take(MAX_ISSUE_COMMENTS)
                .map(|comment| RepoContextIssueComment {
                    id: comment.id,
                    author: comment.user.login.clone(),
                    body: truncate_text(&comment.body, MAX_COMMENT_BODY_CHARS),
                    created_at: comment.created_at.clone(),
                })
                .collect(),
        },
        files: RepoContextFiles {
            tracked_file_count: tracked_files.len(),
            tracked_files: tracked_files
                .iter()
                .take(MAX_TRACKED_FILES)
                .cloned()
                .collect(),
            selected_files,
        },
        project,
        verification_commands,
    })
}

async fn git_ls_files(checkout_path: &Path) -> anyhow::Result<Vec<String>> {
    let output = timeout(
        Duration::from_secs(GIT_LS_FILES_TIMEOUT_SECS),
        Command::new("git")
            .current_dir(checkout_path)
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null())
            .arg("ls-files")
            .arg("-z")
            .output(),
    )
    .await
    .context("git ls-files timed out")?
    .context("failed to execute git ls-files")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "git ls-files failed with status {}: {stderr}",
            output.status
        );
    }

    let mut files = Vec::new();
    for entry in output.stdout.split(|byte| *byte == 0) {
        if entry.is_empty() {
            continue;
        }

        let path = String::from_utf8_lossy(entry).to_string();
        if is_safe_repo_relative_path(&path) {
            files.push(path);
        } else {
            warn!(%path, "ignored unsafe tracked path from git ls-files");
        }
    }

    Ok(files)
}

async fn select_context_files(
    checkout_path: &Path,
    tracked_files: &[String],
    issue: &IssueInfo,
    comments: &[IssueComment],
) -> anyhow::Result<Vec<FileContext>> {
    let mut selected = BTreeSet::new();

    for path in tracked_files {
        if is_root_manifest(path) || is_root_readme(path) {
            selected.insert(path.clone());
        }
    }

    for path in mentioned_paths(issue, comments, tracked_files) {
        selected.insert(path);
    }

    let mut files = Vec::new();
    for path in selected.into_iter().take(MAX_SELECTED_FILES) {
        files.push(summarize_file(checkout_path, &path).await?);
    }

    Ok(files)
}

fn mentioned_paths(
    issue: &IssueInfo,
    comments: &[IssueComment],
    tracked_files: &[String],
) -> BTreeSet<String> {
    let tracked = tracked_files
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let mut matches = BTreeSet::new();

    collect_mentioned_paths_from_text(&issue.title, &tracked, &mut matches);
    if let Some(body) = issue.body.as_deref() {
        collect_mentioned_paths_from_text(body, &tracked, &mut matches);
    }
    for comment in comments.iter().take(MAX_ISSUE_COMMENTS) {
        collect_mentioned_paths_from_text(&comment.body, &tracked, &mut matches);
    }

    matches
}

fn collect_mentioned_paths_from_text(
    text: &str,
    tracked: &HashSet<&str>,
    matches: &mut BTreeSet<String>,
) {
    for token in text.split_whitespace() {
        for candidate in path_candidate_variants(token) {
            if tracked.contains(candidate.as_str()) {
                matches.insert(candidate);
                break;
            }
        }
    }
}

fn path_candidate_variants(token: &str) -> Vec<String> {
    let token = token.trim_matches(|ch: char| {
        matches!(
            ch,
            '`' | '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>'
        )
    });

    if token.is_empty() || token.contains("://") {
        return Vec::new();
    }

    let mut variants = BTreeSet::new();
    add_path_candidate_variant(token, &mut variants);

    let trimmed = token.trim_end_matches([',', ';', ':', '!', '?', '.']);
    add_path_candidate_variant(trimmed, &mut variants);

    if let Some((path, line)) = trimmed.rsplit_once(':') {
        if !path.is_empty() && line.chars().all(|ch| ch.is_ascii_digit()) {
            add_path_candidate_variant(path, &mut variants);
        }
    }

    if let Some((path, _fragment)) = trimmed.split_once("#L") {
        add_path_candidate_variant(path, &mut variants);
    }

    variants.into_iter().collect()
}

fn add_path_candidate_variant(candidate: &str, variants: &mut BTreeSet<String>) {
    let candidate = candidate.strip_prefix("./").unwrap_or(candidate);
    if !candidate.is_empty() && is_safe_repo_relative_path(candidate) {
        variants.insert(candidate.to_owned());
    }
}

async fn summarize_file(checkout_path: &Path, relative_path: &str) -> anyhow::Result<FileContext> {
    if !is_safe_repo_relative_path(relative_path) {
        bail!("unsafe repository relative path `{relative_path}`");
    }

    let absolute_path = checkout_path.join(relative_path);
    if !absolute_path.starts_with(checkout_path) {
        bail!("context file path escaped checkout: {relative_path}");
    }

    let metadata = tokio::fs::metadata(&absolute_path)
        .await
        .with_context(|| format!("failed to stat {}", absolute_path.display()))?;
    let size_bytes = metadata.len();
    let kind = file_kind(relative_path);

    if is_sensitive_path(relative_path) {
        return Ok(FileContext {
            path: relative_path.to_owned(),
            kind,
            size_bytes,
            line_count: None,
            preview: None,
            preview_truncated: false,
            preview_skipped_reason: Some("sensitive path".to_owned()),
        });
    }

    if size_bytes > MAX_FILE_PREVIEW_BYTES as u64 {
        return Ok(FileContext {
            path: relative_path.to_owned(),
            kind,
            size_bytes,
            line_count: None,
            preview: None,
            preview_truncated: true,
            preview_skipped_reason: Some("file too large for preview".to_owned()),
        });
    }

    let bytes = tokio::fs::read(&absolute_path)
        .await
        .with_context(|| format!("failed to read {}", absolute_path.display()))?;
    if bytes.contains(&0) {
        return Ok(FileContext {
            path: relative_path.to_owned(),
            kind,
            size_bytes,
            line_count: None,
            preview: None,
            preview_truncated: false,
            preview_skipped_reason: Some("binary file".to_owned()),
        });
    }

    let content = String::from_utf8_lossy(&bytes);
    let line_count = content.lines().count();
    let preview = content
        .lines()
        .take(MAX_FILE_PREVIEW_LINES)
        .collect::<Vec<_>>()
        .join("\n");

    Ok(FileContext {
        path: relative_path.to_owned(),
        kind,
        size_bytes,
        line_count: Some(line_count),
        preview: Some(preview),
        preview_truncated: line_count > MAX_FILE_PREVIEW_LINES,
        preview_skipped_reason: None,
    })
}

async fn detect_project(
    checkout_path: &Path,
    tracked_files: &[String],
) -> anyhow::Result<ProjectContext> {
    let tracked = tracked_files
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let mut detected = Vec::new();

    if tracked.contains("Cargo.toml") {
        let cargo_toml = checkout_path.join("Cargo.toml");
        let body = tokio::fs::read_to_string(&cargo_toml)
            .await
            .with_context(|| format!("failed to read {}", cargo_toml.display()))?;
        let kind = if body.contains("[workspace]") {
            "rust_workspace"
        } else {
            "rust_package"
        };
        detected.push(ProjectDetection {
            kind: kind.to_owned(),
            evidence: vec!["Cargo.toml".to_owned()],
        });
    }

    if tracked.contains("package.json") {
        detected.push(ProjectDetection {
            kind: "node".to_owned(),
            evidence: vec!["package.json".to_owned()],
        });
    }

    if tracked.contains("go.mod") {
        detected.push(ProjectDetection {
            kind: "go".to_owned(),
            evidence: vec!["go.mod".to_owned()],
        });
    }

    if tracked.contains("pyproject.toml")
        || tracked.iter().any(|path| path.starts_with("requirements"))
    {
        let mut evidence = Vec::new();
        if tracked.contains("pyproject.toml") {
            evidence.push("pyproject.toml".to_owned());
        }
        evidence.extend(
            tracked
                .iter()
                .filter(|path| path.starts_with("requirements"))
                .map(|path| (*path).to_owned()),
        );
        detected.push(ProjectDetection {
            kind: "python".to_owned(),
            evidence,
        });
    }

    Ok(ProjectContext { detected })
}

fn verification_commands(config: &AppConfig, project: &ProjectContext) -> Vec<String> {
    let mut commands = Vec::new();

    if project.has_kind("rust_workspace") {
        commands.push("cargo fmt --all -- --check".to_owned());
        commands.push("cargo test --workspace --all-features".to_owned());
    } else if project.has_kind("rust_package") {
        commands.push("cargo fmt --all -- --check".to_owned());
        commands.push("cargo test --all-features".to_owned());
    }

    commands.extend(config.workspace.verify_commands.iter().cloned());
    dedupe_preserving_order(commands)
}

fn dedupe_preserving_order(commands: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();

    for command in commands {
        if seen.insert(command.clone()) {
            deduped.push(command);
        }
    }

    deduped
}

fn is_root_manifest(path: &str) -> bool {
    ROOT_MANIFESTS.contains(&path)
}

fn is_root_readme(path: &str) -> bool {
    let path = Path::new(path);
    if path.components().count() != 1 {
        return false;
    }

    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.to_ascii_lowercase().starts_with("readme"))
}

fn file_kind(path: &str) -> String {
    if is_root_manifest(path) {
        "manifest".to_owned()
    } else if is_root_readme(path) {
        "readme".to_owned()
    } else {
        "mentioned".to_owned()
    }
}

fn is_safe_repo_relative_path(path: &str) -> bool {
    let path = Path::new(path);
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

fn truncate_optional_text(value: Option<&str>, max_chars: usize) -> Option<String> {
    value.map(|value| truncate_text(value, max_chars))
}

fn truncate_text(value: &str, max_chars: usize) -> String {
    let mut truncated = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        truncated.push_str("\n...[truncated]");
    }
    truncated
}

fn is_sensitive_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower == ".env"
        || lower.starts_with(".env.")
        || lower.contains("/.env")
        || lower.ends_with(".pem")
        || lower.ends_with(".key")
        || lower.contains("secret")
        || lower.contains("token")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoContext {
    pub generated_at: String,
    pub repository: RepoContextRepository,
    pub issue: RepoContextIssue,
    pub files: RepoContextFiles,
    pub project: ProjectContext,
    pub verification_commands: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoContextRepository {
    pub full_name: String,
    pub default_branch: String,
    pub html_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoContextIssue {
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub html_url: String,
    pub labels: Vec<String>,
    pub comments: Vec<RepoContextIssueComment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoContextIssueComment {
    pub id: u64,
    pub author: String,
    pub body: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoContextFiles {
    pub tracked_file_count: usize,
    pub tracked_files: Vec<String>,
    pub selected_files: Vec<FileContext>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileContext {
    pub path: String,
    pub kind: String,
    pub size_bytes: u64,
    pub line_count: Option<usize>,
    pub preview: Option<String>,
    pub preview_truncated: bool,
    pub preview_skipped_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectContext {
    pub detected: Vec<ProjectDetection>,
}

impl ProjectContext {
    fn has_kind(&self, kind: &str) -> bool {
        self.detected.iter().any(|detected| detected.kind == kind)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectDetection {
    pub kind: String,
    pub evidence: Vec<String>,
}

const GIT_LS_FILES_TIMEOUT_SECS: u64 = 30;
const MAX_FILE_PREVIEW_BYTES: usize = 64 * 1024;
const MAX_FILE_PREVIEW_LINES: usize = 80;
const MAX_ISSUE_BODY_CHARS: usize = 8_000;
const MAX_COMMENT_BODY_CHARS: usize = 8_000;
const MAX_ISSUE_COMMENTS: usize = 30;
const MAX_SELECTED_FILES: usize = 40;
const MAX_TRACKED_FILES: usize = 1_000;
const ROOT_MANIFESTS: &[&str] = &[
    "Cargo.toml",
    "Cargo.lock",
    "package.json",
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "go.mod",
    "go.sum",
    "pyproject.toml",
    "requirements.txt",
    "uv.lock",
    "deno.json",
    "deno.jsonc",
    "flake.nix",
    "justfile",
    "Justfile",
    "Dockerfile",
    "docker-compose.yml",
    "compose.yml",
    "Makefile",
];

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::{
        config::{GitHubConfig, ModelConfig, ServerConfig, WorkspaceConfig},
        github::{IssueCommentUser, IssueLabel},
    };

    #[test]
    fn mentioned_paths_match_tracked_files() {
        let issue = test_issue(
            "Update services/coding/src/worker.rs",
            Some("Also check `Cargo.toml` and docs/2.md:12."),
        );
        let comments = vec![test_comment("Maybe `./services/coding/src/model.rs` too")];
        let tracked_files = vec![
            "Cargo.toml".to_owned(),
            "docs/2.md".to_owned(),
            "services/coding/src/model.rs".to_owned(),
            "services/coding/src/worker.rs".to_owned(),
        ];

        let paths = mentioned_paths(&issue, &comments, &tracked_files);

        assert!(paths.contains("Cargo.toml"));
        assert!(paths.contains("docs/2.md"));
        assert!(paths.contains("services/coding/src/model.rs"));
        assert!(paths.contains("services/coding/src/worker.rs"));
    }

    #[test]
    fn rust_workspace_commands_are_detected() {
        let config = test_config(vec!["just check".to_owned()]);
        let project = ProjectContext {
            detected: vec![ProjectDetection {
                kind: "rust_workspace".to_owned(),
                evidence: vec!["Cargo.toml".to_owned()],
            }],
        };

        let commands = verification_commands(&config, &project);

        assert_eq!(
            commands,
            vec![
                "cargo fmt --all -- --check".to_owned(),
                "cargo test --workspace --all-features".to_owned(),
                "just check".to_owned(),
            ]
        );
    }

    #[test]
    fn unknown_project_uses_configured_commands_only() {
        let config = test_config(vec!["just verify".to_owned()]);
        let project = ProjectContext { detected: vec![] };

        let commands = verification_commands(&config, &project);

        assert_eq!(commands, vec!["just verify".to_owned()]);
    }

    #[test]
    fn long_context_text_is_truncated() {
        let text = "a".repeat(10);

        assert_eq!(truncate_text(&text, 5), "aaaaa\n...[truncated]");
    }

    #[test]
    fn unsafe_paths_are_rejected() {
        assert!(!is_safe_repo_relative_path("../secret"));
        assert!(!is_safe_repo_relative_path("/tmp/secret"));
        assert!(is_safe_repo_relative_path("services/coding/src/main.rs"));
    }

    fn test_issue(title: &str, body: Option<&str>) -> IssueInfo {
        IssueInfo {
            number: 1,
            title: title.to_owned(),
            body: body.map(str::to_owned),
            html_url: "https://example.test/issues/1".to_owned(),
            state: "open".to_owned(),
            labels: vec![IssueLabel {
                name: "feature".to_owned(),
            }],
            pull_request: None,
        }
    }

    fn test_comment(body: &str) -> IssueComment {
        IssueComment {
            id: 1,
            body: body.to_owned(),
            created_at: "2026-01-01T00:00:00Z".to_owned(),
            user: IssueCommentUser {
                login: "cachebag".to_owned(),
            },
        }
    }

    fn test_config(verify_commands: Vec<String>) -> AppConfig {
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
                workdir: PathBuf::from("/tmp/work"),
                runs_dir: PathBuf::from("/tmp/runs"),
                max_run_minutes: 45,
                verify_commands,
                max_tool_iterations: 24,
                max_tool_runtime_seconds: 900,
                max_file_read_bytes: 131_072,
                max_write_bytes: 262_144,
                max_changed_files: 12,
                verification_command_timeout_seconds: 600,
                verification_total_timeout_seconds: 1_200,
            },
            server: ServerConfig {
                bind: "127.0.0.1:0".parse().unwrap(),
            },
        }
    }
}
