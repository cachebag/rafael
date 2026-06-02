use std::{collections::BTreeMap, path::PathBuf, process::Stdio, time::Duration};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    time::timeout,
};

#[derive(Debug, Error)]
pub enum ShellError {
    #[error("command program must be non-empty")]
    EmptyProgram,
    #[error("command program must not contain NUL bytes")]
    ProgramContainsNul,
    #[error("failed to parse command line: {0}")]
    Parse(String),
    #[error("failed to spawn `{program}`: {source}")]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to wait for `{program}`: {source}")]
    Wait {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("output task for `{program}` failed: {source}")]
    OutputTask {
        program: String,
        #[source]
        source: tokio::task::JoinError,
    },
    #[error("failed to read output for `{program}`: {source}")]
    OutputRead {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("command `{program}` did not complete successfully")]
    Failed { program: String },
}

#[derive(Debug, Clone)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: BTreeMap<String, String>,
    pub timeout: Option<Duration>,
    pub clear_env: bool,
    pub remove_sensitive_env: bool,
    pub non_interactive: bool,
}

impl CommandSpec {
    pub fn new(program: impl Into<String>) -> Result<Self, ShellError> {
        let program = program.into();
        validate_program(&program)?;
        Ok(Self {
            program,
            args: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
            timeout: None,
            clear_env: false,
            remove_sensitive_env: true,
            non_interactive: true,
        })
    }

