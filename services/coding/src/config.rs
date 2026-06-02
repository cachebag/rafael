use std::{env, net::SocketAddr, path::PathBuf};

use anyhow::{Context, bail};

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub model: ModelConfig,
    pub github: GitHubConfig,
    pub workspace: WorkspaceConfig,
    pub server: ServerConfig,
}

#[derive(Debug, Clone)]
pub struct ModelConfig {
    pub base_url: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct GitHubConfig {
    pub app_id: u64,
    pub installation_id: Option<u64>,
    pub private_key_path: PathBuf,
    pub webhook_secret: Option<String>,
    pub app_slug: String,
    pub collaborator_login: String,
    pub allowed_repos: Vec<String>,
    pub implementation_label: String,
    pub command_mention: String,
    pub trusted_users: Vec<String>,
    pub blocking_labels: Vec<String>,
    pub enable_assignment_trigger: bool,
    pub api_base_url: String,
}

#[derive(Debug, Clone)]
pub struct WorkspaceConfig {
    pub workdir: PathBuf,
    pub runs_dir: PathBuf,
    pub max_run_minutes: u64,
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub bind: SocketAddr,
}

impl AppConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        let app_slug = env_or("RAFAEL_GITHUB_APP_SLUG", "jonah");
        let workdir = PathBuf::from(env_or("RAFAEL_WORKDIR", "/var/lib/rafael/worktrees"));

        Ok(Self {
            model: ModelConfig {
                base_url: env_or("RAFAEL_MODEL_BASE_URL", "http://rafael:8080/v1"),
                name: env_or(
                    "RAFAEL_MODEL_NAME",
                    "Qwen/Qwen2.5-Coder-14B-Instruct-GGUF:Q4_K_M",
                ),
            },
            github: GitHubConfig {
                app_id: required_env("RAFAEL_GITHUB_APP_ID")?
                    .parse()
                    .context("RAFAEL_GITHUB_APP_ID must be an integer")?,
                installation_id: optional_env("RAFAEL_GITHUB_APP_INSTALLATION_ID")
                    .map(|value| {
                        value
                            .parse()
                            .context("RAFAEL_GITHUB_APP_INSTALLATION_ID must be an integer")
                    })
                    .transpose()?,
                private_key_path: PathBuf::from(required_env(
                    "RAFAEL_GITHUB_APP_PRIVATE_KEY_PATH",
                )?),
                webhook_secret: optional_env("RAFAEL_GITHUB_WEBHOOK_SECRET"),
                collaborator_login: env_or(
                    "RAFAEL_COLLABORATOR_LOGIN",
                    &format!("{app_slug}[bot]"),
                ),
                allowed_repos: split_csv(&required_env("RAFAEL_ALLOWED_REPOS")?),
                implementation_label: env_or("RAFAEL_IMPLEMENT_LABEL", "jonah:implement"),
                command_mention: env::var("RAFAEL_COMMAND_MENTION")
                    .unwrap_or_else(|_| format!("@{app_slug}")),
                trusted_users: split_csv(&env_or("RAFAEL_TRUSTED_USERS", "")),
                blocking_labels: split_csv(&env_or(
                    "RAFAEL_BLOCKING_LABELS",
                    "no-ai,needs-human,blocked",
                )),
                enable_assignment_trigger: parse_bool_env(
                    "RAFAEL_ENABLE_ASSIGNMENT_TRIGGER",
                    false,
                )?,
                api_base_url: env_or("RAFAEL_GITHUB_API_BASE_URL", "https://api.github.com"),
                app_slug,
            },
            workspace: WorkspaceConfig {
                runs_dir: PathBuf::from(
                    optional_env("RAFAEL_RUNS_DIR")
                        .unwrap_or_else(|| workdir.join("../runs").display().to_string()),
                ),
                max_run_minutes: env_or("RAFAEL_MAX_RUN_MINUTES", "45")
                    .parse()
                    .context("RAFAEL_MAX_RUN_MINUTES must be an integer")?,
                workdir,
            },
            server: ServerConfig {
                bind: env_or("RAFAEL_BIND", "127.0.0.1:3030")
                    .parse()
                    .context("RAFAEL_BIND must be a socket address like 127.0.0.1:3030")?,
            },
        })
    }
}

impl GitHubConfig {
    pub fn is_allowed_repo(&self, full_name: &str) -> bool {
        self.allowed_repos
            .iter()
            .any(|repo| repo.eq_ignore_ascii_case(full_name))
    }

    pub fn is_blocking_label(&self, label: &str) -> bool {
        self.blocking_labels
            .iter()
            .any(|blocked| blocked.eq_ignore_ascii_case(label))
    }

    pub fn is_trusted_user(&self, login: &str) -> bool {
        self.trusted_users
            .iter()
            .any(|trusted| trusted.eq_ignore_ascii_case(login))
    }
}

fn required_env(name: &str) -> anyhow::Result<String> {
    match env::var(name) {
        Ok(value) if !value.trim().is_empty() => Ok(value),
        _ => bail!("{name} is required"),
    }
}

fn optional_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn env_or(name: &str, default: &str) -> String {
    optional_env(name).unwrap_or_else(|| default.to_owned())
}

fn split_csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

fn parse_bool_env(name: &str, default: bool) -> anyhow::Result<bool> {
    let Some(value) = optional_env(name) else {
        return Ok(default);
    };

    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => bail!("{name} must be a boolean"),
    }
}
