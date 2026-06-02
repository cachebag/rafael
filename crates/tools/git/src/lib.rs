use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use shell::{CommandOutput, CommandRunner, CommandSpec, ShellError};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GitError {
    #[error("git command failed: {0}")]
    Shell(#[from] ShellError),
    #[error("unsafe branch name `{0}`")]
    UnsafeBranch(String),
}

#[derive(Debug, Clone)]
pub struct GitClient {
    runner: CommandRunner,
    program: String,
}

impl GitClient {
    pub fn new() -> Self {
        Self {
            runner: CommandRunner::default(),
            program: "git".to_owned(),
        }
    }

    pub fn with_runner(runner: CommandRunner) -> Self {
        Self {
            runner,
            program: "git".to_owned(),
        }
    }

    pub async fn run<I, S>(
        &self,
        repo: impl AsRef<Path>,
        args: I,
    ) -> Result<CommandOutput, GitError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Ok(self.runner.run(self.spec(Some(repo.as_ref()), args)?).await?)
    }

    pub async fn run_required<I, S>(
        &self,
        repo: impl AsRef<Path>,
        args: I,
    ) -> Result<CommandOutput, GitError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Ok(self
            .runner
            .run_required(self.spec(Some(repo.as_ref()), args)?)
            .await?)
    }

    pub async fn status_short(&self, repo: impl AsRef<Path>) -> Result<String, GitError> {
        Ok(self
            .run_required(repo, ["status", "--short"])
            .await?
            .stdout)
    }

    pub async fn changed_files(&self, repo: impl AsRef<Path>) -> Result<Vec<String>, GitError> {
        let output = self
            .run_required(repo, ["status", "--porcelain=v1"])
            .await?
            .stdout;
        Ok(output.lines().filter_map(parse_status_path).collect())
    }

    pub async fn diff_stat(&self, repo: impl AsRef<Path>) -> Result<String, GitError> {
        Ok(self.run_required(repo, ["diff", "--stat"]).await?.stdout)
    }

    pub async fn current_branch(&self, repo: impl AsRef<Path>) -> Result<String, GitError> {
        Ok(self
            .run_required(repo, ["rev-parse", "--abbrev-ref", "HEAD"])
            .await?
            .stdout
            .trim()
            .to_owned())
    }

    pub async fn rev_parse(
        &self,
        repo: impl AsRef<Path>,
        rev: &str,
    ) -> Result<String, GitError> {
        Ok(self
            .run_required(repo, ["rev-parse", rev])
            .await?
            .stdout
            .trim()
            .to_owned())
    }

    pub async fn switch_branch(
        &self,
        repo: impl AsRef<Path>,
        branch: &str,
        start_point: Option<&str>,
    ) -> Result<(), GitError> {
        if !is_safe_branch_name(branch) {
            return Err(GitError::UnsafeBranch(branch.to_owned()));
        }
        let mut args = vec!["switch".to_owned(), "-C".to_owned(), branch.to_owned()];
        if let Some(start_point) = start_point {
            args.push(start_point.to_owned());
        }
        self.run_required(repo, args).await?;
        Ok(())
    }

    pub async fn clone_repo(&self, options: CloneOptions) -> Result<(), GitError> {
        let mut args = Vec::new();
        if let Some(auth_header) = options.auth_header {
            args.push("-c".to_owned());
            args.push(format!(
                "http.{}.extraheader={auth_header}",
                options.auth_url_prefix.unwrap_or_else(|| "https://github.com/".to_owned())
            ));
        }
        args.push("clone".to_owned());
        args.push(options.remote_url);
        args.push(options.destination.to_string_lossy().into_owned());
        self.runner.run_required(self.spec(options.cwd.as_deref(), args)?).await?;
        Ok(())
    }

    fn spec<I, S>(&self, cwd: Option<&Path>, args: I) -> Result<CommandSpec, ShellError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut spec = CommandSpec::new(self.program.clone())?
            .args(args)
            .timeout(Duration::from_secs(300));
        if let Some(cwd) = cwd {
            spec = spec.cwd(cwd);
        }
        Ok(spec)
    }
}

impl Default for GitClient {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct CloneOptions {
    pub remote_url: String,
    pub destination: PathBuf,
    pub cwd: Option<PathBuf>,
    pub auth_header: Option<String>,
    pub auth_url_prefix: Option<String>,
}

impl CloneOptions {
    pub fn new(remote_url: impl Into<String>, destination: impl Into<PathBuf>) -> Self {
        Self {
            remote_url: remote_url.into(),
            destination: destination.into(),
            cwd: None,
            auth_header: None,
            auth_url_prefix: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchName(pub String);

impl BranchName {
    pub fn new(value: impl Into<String>) -> Result<Self, GitError> {
        let value = value.into();
        if is_safe_branch_name(&value) {
            Ok(Self(value))
        } else {
            Err(GitError::UnsafeBranch(value))
        }
    }
}

pub fn github_basic_auth_header(token: &str) -> String {
    let encoded = STANDARD.encode(format!("x-access-token:{token}"));
    format!("AUTHORIZATION: basic {encoded}")
}

pub fn is_safe_branch_name(branch: &str) -> bool {
    !branch.is_empty()
        && !branch.starts_with('-')
        && !branch.contains("..")
        && !branch.contains('\\')
        && !branch.contains(' ')
        && !branch.contains('\n')
        && !branch.contains('\r')
        && !branch.contains('\0')
        && !branch.ends_with('/')
        && !branch.ends_with(".lock")
}

pub fn slug_branch_component(value: &str, max_len: usize, fallback: &str) -> String {
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

        if slug.len() >= max_len {
            break;
        }
    }

    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        fallback.to_owned()
    } else {
        slug.to_owned()
    }
}

fn parse_status_path(line: &str) -> Option<String> {
    let path = line.get(3..)?.trim();
    if path.is_empty() {
        return None;
    }
    Some(path.rsplit(" -> ").next().unwrap_or(path).trim_matches('"').to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_name_validation_rejects_dangerous_refs() {
        assert!(is_safe_branch_name("feat/work").then_some(()).is_some());
        assert!(!is_safe_branch_name("-bad"));
        assert!(!is_safe_branch_name("bad..ref"));
        assert!(!is_safe_branch_name("bad.lock"));
    }

    #[test]
    fn status_path_handles_renames() {
        assert_eq!(
            parse_status_path("R  old.rs -> new.rs").as_deref(),
            Some("new.rs")
        );
    }

    #[test]
    fn slug_branch_component_falls_back() {
        assert_eq!(slug_branch_component("?!", 32, "work"), "work");
    }
}
