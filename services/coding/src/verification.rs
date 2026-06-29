use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use tokio::{
    fs::{File, OpenOptions},
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
    time::{Instant, timeout},
};

use crate::{config::AppConfig, repo_context::RepoContext};

pub struct VerificationRequest<'a> {
    pub config: &'a AppConfig,
    pub repo_context: &'a RepoContext,
    pub checkout_path: &'a Path,
    pub run_dir: &'a Path,
    pub commands: Vec<String>,
    pub start_index: usize,
}

pub async fn run_verification(
    request: VerificationRequest<'_>,
) -> anyhow::Result<VerificationOutcome> {
    let VerificationRequest {
        config,
        repo_context,
        checkout_path,
        run_dir,
        commands,
        start_index,
    } = request;
    let commands = if commands.is_empty() {
        select_verification_commands(config, repo_context)
    } else {
        dedupe_preserving_order(commands)
    };
    let verification_path = run_dir.join("verification");
    tokio::fs::create_dir_all(&verification_path)
        .await
        .with_context(|| format!("failed to create {}", verification_path.display()))?;

    if commands.is_empty() {
        return Ok(VerificationOutcome {
            status: VerificationStatus::Skipped,
            summary: "no verification commands selected".to_owned(),
            verification_path,
            results: Vec::new(),
            failed_commands: Vec::new(),
            next_index: start_index.max(1),
        });
    }

    let limits = VerificationLimits::from_config(config);
    let started_at = Instant::now();
    let mut results = Vec::new();
    let mut next_index = start_index.max(1);
    let mut total_timed_out = false;

    for command in commands {
        let elapsed = started_at.elapsed();
        if elapsed >= limits.total_timeout {
            total_timed_out = true;
            break;
        }

        let remaining = limits.total_timeout.saturating_sub(elapsed);
        let command_timeout = limits.command_timeout.min(remaining);
        let result = run_and_record_command(CommandRunRequest {
            index: next_index,
            command: &command,
            checkout_path,
            verification_path: &verification_path,
            timeout: command_timeout,
        })
        .await?;

        next_index += 1;
        results.push(result);
    }

    let status = verification_status(&results, total_timed_out);
    let failed_commands = results
        .iter()
        .filter(|result| !result.success)
        .map(|result| result.command.clone())
        .collect::<Vec<_>>();
    let summary = summarize_verification(status, &results, total_timed_out);

    Ok(VerificationOutcome {
        status,
        summary,
        verification_path,
        results,
        failed_commands,
        next_index,
    })
}

pub fn select_verification_commands(config: &AppConfig, repo_context: &RepoContext) -> Vec<String> {
    let mut commands = repo_context.verification_commands.clone();
    commands.extend(config.workspace.verify_commands.iter().cloned());

    if commands.is_empty() {
        commands.extend(fallback_commands(repo_context));
    }

    dedupe_preserving_order(commands)
}