    pub fn parse(command: &str) -> Result<Self, ShellError> {
        let parsed = parse_command_line(command)?;
        Ok(Self::new(parsed.program)?.args(parsed.args))
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn clear_env(mut self, clear_env: bool) -> Self {
        self.clear_env = clear_env;
        self
    }

    pub fn remove_sensitive_env(mut self, remove_sensitive_env: bool) -> Self {
        self.remove_sensitive_env = remove_sensitive_env;
        self
    }

    pub fn non_interactive(mut self, non_interactive: bool) -> Self {
        self.non_interactive = non_interactive;
        self
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CommandRunner {
    pub default_timeout: Duration,
    pub output_limit: usize,
}

impl CommandRunner {
    pub fn new(default_timeout: Duration, output_limit: usize) -> Self {
        Self {
            default_timeout,
            output_limit,
        }
    }

    pub async fn run(&self, spec: CommandSpec) -> Result<CommandOutput, ShellError> {
        let mut command = Command::new(&spec.program);
        command
            .args(&spec.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(cwd) = &spec.cwd {
            command.current_dir(cwd);
        }
        if spec.clear_env {
            command.env_clear();
        } else if spec.remove_sensitive_env {
            remove_sensitive_child_env(&mut command);
        }
        if spec.non_interactive {
            command
                .env("CI", "true")
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("NO_COLOR", "1");
        }
        command.envs(&spec.env);

        let mut child = command.spawn().map_err(|source| ShellError::Spawn {
            program: spec.program.clone(),
            source,
        })?;
        let stdout = child.stdout.take().ok_or_else(|| ShellError::Spawn {
            program: spec.program.clone(),
            source: std::io::Error::other("stdout was not piped"),
        })?;
        let stderr = child.stderr.take().ok_or_else(|| ShellError::Spawn {
            program: spec.program.clone(),
            source: std::io::Error::other("stderr was not piped"),
        })?;
        let stdout_task = tokio::spawn(read_capped(stdout, self.output_limit));
        let stderr_task = tokio::spawn(read_capped(stderr, self.output_limit));
        let timeout_duration = spec.timeout.unwrap_or(self.default_timeout);

        let (status_code, success, timed_out) = match timeout(timeout_duration, child.wait()).await
        {
            Ok(Ok(status)) => (status.code(), status.success(), false),
            Ok(Err(source)) => {
                return Err(ShellError::Wait {
                    program: spec.program,
                    source,
                });
            }
            Err(_) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                (None, false, true)
            }
        };

        let stdout = stdout_task
            .await
            .map_err(|source| ShellError::OutputTask {
                program: spec.program.clone(),
                source,
            })?
            .map_err(|source| ShellError::OutputRead {
                program: spec.program.clone(),
                source,
            })?;
        let stderr = stderr_task
            .await
            .map_err(|source| ShellError::OutputTask {
                program: spec.program.clone(),
                source,
            })?
            .map_err(|source| ShellError::OutputRead {
                program: spec.program.clone(),
                source,
            })?;

        Ok(CommandOutput {
            program: spec.program,
            args: spec.args,
            status_code,
            success,
            timed_out,
            stdout: stdout.text,
            stderr: stderr.text,
            stdout_bytes: stdout.bytes_read,
            stderr_bytes: stderr.bytes_read,
            stdout_truncated: stdout.truncated,
            stderr_truncated: stderr.truncated,
        })
    }

    pub async fn run_required(&self, spec: CommandSpec) -> Result<CommandOutput, ShellError> {
        let output = self.run(spec).await?;
        if output.success {
            Ok(output)
        } else {
            Err(ShellError::Failed {
                program: output.program,
            })
        }
    }
}

impl Default for CommandRunner {
    fn default() -> Self {
        Self {
            default_timeout: Duration::from_secs(300),
            output_limit: 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandOutput {
    pub program: String,
    pub args: Vec<String>,
    pub status_code: Option<i32>,
    pub success: bool,
    pub timed_out: bool,
    pub stdout: String,
    pub stderr: String,
    pub stdout_bytes: usize,
    pub stderr_bytes: usize,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCommand {
    pub program: String,
    pub args: Vec<String>,
}

pub fn parse_command_line(command: &str) -> Result<ParsedCommand, ShellError> {
    let command = command.trim();
    if command.is_empty() {
        return Err(ShellError::Parse("command must be non-empty".to_owned()));
    }
    if contains_shell_control(command) {
        return Err(ShellError::Parse(
            "shell control operators and variable expansion are not supported".to_owned(),
        ));
    }

    let parts = split_command_words(command)?;
    let Some((program, args)) = parts.split_first() else {
        return Err(ShellError::Parse(
            "command must include a program".to_owned(),
        ));
    };
    validate_program(program)?;

    Ok(ParsedCommand {
        program: program.clone(),
        args: args.to_vec(),
    })
}

fn validate_program(program: &str) -> Result<(), ShellError> {
    if program.is_empty() {
        return Err(ShellError::EmptyProgram);
    }
    if program.contains('\0') {
        return Err(ShellError::ProgramContainsNul);
    }
    Ok(())
}

fn remove_sensitive_child_env(command: &mut Command) {
    for key in std::env::vars()
        .map(|(key, _value)| key)
        .filter(|key| looks_sensitive_env_name(key))
    {
        command.env_remove(key);
    }
}

fn looks_sensitive_env_name(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    upper.contains("TOKEN")
        || upper.contains("SECRET")
        || upper.contains("PASSWORD")
        || upper.contains("PRIVATE_KEY")
        || upper.contains("CREDENTIAL")
        || upper.ends_with("_KEY")
}

fn contains_shell_control(command: &str) -> bool {
    command
        .chars()
        .any(|ch| matches!(ch, '\n' | '\r' | ';' | '|' | '<' | '>' | '`' | '$' | '&'))
}

fn split_command_words(command: &str) -> Result<Vec<String>, ShellError> {
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
        return Err(ShellError::Parse(
            "command ends with an unfinished escape".to_owned(),
        ));
    }
    if quote != Quote::None {
        return Err(ShellError::Parse(
            "command contains an unterminated quote".to_owned(),
        ));
    }

    push_word(&mut words, &mut current);
    Ok(words)
}

fn push_word(words: &mut Vec<String>, current: &mut String) {
    if !current.is_empty() {
        words.push(std::mem::take(current));
    }
}

async fn read_capped<R>(mut reader: R, limit: usize) -> Result<CapturedStream, std::io::Error>
where
    R: AsyncRead + Unpin,
{
    let mut buffer = [0_u8; 8192];
    let mut kept = Vec::new();
    let mut bytes_read = 0_usize;
    let mut truncated = false;

    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        bytes_read = bytes_read.saturating_add(read);
        if kept.len() < limit {
            let remaining = limit - kept.len();
            let keep = read.min(remaining);
            kept.extend_from_slice(&buffer[..keep]);
            truncated = truncated || keep < read;
        } else {
            truncated = true;
        }
    }

    Ok(CapturedStream {
        text: String::from_utf8_lossy(&kept).into_owned(),
        bytes_read,
        truncated,
    })
}

#[derive(Debug)]
struct CapturedStream {
    text: String,
    bytes_read: usize,
    truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Quote {
    None,
    Single,
    Double,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quoted_command_line() {
        let parsed = parse_command_line("cargo test 'one two' \"three\"").unwrap();

        assert_eq!(parsed.program, "cargo");
        assert_eq!(parsed.args, ["test", "one two", "three"]);
    }

    #[test]
    fn rejects_shell_control() {
        assert!(parse_command_line("cargo test && rm -rf target").is_err());
        assert!(parse_command_line("echo $TOKEN").is_err());
    }
}
