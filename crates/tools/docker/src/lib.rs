use std::time::Duration;

use serde::{Deserialize, Serialize};
use shell::{CommandOutput, CommandRunner, CommandSpec, ShellError};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DockerError {
    #[error("docker command failed: {0}")]
    Shell(#[from] ShellError),
    #[error("failed to parse docker json output: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone)]
pub struct DockerClient {
    runner: CommandRunner,
    program: String,
}

impl DockerClient {
    pub fn new() -> Self {
        Self {
            runner: CommandRunner::default(),
            program: "docker".to_owned(),
        }
    }

    pub fn with_runner(runner: CommandRunner) -> Self {
        Self {
            runner,
            program: "docker".to_owned(),
        }
    }

    pub async fn raw<I, S>(&self, args: I) -> Result<CommandOutput, DockerError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Ok(self.runner.run(self.spec(args)?).await?)
    }

    pub async fn version(&self) -> Result<serde_json::Value, DockerError> {
        let output = self
            .runner
            .run_required(self.spec(["version", "--format", "{{json .}}"])?)
            .await?;
        Ok(serde_json::from_str(output.stdout.trim())?)
    }

    pub async fn list_containers(
        &self,
        all: bool,
    ) -> Result<Vec<ContainerSummary>, DockerError> {
        let mut args = vec!["ps".to_owned(), "--format".to_owned(), "json".to_owned()];
        if all {
            args.push("--all".to_owned());
        }
        let output = self.runner.run_required(self.spec(args)?).await?;
        parse_json_lines(&output.stdout)
    }

    pub async fn inspect(&self, target: &str) -> Result<serde_json::Value, DockerError> {
        let output = self
            .runner
            .run_required(self.spec(["inspect", target])?)
            .await?;
        Ok(serde_json::from_str(&output.stdout)?)
    }

    pub async fn logs(&self, container: &str, tail: usize) -> Result<CommandOutput, DockerError> {
        Ok(self
            .runner
            .run(self.spec([
                "logs".to_owned(),
                "--tail".to_owned(),
                tail.to_string(),
                container.to_owned(),
            ])?)
            .await?)
    }

    fn spec<I, S>(&self, args: I) -> Result<CommandSpec, ShellError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Ok(CommandSpec::new(self.program.clone())?
            .args(args)
            .timeout(Duration::from_secs(120)))
    }
}

impl Default for DockerClient {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerSummary {
    #[serde(rename = "ID")]
    pub id: Option<String>,
    #[serde(rename = "Image")]
    pub image: Option<String>,
    #[serde(rename = "Command")]
    pub command: Option<String>,
    #[serde(rename = "CreatedAt")]
    pub created_at: Option<String>,
    #[serde(rename = "RunningFor")]
    pub running_for: Option<String>,
    #[serde(rename = "Ports")]
    pub ports: Option<String>,
    #[serde(rename = "State")]
    pub state: Option<String>,
    #[serde(rename = "Status")]
    pub status: Option<String>,
    #[serde(rename = "Names")]
    pub names: Option<String>,
    #[serde(rename = "Labels")]
    pub labels: Option<String>,
    #[serde(rename = "Mounts")]
    pub mounts: Option<String>,
    #[serde(rename = "Networks")]
    pub networks: Option<String>,
}

fn parse_json_lines<T>(stdout: &str) -> Result<Vec<T>, DockerError>
where
    T: for<'de> Deserialize<'de>,
{
    stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_str(line).map_err(DockerError::Json))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_container_json_lines() {
        let containers: Vec<ContainerSummary> =
            parse_json_lines(r#"{"ID":"abc","Image":"nginx","Names":"web"}"#).unwrap();

        assert_eq!(containers[0].id.as_deref(), Some("abc"));
        assert_eq!(containers[0].names.as_deref(), Some("web"));
    }
}
