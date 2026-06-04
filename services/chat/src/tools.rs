use std::time::Duration;

use anyhow::Context;
use client::ChatToolCall;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tool_registry::{ToolDefinition, ToolRegistry, object_schema};
use tracing::warn;
use web::{
    FetchOptions, FetchedPage, SearchOptions, SearchProvider, SearchResponse, SearchResult,
    WebFetcher, WebSearchClient,
};

use crate::types::{ChatMessageMetadata, ChatSource, ChatToolUse};

#[derive(Debug, Clone)]
pub struct ChatToolsConfig {
    pub search_provider: Option<SearchProvider>,
    pub search_timeout: Duration,
    pub fetch_timeout: Duration,
    pub max_search_results: usize,
    pub max_fetch_bytes: usize,
    pub max_search_fetches: usize,
    pub max_search_fetch_bytes: usize,
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

    pub async fn invoke(&self, call: &ChatToolCall) -> ToolInvocationResult {
        let mut metadata = ChatMessageMetadata {
            tool_uses: vec![ChatToolUse {
                name: call.function.name.clone(),
            }],
            sources: Vec::new(),
        };

        if self.registry.get(&call.function.name).is_none() {
            return ToolInvocationResult {
                content: tool_error(format!("unknown tool `{}`", call.function.name)),
                metadata,
            };
        }

        let result = match call.function.name.as_str() {
            "web_search" => self.invoke_web_search(&call.function.arguments).await,
            "fetch_url" => self.invoke_fetch_url(&call.function.arguments).await,
            _ => Err(anyhow::anyhow!("unknown tool `{}`", call.function.name)),
        };

        match result {
            Ok(invocation) => {
                metadata.merge(invocation.metadata);
                ToolInvocationResult {
                    content: tool_ok(invocation.value),
                    metadata,
                }
            }
            Err(err) => {
                warn!(tool = %call.function.name, error = %err, "tool invocation failed");
                ToolInvocationResult {
                    content: tool_error(err.to_string()),
                    metadata,
                }
            }
        }
    }

    async fn invoke_web_search(&self, arguments: &str) -> anyhow::Result<ToolValue> {
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
        let sources = response
            .results
            .iter()
            .map(search_result_source)
            .collect::<Vec<_>>();
        let enriched = self.enrich_search_response(response).await;

        Ok(ToolValue {
            value: serde_json::to_value(enriched)?,
            metadata: ChatMessageMetadata {
                tool_uses: Vec::new(),
                sources,
            },
        })
    }

    async fn invoke_fetch_url(&self, arguments: &str) -> anyhow::Result<ToolValue> {
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
        let source = ChatSource {
            title: None,
            url: page.url.clone(),
        };
        Ok(ToolValue {
            value: serde_json::to_value(page)?,
            metadata: ChatMessageMetadata {
                tool_uses: Vec::new(),
                sources: vec![source],
            },
        })
    }

    async fn enrich_search_response(&self, response: SearchResponse) -> EnrichedSearchResponse {
        let mut enriched_results = Vec::with_capacity(response.results.len());
        let mut fetches_remaining = self.config.max_search_fetches;

        for result in response.results {
            let fetched_page = if fetches_remaining > 0 {
                fetches_remaining -= 1;
                self.fetch_search_result(&result).await
            } else {
                None
            };
            enriched_results.push(EnrichedSearchResult::from_result(result, fetched_page));
        }

        EnrichedSearchResponse {
            query: response.query,
            provider: response.provider,
            results: enriched_results,
        }
    }

    async fn fetch_search_result(&self, result: &SearchResult) -> Option<FetchedPage> {
        match self
            .fetcher
            .fetch(
                &result.url,
                FetchOptions {
                    max_bytes: self.config.max_search_fetch_bytes,
                },
            )
            .await
        {
            Ok(page) if !page.text.trim().is_empty() => Some(page),
            Ok(_) => None,
            Err(err) => {
                warn!(url = %result.url, error = %err, "failed to enrich search result");
                None
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ToolInvocationResult {
    pub content: String,
    pub metadata: ChatMessageMetadata,
}

struct ToolValue {
    value: Value,
    metadata: ChatMessageMetadata,
}

#[derive(Debug, serde::Serialize)]
struct EnrichedSearchResponse {
    query: String,
    provider: String,
    results: Vec<EnrichedSearchResult>,
}

#[derive(Debug, serde::Serialize)]
struct EnrichedSearchResult {
    title: String,
    url: String,
    snippet: String,
    source: Option<String>,
    published_at: Option<String>,
    fetched_page: Option<EnrichedFetchedPage>,
}

impl EnrichedSearchResult {
    fn from_result(result: SearchResult, fetched_page: Option<FetchedPage>) -> Self {
        Self {
            title: result.title,
            url: result.url,
            snippet: result.snippet,
            source: result.source,
            published_at: result.published_at,
            fetched_page: fetched_page.map(EnrichedFetchedPage::from_page),
        }
    }
}

#[derive(Debug, serde::Serialize)]
struct EnrichedFetchedPage {
    url: String,
    text: String,
    bytes: usize,
    truncated: bool,
}

impl EnrichedFetchedPage {
    fn from_page(page: FetchedPage) -> Self {
        Self {
            url: page.url,
            text: page.text,
            bytes: page.bytes,
            truncated: page.truncated,
        }
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

fn search_result_source(result: &SearchResult) -> ChatSource {
    ChatSource {
        title: if result.title.trim().is_empty() {
            None
        } else {
            Some(result.title.clone())
        },
        url: result.url.clone(),
    }
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