async fn run_and_record_command(
    request: CommandRunRequest<'_>,
) -> anyhow::Result<VerificationCommandResult> {
    let CommandRunRequest {
        index,
        command,
        checkout_path,
        verification_path,
        timeout: command_timeout,
    } = request;
    let command_path = verification_path.join(format!("{index}-command.txt"));
    let stdout_path = verification_path.join(format!("{index}-stdout.log"));
    let stderr_path = verification_path.join(format!("{index}-stderr.log"));

    tokio::fs::write(&command_path, format!("{command}\n"))
        .await
        .with_context(|| format!("failed to write {}", command_path.display()))?;

    let parsed_command = match parse_command_line(command) {
        Ok(parsed_command) => parsed_command,
        Err(message) => {
            let stderr = format!("verification command rejected: {message}\n");
            write_command_logs(&stdout_path, &stderr_path, b"", stderr.as_bytes()).await?;
            return Ok(VerificationCommandResult::failed_before_spawn(
                index,
                command,
                command_path,
                stdout_path,
                stderr_path,
                message,
            ));
        }
    };

    let mut process = Command::new(&parsed_command.program);
    process
        .args(&parsed_command.args)
        .current_dir(checkout_path)
        .env("CI", "true")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("NO_COLOR", "1")
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    remove_sensitive_child_env(&mut process);

    let mut child = match process.spawn() {
        Ok(child) => child,
        Err(err) => {
            let stderr = format!("failed to execute verification command: {err}\n");
            write_command_logs(&stdout_path, &stderr_path, b"", stderr.as_bytes()).await?;
            return Ok(VerificationCommandResult::failed_before_spawn(
                index,
                command,
                command_path,
                stdout_path,
                stderr_path,
                stderr,
            ));
        }
    };

    let stdout = child
        .stdout
        .take()
        .context("spawned verification command stdout was not piped")?;
    let stderr = child
        .stderr
        .take()
        .context("spawned verification command stderr was not piped")?;
    let stdout_task = tokio::spawn(stream_output_to_file(stdout, stdout_path.clone()));
    let stderr_task = tokio::spawn(stream_output_to_file(stderr, stderr_path.clone()));

    let status = match timeout(command_timeout, child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(err)) => {
            let stderr = format!("failed to wait for verification command: {err}\n");
            write_command_logs(&stdout_path, &stderr_path, b"", stderr.as_bytes()).await?;
            return Ok(VerificationCommandResult::failed_before_spawn(
                index,
                command,
                command_path,
                stdout_path,
                stderr_path,
                stderr,
            ));
        }
        Err(_) => {
            let timeout_message = format!(
                "verification command timed out after {} seconds",
                command_timeout.as_secs()
            );
            if let Err(err) = child.kill().await {
                append_log_marker(
                    &stderr_path,
                    &format!("{timeout_message}; failed to kill process: {err}\n"),
                )
                .await?;
            }
            let stdout_log = finish_output_task(stdout_task).await?;
            let mut stderr_log = finish_output_task(stderr_task).await?;
            append_log_marker(&stderr_path, &format!("{timeout_message}\n")).await?;
            stderr_log.preview = Some(append_preview_marker(
                stderr_log.preview.as_deref(),
                &timeout_message,
            ));

            return Ok(VerificationCommandResult::timed_out(
                index,
                command,
                command_path,
                stdout_path,
                stderr_path,
                command_timeout.as_secs(),
                stdout_log,
                stderr_log,
            ));
        }
    };

    let stdout_log = finish_output_task(stdout_task).await?;
    let stderr_log = finish_output_task(stderr_task).await?;
    let success = status.success();
    Ok(VerificationCommandResult {
        index,
        command: command.to_owned(),
        command_path,
        stdout_path,
        stderr_path,
        success,
        exit_code: status.code(),
        timed_out: false,
        error: None,
        stdout_preview: stdout_log.preview,
        stderr_preview: stderr_log.preview,
        stdout_truncated: stdout_log.truncated,
        stderr_truncated: stderr_log.truncated,
    })
}

async fn write_command_logs(
    stdout_path: &Path,
    stderr_path: &Path,
    stdout: &[u8],
    stderr: &[u8],
) -> anyhow::Result<()> {
    tokio::fs::write(stdout_path, stdout)
        .await
        .with_context(|| format!("failed to write {}", stdout_path.display()))?;
    tokio::fs::write(stderr_path, stderr)
        .await
        .with_context(|| format!("failed to write {}", stderr_path.display()))?;
    Ok(())
}

fn remove_sensitive_child_env(process: &mut Command) {
    for key in std::env::vars()
        .map(|(key, _value)| key)
        .filter(|key| key.starts_with("RAFAEL_"))
    {
        process.env_remove(key);
    }
}

async fn stream_output_to_file<R>(mut reader: R, path: PathBuf) -> anyhow::Result<OutputLog>
where
    R: AsyncRead + Unpin,
{
    let mut file = File::create(&path)
        .await
        .with_context(|| format!("failed to create {}", path.display()))?;
    let mut buffer = [0_u8; OUTPUT_STREAM_BUFFER_BYTES];
    let mut tail = Vec::new();
    let mut bytes_read = 0_usize;
    let mut bytes_written = 0_usize;
    let mut truncated = false;

    loop {
        let read = reader
            .read(&mut buffer)
            .await
            .with_context(|| format!("failed to read command output for {}", path.display()))?;
        if read == 0 {
            break;
        }

        bytes_read = bytes_read.saturating_add(read);
        if bytes_written < MAX_OUTPUT_LOG_BYTES {
            let remaining = MAX_OUTPUT_LOG_BYTES - bytes_written;
            let write_len = read.min(remaining);
            file.write_all(&buffer[..write_len])
                .await
                .with_context(|| format!("failed to write {}", path.display()))?;
            append_tail(&mut tail, &buffer[..write_len]);
            bytes_written += write_len;
            truncated = truncated || write_len < read;
        } else {
            truncated = true;
        }
    }

    if truncated {
        let marker = format!(
            "\n...[truncated after {} bytes; read {} bytes]\n",
            MAX_OUTPUT_LOG_BYTES, bytes_read
        );
        file.write_all(marker.as_bytes())
            .await
            .with_context(|| format!("failed to write truncation marker to {}", path.display()))?;
        append_tail(&mut tail, marker.as_bytes());
    }
    file.flush()
        .await
        .with_context(|| format!("failed to flush {}", path.display()))?;

    Ok(OutputLog {
        preview: preview_output(&tail),
        truncated,
    })
}

