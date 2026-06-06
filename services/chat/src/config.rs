use std::{net::SocketAddr, path::PathBuf, time::Duration};

use anyhow::Context;
use config::EnvConfig;
use web::SearchProvider;

use crate::tools::ChatToolsConfig;
use crate::types::{ProviderKind, StoredProvider};

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub bind: SocketAddr,
    pub data_dir: PathBuf,
    pub web_dist: PathBuf,
    pub default_provider: StoredProvider,
    pub model_timeout: Duration,
    pub model_context_max_chars: usize,
    pub model_list_timeout: Duration,
    pub auth_token_ttl: chrono::Duration,
    pub tools: ChatToolsConfig,
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
        let model_context_max_chars = reader
            .parse_or("RAFAEL_CHAT_MODEL_CONTEXT_MAX_CHARS", 60_000_usize)
            .context("failed to parse RAFAEL_CHAT_MODEL_CONTEXT_MAX_CHARS")?
            .clamp(4096, 262_144);
        let model_list_timeout_seconds = reader
            .parse_or("RAFAEL_CHAT_MODEL_LIST_TIMEOUT_SECONDS", 5_u64)
            .context("failed to parse RAFAEL_CHAT_MODEL_LIST_TIMEOUT_SECONDS")?;
        let auth_token_days = reader
            .parse_or("RAFAEL_CHAT_AUTH_TOKEN_DAYS", 30_i64)
            .context("failed to parse RAFAEL_CHAT_AUTH_TOKEN_DAYS")?
            .clamp(1, 365);
        let web_search_timeout_seconds = reader
            .parse_or("RAFAEL_CHAT_WEB_SEARCH_TIMEOUT_SECONDS", 15_u64)
            .context("failed to parse RAFAEL_CHAT_WEB_SEARCH_TIMEOUT_SECONDS")?;
        let web_fetch_timeout_seconds = reader
            .parse_or("RAFAEL_CHAT_WEB_FETCH_TIMEOUT_SECONDS", 15_u64)
            .context("failed to parse RAFAEL_CHAT_WEB_FETCH_TIMEOUT_SECONDS")?;
        let web_search_max_results = reader
            .parse_or("RAFAEL_CHAT_WEB_SEARCH_MAX_RESULTS", 5_usize)
            .context("failed to parse RAFAEL_CHAT_WEB_SEARCH_MAX_RESULTS")?
            .clamp(1, 8);
        let web_fetch_max_bytes = reader
            .parse_or("RAFAEL_CHAT_WEB_FETCH_MAX_BYTES", 64 * 1024_usize)
            .context("failed to parse RAFAEL_CHAT_WEB_FETCH_MAX_BYTES")?
            .clamp(1024, 128 * 1024);
        let web_search_fetch_results = reader
            .parse_or("RAFAEL_CHAT_WEB_SEARCH_FETCH_RESULTS", 3_usize)
            .context("failed to parse RAFAEL_CHAT_WEB_SEARCH_FETCH_RESULTS")?
            .clamp(0, 5);
        let web_search_fetch_max_bytes = reader
            .parse_or("RAFAEL_CHAT_WEB_SEARCH_FETCH_MAX_BYTES", 16 * 1024_usize)
            .context("failed to parse RAFAEL_CHAT_WEB_SEARCH_FETCH_MAX_BYTES")?
            .clamp(2048, 32 * 1024);
        let web_tool_max_invocations = reader
            .parse_or("RAFAEL_CHAT_WEB_TOOL_MAX_INVOCATIONS", 4_usize)
            .context("failed to parse RAFAEL_CHAT_WEB_TOOL_MAX_INVOCATIONS")?
            .clamp(1, 8);
        let search_provider = web_search_provider(&reader)?;

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
            model_context_max_chars,
            model_list_timeout: Duration::from_secs(model_list_timeout_seconds),
            auth_token_ttl: chrono::Duration::days(auth_token_days),
            tools: ChatToolsConfig {
                search_provider,
                search_timeout: Duration::from_secs(web_search_timeout_seconds),
                fetch_timeout: Duration::from_secs(web_fetch_timeout_seconds),
                max_search_results: web_search_max_results,
                max_fetch_bytes: web_fetch_max_bytes,
                max_search_fetches: web_search_fetch_results,
                max_search_fetch_bytes: web_search_fetch_max_bytes,
                max_invocations: web_tool_max_invocations,
            },
        })
    }
}

fn web_search_provider(reader: &config::EnvReader<'_>) -> anyhow::Result<Option<SearchProvider>> {
    let explicit = reader
        .optional_string("RAFAEL_CHAT_WEB_SEARCH_PROVIDER")
        .map(|value| value.to_ascii_lowercase());
    let searxng_base_url = reader.optional_string("RAFAEL_CHAT_SEARXNG_BASE_URL");
    let brave_api_key = reader.optional_string("RAFAEL_CHAT_BRAVE_SEARCH_API_KEY");

    match explicit.as_deref() {
        Some("disabled" | "none" | "off") => Ok(None),
        Some("searxng") => {
            let base_url = searxng_base_url.context(
                "RAFAEL_CHAT_SEARXNG_BASE_URL is required when web search provider is searxng",
            )?;
            Ok(Some(SearchProvider::Searxng { base_url }))
        }
        Some("brave") => {
            let api_key = brave_api_key.context(
                "RAFAEL_CHAT_BRAVE_SEARCH_API_KEY is required when web search provider is brave",
            )?;
            Ok(Some(SearchProvider::Brave { api_key }))
        }
        Some(provider) => anyhow::bail!(
            "RAFAEL_CHAT_WEB_SEARCH_PROVIDER must be disabled, searxng, or brave, got {provider}"
        ),
        None => {
            if let Some(base_url) = searxng_base_url {
                Ok(Some(SearchProvider::Searxng { base_url }))
            } else if let Some(api_key) = brave_api_key {
                Ok(Some(SearchProvider::Brave { api_key }))
            } else {
                Ok(None)
            }
        }
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
