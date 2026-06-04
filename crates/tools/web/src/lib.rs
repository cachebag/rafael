use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use reqwest::{Client, redirect::Policy};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::net::lookup_host;
use url::Url;

#[derive(Debug, Error)]
pub enum WebError {
    #[error("web search provider is not configured")]
    MissingSearchProvider,
    #[error("query must be non-empty")]
    EmptyQuery,
    #[error("URL must use http or https: {0}")]
    UnsupportedUrlScheme(String),
    #[error("URL must include a host: {0}")]
    MissingHost(String),
    #[error("blocked local or private-network host: {0}")]
    BlockedHost(String),
    #[error("failed to resolve host `{host}`: {source}")]
    Dns {
        host: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed HTTP request: {0}")]
    Http(#[from] reqwest::Error),
    #[error("failed to parse URL: {0}")]
    Url(#[from] url::ParseError),
}

#[derive(Debug, Clone)]
pub struct WebSearchClient {
    http: Client,
    provider: Option<SearchProvider>,
}

impl WebSearchClient {
    pub fn new(provider: Option<SearchProvider>, timeout: Duration) -> Result<Self, WebError> {
        Ok(Self {
            http: http_client(timeout)?,
            provider,
        })
    }

    pub async fn search(
        &self,
        query: &str,
        options: SearchOptions,
    ) -> Result<SearchResponse, WebError> {
        let query = query.trim();
        if query.is_empty() {
            return Err(WebError::EmptyQuery);
        }

        let Some(provider) = &self.provider else {
            return Err(WebError::MissingSearchProvider);
        };

        match provider {
            SearchProvider::Searxng { base_url } => {
                self.search_searxng(base_url, query, options).await
            }
            SearchProvider::Brave { api_key } => self.search_brave(api_key, query, options).await,
        }
    }

    async fn search_searxng(
        &self,
        base_url: &str,
        query: &str,
        options: SearchOptions,
    ) -> Result<SearchResponse, WebError> {
        let mut url = Url::parse(base_url)?.join("search")?;
        url.query_pairs_mut()
            .append_pair("q", query)
            .append_pair("format", "json")
            .append_pair("categories", "general")
            .append_pair("safesearch", "1")
            .append_pair("pageno", "1");

        let response = self
            .http
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .json::<SearxngResponse>()
            .await?;

        let results = response
            .results
            .into_iter()
            .take(options.max_results)
            .map(|result| SearchResult {
                title: clean_text(&result.title),
                url: result.url,
                snippet: clean_text(&result.content.unwrap_or_default()),
                source: result.engine.or(result.category),
                published_at: result.published_date,
            })
            .collect();

        Ok(SearchResponse {
            query: query.to_owned(),
            provider: "searxng".to_owned(),
            results,
        })
    }

    async fn search_brave(
        &self,
        api_key: &str,
        query: &str,
        options: SearchOptions,
    ) -> Result<SearchResponse, WebError> {
        let mut url = Url::parse("https://api.search.brave.com/res/v1/web/search")?;
        url.query_pairs_mut()
            .append_pair("q", query)
            .append_pair("count", &options.max_results.min(20).to_string())
            .append_pair("extra_snippets", "true");

        let response = self
            .http
            .get(url)
            .header("X-Subscription-Token", api_key)
            .send()
            .await?
            .error_for_status()?
            .json::<BraveResponse>()
            .await?;

        let results = response
            .web
            .map(|web| web.results)
            .unwrap_or_default()
            .into_iter()
            .take(options.max_results)
            .map(|result| {
                let mut snippets = Vec::new();
                if let Some(description) = result.description {
                    snippets.push(description);
                }
                snippets.extend(result.extra_snippets);
                SearchResult {
                    title: clean_text(&result.title),
                    url: result.url,
                    snippet: clean_text(&snippets.join("\n")),
                    source: result.profile.and_then(|profile| profile.name),
                    published_at: result.age,
                }
            })
            .collect();

        Ok(SearchResponse {
            query: query.to_owned(),
            provider: "brave".to_owned(),
            results,
        })
    }
}

#[derive(Debug, Clone)]
pub enum SearchProvider {
    Searxng { base_url: String },
    Brave { api_key: String },
}

#[derive(Debug, Clone, Copy)]
pub struct SearchOptions {
    pub max_results: usize,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self { max_results: 5 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResponse {
    pub query: String,
    pub provider: String,
    pub results: Vec<SearchResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub source: Option<String>,
    pub published_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WebFetcher {
    http: Client,
}

impl WebFetcher {
    pub fn new(timeout: Duration) -> Result<Self, WebError> {
        Ok(Self {
            http: http_client(timeout)?,
        })
    }

    pub async fn fetch(&self, url: &str, options: FetchOptions) -> Result<FetchedPage, WebError> {
        let url = validate_public_url(url).await?;
        let response = self
            .http
            .get(url.clone())
            .send()
            .await?
            .error_for_status()?;
        let final_url = response.url().to_string();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let bytes = response.bytes().await?;
        let truncated = bytes.len() > options.max_bytes;
        let bytes = if truncated {
            &bytes[..options.max_bytes]
        } else {
            &bytes
        };
        let raw_text = String::from_utf8_lossy(bytes);
        let text = if content_type
            .as_deref()
            .is_some_and(|value| value.to_ascii_lowercase().contains("html"))
        {
            extract_html_text(&raw_text)
        } else {
            clean_text(&raw_text)
        };

        Ok(FetchedPage {
            url: final_url,
            content_type,
            text,
            bytes: bytes.len(),
            truncated,
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FetchOptions {
    pub max_bytes: usize,
}

impl Default for FetchOptions {
    fn default() -> Self {
        Self {
            max_bytes: 64 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchedPage {
    pub url: String,
    pub content_type: Option<String>,
    pub text: String,
    pub bytes: usize,
    pub truncated: bool,
}

async fn validate_public_url(value: &str) -> Result<Url, WebError> {
    let url = Url::parse(value)?;
    match url.scheme() {
        "http" | "https" => {}
        scheme => return Err(WebError::UnsupportedUrlScheme(scheme.to_owned())),
    }

    let host = url
        .host_str()
        .ok_or_else(|| WebError::MissingHost(value.to_owned()))?;
    validate_public_host(host, url.port_or_known_default()).await?;
    Ok(url)
}

async fn validate_public_host(host: &str, port: Option<u16>) -> Result<(), WebError> {
    let normalized = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if normalized == "localhost"
        || normalized.ends_with(".localhost")
        || normalized.ends_with(".local")
        || !normalized.contains('.')
    {
        return Err(WebError::BlockedHost(host.to_owned()));
    }

    if let Ok(ip) = normalized.parse::<IpAddr>() {
        if is_blocked_ip(ip) {
            return Err(WebError::BlockedHost(host.to_owned()));
        }
        return Ok(());
    }

    let port = port.unwrap_or(443);
    let addrs = lookup_host((normalized.as_str(), port))
        .await
        .map_err(|source| WebError::Dns {
            host: host.to_owned(),
            source,
        })?
        .collect::<Vec<SocketAddr>>();
    if addrs.is_empty() || addrs.iter().any(|addr| is_blocked_ip(addr.ip())) {
        return Err(WebError::BlockedHost(host.to_owned()));
    }
    Ok(())
}

fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_multicast()
                || ip.is_unspecified()
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
        }
    }
}

fn http_client(timeout: Duration) -> Result<Client, WebError> {
    Ok(Client::builder()
        .timeout(timeout)
        .redirect(Policy::none())
        .user_agent("rafael-web-tool/0.1")
        .build()?)
}

fn extract_html_text(html: &str) -> String {
    let document = Html::parse_document(html);
    let selector = Selector::parse("body").expect("body selector is valid");
    let text = document
        .select(&selector)
        .flat_map(|element| element.text())
        .collect::<Vec<_>>()
        .join(" ");
    clean_text(&text)
}

fn clean_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Debug, Deserialize)]
struct SearxngResponse {
    #[serde(default)]
    results: Vec<SearxngResult>,
}

#[derive(Debug, Deserialize)]
struct SearxngResult {
    title: String,
    url: String,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    engine: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default, rename = "publishedDate")]
    published_date: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BraveResponse {
    web: Option<BraveWeb>,
}

#[derive(Debug, Deserialize)]
struct BraveWeb {
    #[serde(default)]
    results: Vec<BraveResult>,
}

#[derive(Debug, Deserialize)]
struct BraveResult {
    title: String,
    url: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    age: Option<String>,
    #[serde(default)]
    profile: Option<BraveProfile>,
    #[serde(default)]
    extra_snippets: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct BraveProfile {
    name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_localhost_fetch_urls() {
        let error = validate_public_url("http://localhost:8080")
            .await
            .unwrap_err();
        assert!(matches!(error, WebError::BlockedHost(_)));
    }

    #[tokio::test]
    async fn rejects_private_ip_fetch_urls() {
        let error = validate_public_url("http://192.168.1.1").await.unwrap_err();
        assert!(matches!(error, WebError::BlockedHost(_)));
    }

    #[test]
    fn extracts_html_body_text() {
        let text = extract_html_text("<html><body><h1>Hello</h1><p>world</p></body></html>");
        assert_eq!(text, "Hello world");
    }

    #[test]
    fn parses_searxng_response() {
        let response: SearxngResponse = serde_json::from_str(
            r#"{"results":[{"title":"A","url":"https://example.com","content":"Snippet","engine":"brave"}]}"#,
        )
        .unwrap();
        assert_eq!(response.results[0].title, "A");
    }

    #[test]
    fn parses_brave_response() {
        let response: BraveResponse = serde_json::from_str(
            r#"{"web":{"results":[{"title":"A","url":"https://example.com","description":"Snippet"}]}}"#,
        )
        .unwrap();
        assert_eq!(response.web.unwrap().results[0].title, "A");
    }
}