async fn finish_output_task(
    task: tokio::task::JoinHandle<anyhow::Result<OutputLog>>,
) -> anyhow::Result<OutputLog> {
    task.await
        .context("verification output stream task failed")?
}

async fn append_log_marker(path: &Path, marker: &str) -> anyhow::Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
        .with_context(|| format!("failed to open {}", path.display()))?;
    file.write_all(marker.as_bytes())
        .await
        .with_context(|| format!("failed to append {}", path.display()))?;
    Ok(())
}

fn append_tail(tail: &mut Vec<u8>, bytes: &[u8]) {
    tail.extend_from_slice(bytes);
    if tail.len() > MAX_OUTPUT_PREVIEW_BYTES {
        let overflow = tail.len() - MAX_OUTPUT_PREVIEW_BYTES;
        tail.drain(..overflow);
    }
}

fn append_preview_marker(preview: Option<&str>, marker: &str) -> String {
    let combined = match preview {
        Some(preview) if !preview.is_empty() => format!("{preview}\n{marker}"),
        _ => marker.to_owned(),
    };
    truncate_text(&combined, MAX_OUTPUT_PREVIEW_CHARS)
}

fn verification_status(
    results: &[VerificationCommandResult],
    total_timed_out: bool,
) -> VerificationStatus {
    if results.is_empty() && total_timed_out {
        return VerificationStatus::TimedOut;
    }
    if results.iter().any(|result| result.timed_out) || total_timed_out {
        return VerificationStatus::TimedOut;
    }
    if results.iter().any(|result| !result.success) {
        return VerificationStatus::Failed;
    }

    VerificationStatus::Passed
}

fn summarize_verification(
    status: VerificationStatus,
    results: &[VerificationCommandResult],
    total_timed_out: bool,
) -> String {
    match status {
        VerificationStatus::Passed => format!(
            "verification passed: {} command(s) completed successfully",
            results.len()
        ),
        VerificationStatus::Skipped => "no verification commands selected".to_owned(),
        VerificationStatus::Failed | VerificationStatus::TimedOut => {
            let mut lines = Vec::new();
            if total_timed_out {
                lines.push("verification total timeout was reached".to_owned());
            } else {
                lines.push(format!(
                    "verification {}: {} command(s) failed",
                    status.as_str(),
                    results.iter().filter(|result| !result.success).count()
                ));
            }

            for result in results
                .iter()
                .filter(|result| !result.success)
                .take(MAX_FAILED_SUMMARIES)
            {
                lines.push(format!(
                    "command {} failed: `{}`",
                    result.index, result.command
                ));
                if let Some(exit_code) = result.exit_code {
                    lines.push(format!("exit code: {exit_code}"));
                }
                if result.timed_out {
                    lines.push("timed out: true".to_owned());
                }
                if let Some(error) = result.error.as_deref() {
                    lines.push(format!("error: {error}"));
                }
                if let Some(stderr) = result.stderr_preview.as_deref() {
                    lines.push(format!("stderr:\n{stderr}"));
                } else if let Some(stdout) = result.stdout_preview.as_deref() {
                    lines.push(format!("stdout:\n{stdout}"));
                }
            }

            lines.join("\n")
        }
    }
}

fn fallback_commands(repo_context: &RepoContext) -> Vec<String> {
    let mut commands = Vec::new();

    if has_project_kind(repo_context, "rust_workspace") {
        commands.push("cargo fmt --all -- --check".to_owned());
        commands.push("cargo test --workspace --all-features".to_owned());
    } else if has_project_kind(repo_context, "rust_package") {
        commands.push("cargo fmt --all -- --check".to_owned());
        commands.push("cargo test --all-features".to_owned());
    } else if has_project_kind(repo_context, "go") {
        commands.push("go test ./...".to_owned());
    } else if has_project_kind(repo_context, "node") {
        commands.push("bun test".to_owned());
    } else if has_project_kind(repo_context, "python") {
        commands.push("python -m pytest".to_owned());
    }

    commands
}

