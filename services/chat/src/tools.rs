use std::time::Duration;

use anyhow::Context;
use client::ChatToolCall;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tool_registry::{ToolDefinition, ToolRegistry, object_schema};
use tracing::warn;
use web::{FetchOptions, SearchOptions, SearchProvider, WebFetcher, WebSearchClient};

#[derive(Debug, Clone)]
pub struct ChatToolsConfig {
    pub search_provider: Option<SearchProvider>,
    pub search_timeout: Duration,
    pub fetch_timeout: Duration,
    pub max_search_results: usize,
    pub max_fetch_bytes: usize,
    pub max_invocations: usize,
}

impl ChatToolsConfig {
    pub fn enabled(&self) -> bool {
        self.search_provider.is_some()
    }
}

#[derive(Clone)]
pub struct ChatToolRuntime {
    config: ChatToolsConfig,
    search: WebSearchClient,
    fetcher: WebFetcher,
    registry: ToolRegistry,
}

impl ChatToolRuntime {
    pub fn new(config: ChatToolsConfig) -> anyhow::Result<Self> {
        let registry =
            ToolRegistry::with_tools([web_search_definition()?, fetch_url_definition()?])
                .context("failed to initialize chat tool registry")?;
        Ok(Self {
            search: WebSearchClient::new(config.search_provider.clone(), config.search_timeout)
                .context("failed to initialize web search client")?,
            fetcher: WebFetcher::new(config.fetch_timeout)
                .context("failed to initialize web fetcher")?,
            config,
            registry,
        })
    }

    pub fn enabled(&self) -> bool {
        self.config.enabled()
    }

    pub fn max_invocations(&self) -> usize {
        self.config.max_invocations
    }

    pub fn openai_tools(&self) -> anyhow::Result<Vec<Value>> {
        self.registry
            .openai_tools()
            .into_iter()
            .map(|tool| serde_json::to_value(tool).context("failed to serialize tool schema"))
            .collect()
    }

    pub async fn invoke(&self, call: &ChatToolCall) -> String {
        if self.registry.get(&call.function.name).is_none() {
            return tool_error(format!("unknown tool `{}`", call.function.name));
        }

        let result = match call.function.name.as_str() {
            "web_search" => self.invoke_web_search(&call.function.arguments).await,
            "fetch_url" => self.invoke_fetch_url(&call.function.arguments).await,
            _ => Err(anyhow::anyhow!("unknown tool `{}`", call.function.name)),
        };

        match result {
            Ok(value) => tool_ok(value),
            Err(err) => {
                warn!(tool = %call.function.name, error = %err, "tool invocation failed");
                tool_error(err.to_string())
            }
        }
    }

    async fn invoke_web_search(&self, arguments: &str) -> anyhow::Result<Value> {
        let args: WebSearchArgs =
            serde_json::from_str(arguments).context("web_search arguments must be json")?;
        let max_results = args
            .max_results
            .unwrap_or(self.config.max_search_results)
            .clamp(1, self.config.max_search_results);
        let response = self
            .search
            .search(&args.query, SearchOptions { max_results })
            .await
            .context("web_search failed")?;
        Ok(serde_json::to_value(response)?)
    }

    async fn invoke_fetch_url(&self, arguments: &str) -> anyhow::Result<Value> {
        let args: FetchUrlArgs =
            serde_json::from_str(arguments).context("fetch_url arguments must be json")?;
        let max_bytes = args
            .max_bytes
            .unwrap_or(self.config.max_fetch_bytes)
            .clamp(1024, self.config.max_fetch_bytes);
        let page = self
            .fetcher
            .fetch(&args.url, FetchOptions { max_bytes })
            .await
            .context("fetch_url failed")?;
        Ok(serde_json::to_value(page)?)
    }
}

#[derive(Debug, Deserialize)]
struct WebSearchArgs {
    query: String,
    max_results: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct FetchUrlArgs {
    url: String,
    max_bytes: Option<usize>,
}

fn web_search_definition() -> Result<ToolDefinition, tool_registry::RegistryError> {
    ToolDefinition::new(
        "web_search",
        "Search the public web for current information. Returns a bounded list of result titles, URLs, snippets, sources, and dates when available.",
        object_schema(
            Map::from_iter([
                (
                    "query".to_owned(),
                    json!({
                        "type": "string",
                        "description": "Search query. Include domain filters or dates when useful."
                    }),
                ),
                (
                    "max_results".to_owned(),
                    json!({
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 8,
                        "description": "Maximum number of search results to return."
                    }),
                ),
            ]),
            vec!["query".to_owned()],
        ),
    )
}

fn fetch_url_definition() -> Result<ToolDefinition, tool_registry::RegistryError> {
    ToolDefinition::new(
        "fetch_url",
        "Fetch one public http(s) URL and return extracted readable text. Use this after web_search when a result needs source details.",
        object_schema(
            Map::from_iter([
                (
                    "url".to_owned(),
                    json!({
                        "type": "string",
                        "description": "A public http or https URL. Local/private-network URLs are blocked."
                    }),
                ),
                (
                    "max_bytes".to_owned(),
                    json!({
                        "type": "integer",
                        "minimum": 1024,
                        "maximum": 65536,
                        "description": "Maximum bytes to fetch before text extraction."
                    }),
                ),
            ]),
            vec!["url".to_owned()],
        ),
    )
}

fn tool_ok(value: Value) -> String {
    json!({
        "ok": true,
        "result": value
    })
    .to_string()
}

fn tool_error(error: impl Into<String>) -> String {
    json!({
        "ok": false,
        "error": error.into()
    })
    .to_string()
}
