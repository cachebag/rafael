use std::{net::SocketAddr, path::PathBuf, time::Duration};

use anyhow::Context;
use config::EnvConfig;

use crate::types::{ProviderKind, StoredProvider};

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub bind: SocketAddr,
    pub data_dir: PathBuf,
    pub web_dist: PathBuf,
    pub default_provider: StoredProvider,
    pub model_timeout: Duration,
    pub model_list_timeout: Duration,
}

impl AppConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        let env = EnvConfig::from_current_env();
        let reader = env.reader();
        let repo_root = repo_root()?;
        let service_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

        let model_timeout_seconds = reader
            .parse_or("RAFAEL_CHAT_MODEL_TIMEOUT_SECONDS", 300_u64)
            .context("failed to parse RAFAEL_CHAT_MODEL_TIMEOUT_SECONDS")?;
        let model_list_timeout_seconds = reader
            .parse_or("RAFAEL_CHAT_MODEL_LIST_TIMEOUT_SECONDS", 5_u64)
            .context("failed to parse RAFAEL_CHAT_MODEL_LIST_TIMEOUT_SECONDS")?;

        Ok(Self {
            bind: reader
                .parse_or("RAFAEL_CHAT_BIND", "127.0.0.1:3031".parse()?)
                .context("RAFAEL_CHAT_BIND must be a socket address like 127.0.0.1:3031")?,
            data_dir: reader.path_or("RAFAEL_CHAT_DATA_DIR", repo_root.join("data/chat")),
            web_dist: reader.path_or("RAFAEL_CHAT_WEB_DIST", service_root.join("web/dist")),
            default_provider: StoredProvider {
                id: "local-llama-swap".to_owned(),
                name: "Local llama-swap".to_owned(),
                kind: ProviderKind::OpenAiCompatible,
                base_url: reader.string_or("RAFAEL_CHAT_MODEL_BASE_URL", "http://rafael:8080/v1"),
                model: reader.string_or("RAFAEL_CHAT_MODEL_NAME", "gemma-everyday"),
                api_key: reader.optional_string("RAFAEL_CHAT_MODEL_API_KEY"),
                system_prompt: reader.optional_string("RAFAEL_CHAT_SYSTEM_PROMPT"),
            },
            model_timeout: Duration::from_secs(model_timeout_seconds),
            model_list_timeout: Duration::from_secs(model_list_timeout_seconds),
        })
    }
}

fn repo_root() -> anyhow::Result<PathBuf> {
    let service_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    service_root
        .parent()
        .and_then(|path| path.parent())
        .map(PathBuf::from)
        .context("failed to find repository root from CARGO_MANIFEST_DIR")
}