fn has_project_kind(repo_context: &RepoContext, kind: &str) -> bool {
    repo_context
        .project
        .detected
        .iter()
        .any(|detected| detected.kind == kind)
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

fn parse_command_line(command: &str) -> Result<ParsedCommand, String> {
    let command = command.trim();
    if command.is_empty() {
        return Err("command must be non-empty".to_owned());
    }
    if contains_shell_control(command) {
        return Err(
            "shell control operators and variable expansion are not supported for verification commands"
                .to_owned(),
        );
    }

    let parts = split_command_words(command)?;
    let Some((program, args)) = parts.split_first() else {
        return Err("command must include a program".to_owned());
    };

    Ok(ParsedCommand {
        program: program.clone(),
        args: args.to_vec(),
    })
}

fn contains_shell_control(command: &str) -> bool {
    command
        .chars()
        .any(|ch| matches!(ch, '\n' | '\r' | ';' | '|' | '<' | '>' | '`' | '$' | '&'))
}

fn split_command_words(command: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote = Quote::None;
    let mut escaped = false;

    for ch in command.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }

        match quote {
            Quote::None => match ch {
                '\\' => escaped = true,
                '\'' => quote = Quote::Single,
                '"' => quote = Quote::Double,
                ch if ch.is_whitespace() => push_word(&mut words, &mut current),
                _ => current.push(ch),
            },
            Quote::Single => match ch {
                '\'' => quote = Quote::None,
                _ => current.push(ch),
            },
            Quote::Double => match ch {
                '\\' => escaped = true,
                '"' => quote = Quote::None,
                _ => current.push(ch),
            },
        }
    }

    if escaped {
        return Err("command ends with an unfinished escape".to_owned());
    }
    if quote != Quote::None {
        return Err("command contains an unterminated quote".to_owned());
    }

    push_word(&mut words, &mut current);
    Ok(words)
}

fn push_word(words: &mut Vec<String>, current: &mut String) {
    if current.is_empty() {
        return;
    }

    words.push(std::mem::take(current));
}

fn preview_output(output: &[u8]) -> Option<String> {
    if output.is_empty() {
        return None;
    }

    let text = String::from_utf8_lossy(output);
    let lines = text
        .lines()
        .rev()
        .take(MAX_OUTPUT_PREVIEW_LINES)
        .collect::<Vec<_>>();
    let mut preview = lines.into_iter().rev().collect::<Vec<_>>().join("\n");
    if text.lines().count() > MAX_OUTPUT_PREVIEW_LINES {
        preview = format!("...[truncated]\n{preview}");
    }

    Some(truncate_text(&preview, MAX_OUTPUT_PREVIEW_CHARS))
}

fn truncate_text(value: &str, max_chars: usize) -> String {
    let mut truncated = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        truncated.push_str("\n...[truncated]");
    }
    truncated
}

