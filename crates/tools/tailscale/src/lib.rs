use std::{collections::BTreeMap, time::Duration};

use serde::{Deserialize, Serialize};
use shell::{CommandOutput, CommandRunner, CommandSpec, ShellError};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TailscaleError {
    #[error("tailscale command failed: {0}")]
    Shell(#[from] ShellError),
    #[error("failed to parse tailscale json output: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone)]
pub struct TailscaleClient {
    runner: CommandRunner,
    program: String,
}

impl TailscaleClient {
    pub fn new() -> Self {
        Self {
            runner: CommandRunner::default(),
            program: "tailscale".to_owned(),
        }
    }

    pub fn with_runner(runner: CommandRunner) -> Self {
        Self {
            runner,
            program: "tailscale".to_owned(),
        }
    }

    pub async fn raw<I, S>(&self, args: I) -> Result<CommandOutput, TailscaleError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Ok(self.runner.run(self.spec(args)?).await?)
    }

    pub async fn status(&self) -> Result<TailscaleStatus, TailscaleError> {
        let output = self
            .runner
            .run_required(self.spec(["status", "--json"])?)
            .await?;
        Ok(serde_json::from_str(&output.stdout)?)
    }

    pub async fn ip(&self) -> Result<Vec<String>, TailscaleError> {
        let output = self.runner.run_required(self.spec(["ip"])?).await?;
        Ok(output
            .stdout
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect())
    }

    pub async fn ping(
        &self,
        target: &str,
        timeout_seconds: u64,
    ) -> Result<CommandOutput, TailscaleError> {
        Ok(self
            .runner
            .run(self.spec([
                "ping".to_owned(),
                format!("--timeout={}s", timeout_seconds),
                target.to_owned(),
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
            .timeout(Duration::from_secs(60)))
    }
}

impl Default for TailscaleClient {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TailscaleStatus {
    #[serde(rename = "BackendState")]
    pub backend_state: Option<String>,
    #[serde(rename = "Self")]
    pub self_node: Option<TailscalePeer>,
    #[serde(rename = "Peer", default)]
    pub peers: BTreeMap<String, TailscalePeer>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TailscalePeer {
    #[serde(rename = "ID")]
    pub id: Option<String>,
    #[serde(rename = "HostName")]
    pub host_name: Option<String>,
    #[serde(rename = "DNSName")]
    pub dns_name: Option<String>,
    #[serde(rename = "OS")]
    pub os: Option<String>,
    #[serde(rename = "UserID")]
    pub user_id: Option<u64>,
    #[serde(rename = "TailscaleIPs", default)]
    pub tailscale_ips: Vec<String>,
    #[serde(rename = "Online")]
    pub online: Option<bool>,
    #[serde(rename = "ExitNode")]
    pub exit_node: Option<bool>,
    #[serde(rename = "ActiveTags", default)]
    pub active_tags: Vec<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_status_shape() {
        let status: TailscaleStatus = serde_json::from_str(
            r#"{"BackendState":"Running","Self":{"HostName":"rafael"},"Peer":{}}"#,
        )
        .unwrap();

        assert_eq!(status.backend_state.as_deref(), Some("Running"));
        assert_eq!(
            status.self_node.unwrap().host_name.as_deref(),
            Some("rafael")
        );
    }
}