#[derive(Debug)]
struct CommandRunRequest<'a> {
    index: usize,
    command: &'a str,
    checkout_path: &'a Path,
    verification_path: &'a Path,
    timeout: Duration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationOutcome {
    pub status: VerificationStatus,
    pub summary: String,
    pub verification_path: PathBuf,
    pub results: Vec<VerificationCommandResult>,
    pub failed_commands: Vec<String>,
    pub next_index: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VerificationStatus {
    Passed,
    Failed,
    TimedOut,
    Skipped,
}

impl VerificationStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::TimedOut => "timed out",
            Self::Skipped => "skipped",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationCommandResult {
    pub index: usize,
    pub command: String,
    pub command_path: PathBuf,
    pub stdout_path: PathBuf,
    pub stderr_path: PathBuf,
    pub success: bool,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub error: Option<String>,
    pub stdout_preview: Option<String>,
    pub stderr_preview: Option<String>,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

impl VerificationCommandResult {
    fn failed_before_spawn(
        index: usize,
        command: &str,
        command_path: PathBuf,
        stdout_path: PathBuf,
        stderr_path: PathBuf,
        error: String,
    ) -> Self {
        Self {
            index,
            command: command.to_owned(),
            command_path,
            stdout_path,
            stderr_path,
            success: false,
            exit_code: None,
            timed_out: false,
            error: Some(error),
            stdout_preview: None,
            stderr_preview: None,
            stdout_truncated: false,
            stderr_truncated: false,
        }
    }

    fn timed_out(
        index: usize,
        command: &str,
        command_path: PathBuf,
        stdout_path: PathBuf,
        stderr_path: PathBuf,
        timeout_seconds: u64,
        stdout_log: OutputLog,
        stderr_log: OutputLog,
    ) -> Self {
        Self {
            index,
            command: command.to_owned(),
            command_path,
            stdout_path,
            stderr_path,
            success: false,
            exit_code: None,
            timed_out: true,
            error: Some(format!("timed out after {timeout_seconds} seconds")),
            stdout_preview: stdout_log.preview,
            stderr_preview: stderr_log.preview.or_else(|| {
                Some(format!(
                    "verification command timed out after {timeout_seconds} seconds"
                ))
            }),
            stdout_truncated: stdout_log.truncated,
            stderr_truncated: stderr_log.truncated,
        }
    }
}

#[derive(Debug)]
struct OutputLog {
    preview: Option<String>,
    truncated: bool,
}

#[derive(Debug)]
struct ParsedCommand {
    program: String,
    args: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Quote {
    None,
    Single,
    Double,
}

#[derive(Debug, Clone, Copy)]
struct VerificationLimits {
    command_timeout: Duration,
    total_timeout: Duration,
}

impl VerificationLimits {
    fn from_config(config: &AppConfig) -> Self {
        Self {
            command_timeout: Duration::from_secs(
                config.workspace.verification_command_timeout_seconds,
            ),
            total_timeout: Duration::from_secs(config.workspace.verification_total_timeout_seconds),
        }
    }
}

const MAX_FAILED_SUMMARIES: usize = 3;
const MAX_OUTPUT_LOG_BYTES: usize = 1024 * 1024;
const MAX_OUTPUT_PREVIEW_BYTES: usize = 16 * 1024;
const MAX_OUTPUT_PREVIEW_LINES: usize = 40;
const MAX_OUTPUT_PREVIEW_CHARS: usize = 8_000;
const OUTPUT_STREAM_BUFFER_BYTES: usize = 8 * 1024;

#[cfg(test)]
mod tests {
    use std::{net::SocketAddr, path::PathBuf};

    use super::*;
    use crate::{
        config::{GitHubConfig, ModelConfig, ServerConfig, WorkspaceConfig},
        repo_context::{
            ProjectContext, ProjectDetection, RepoContextFiles, RepoContextIssue,
            RepoContextRepository,
        },
    };

    #[test]
    fn parses_simple_command_with_quotes() {
        let command = parse_command_line("cargo test --package 'coding service'").unwrap();

        assert_eq!(command.program, "cargo");
        assert_eq!(command.args, vec!["test", "--package", "coding service"]);
    }

    #[test]
    fn rejects_shell_control() {
        assert!(parse_command_line("cargo test && rm -rf target").is_err());
        assert!(parse_command_line("echo $SECRET").is_err());
    }

    #[test]
    fn selects_context_commands_before_fallbacks() {
        let config = test_config(Vec::new());
        let mut context = test_context(Vec::new());
        context.verification_commands = vec!["just verify".to_owned()];

        let commands = select_verification_commands(&config, &context);

        assert_eq!(commands, vec!["just verify".to_owned()]);
    }

    #[test]
    fn falls_back_for_go_project() {
        let config = test_config(Vec::new());
        let context = test_context(vec![ProjectDetection {
            kind: "go".to_owned(),
            evidence: vec!["go.mod".to_owned()],
        }]);

        let commands = select_verification_commands(&config, &context);

        assert_eq!(commands, vec!["go test ./...".to_owned()]);
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
                quiet_comments: false,
                api_base_url: "https://api.github.com".to_owned(),
                git_author_name: "netshared[bot]".to_owned(),
                git_author_email: "1+netshared[bot]@users.noreply.github.com".to_owned(),
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
                allow_unverified_publish: false,
            },
            server: ServerConfig {
                bind: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
            },
        }
    }

    fn test_context(detected: Vec<ProjectDetection>) -> RepoContext {
        RepoContext {
            generated_at: "2026-01-01T00:00:00Z".to_owned(),
            repository: RepoContextRepository {
                full_name: "cachebag/rafael".to_owned(),
                default_branch: "main".to_owned(),
                html_url: "https://example.test/cachebag/rafael".to_owned(),
            },
            issue: RepoContextIssue {
                number: 1,
                title: "Test".to_owned(),
                body: None,
                html_url: "https://example.test/issues/1".to_owned(),
                labels: Vec::new(),
                comments: Vec::new(),
            },
            files: RepoContextFiles {
                tracked_file_count: 0,
                tracked_files: Vec::new(),
                selected_files: Vec::new(),
            },
            project: ProjectContext { detected },
            verification_commands: Vec::new(),
        }
    }
}
