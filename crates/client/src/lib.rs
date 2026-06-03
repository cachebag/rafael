use std::time::Duration;

use common::SecretString;
use futures_util::StreamExt;
use reqwest::Client;
use reqwest::header::AUTHORIZATION;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("failed to build http client: {0}")]
    BuildHttp(#[source] reqwest::Error),
    #[error("failed to call model endpoint: {0}")]
    Http(#[source] reqwest::Error),
    #[error("model endpoint returned no choices")]
    NoChoices,
    #[error("model response did not contain json")]
    NoJson,
    #[error("failed to parse json model output: {0}")]
    Json(#[from] serde_json::Error),
    #[error("model stream returned invalid utf8: {0}")]
    Utf8(#[from] std::str::Utf8Error),
}

#[derive(Debug, Clone)]
pub struct ModelClientConfig {
    pub base_url: String,
    pub model: String,
    pub timeout: Option<Duration>,
    pub authorization: Option<AuthorizationHeader>,
}

impl ModelClientConfig {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            model: model.into(),
            timeout: None,
            authorization: None,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn with_bearer_token(mut self, token: impl Into<String>) -> Self {
        self.authorization = Some(AuthorizationHeader::bearer(token));
        self
    }

    pub fn with_authorization(mut self, authorization: AuthorizationHeader) -> Self {
        self.authorization = Some(authorization);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizationHeader {
    value: SecretString,
}

impl AuthorizationHeader {
    pub fn bearer(token: impl Into<String>) -> Self {
        Self {
            value: SecretString::new(format!("Bearer {}", token.into())),
        }
    }

    pub fn raw(value: impl Into<String>) -> Self {
        Self {
            value: SecretString::new(value),
        }
    }

    pub fn expose_value(&self) -> &str {
        self.value.expose_secret()
    }
}

#[derive(Clone)]
pub struct LocalModelClient {
    http: Client,
    base_url: String,
    model: String,
    authorization: Option<AuthorizationHeader>,
}

impl LocalModelClient {
    pub fn new(config: ModelClientConfig) -> Result<Self, ClientError> {
        let mut builder = Client::builder();
        if let Some(timeout) = config.timeout {
            builder = builder.timeout(timeout);
        }

        Ok(Self {
            http: builder.build().map_err(ClientError::BuildHttp)?,
            base_url: config.base_url.trim_end_matches('/').to_owned(),
            model: config.model,
            authorization: config.authorization,
        })
    }

    pub fn with_http_client(config: ModelClientConfig, http: Client) -> Self {
        Self {
            http,
            base_url: config.base_url.trim_end_matches('/').to_owned(),
            model: config.model,
            authorization: config.authorization,
        }
    }

    pub async fn list_models(&self) -> Result<Vec<ModelInfo>, ClientError> {
        let mut http_request = self.http.get(format!("{}/models", self.base_url));

        if let Some(authorization) = &self.authorization {
            http_request = http_request.header(AUTHORIZATION, authorization.expose_value());
        }

        let response = http_request
            .send()
            .await
            .map_err(ClientError::Http)?
            .error_for_status()
            .map_err(ClientError::Http)?
            .json::<ModelsResponse>()
            .await
            .map_err(ClientError::Http)?;

        Ok(response.into_models())
    }

    pub async fn chat(&self, request: ChatRequest) -> Result<ChatCompletion, ClientError> {
        let mut http_request = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .json(&ChatCompletionsRequestBody {
                model: &self.model,
                messages: &request.messages,
                temperature: request.options.temperature,
                max_tokens: request.options.max_tokens,
                stream: false,
                response_format: request.options.response_format,
            });

        if let Some(authorization) = &self.authorization {
            http_request = http_request.header(AUTHORIZATION, authorization.expose_value());
        }

        let response = http_request
            .send()
            .await
            .map_err(ClientError::Http)?
            .error_for_status()
            .map_err(ClientError::Http)?
            .json::<ChatCompletionsResponse>()
            .await
            .map_err(ClientError::Http)?;

        let choice = response
            .choices
            .into_iter()
            .next()
            .ok_or(ClientError::NoChoices)?;
        Ok(ChatCompletion {
            content: choice.message.content,
            finish_reason: choice.finish_reason,
            usage: response.usage,
        })
    }

    pub async fn chat_stream<F>(
        &self,
        request: ChatRequest,
        mut on_delta: F,
    ) -> Result<ChatCompletion, ClientError>
    where
        F: FnMut(&str),
    {
        let mut http_request = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .json(&ChatCompletionsRequestBody {
                model: &self.model,
                messages: &request.messages,
                temperature: request.options.temperature,
                max_tokens: request.options.max_tokens,
                stream: true,
                response_format: request.options.response_format,
            });

        if let Some(authorization) = &self.authorization {
            http_request = http_request.header(AUTHORIZATION, authorization.expose_value());
        }

        let mut stream = http_request
            .send()
            .await
            .map_err(ClientError::Http)?
            .error_for_status()
            .map_err(ClientError::Http)?
            .bytes_stream();

        let mut buffer = String::new();
        let mut content = String::new();
        let mut finish_reason = None;
        let mut usage = None;

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(ClientError::Http)?;
            buffer.push_str(std::str::from_utf8(&chunk)?);

            while let Some(line_end) = buffer.find('\n') {
                let line = buffer[..line_end].trim_end_matches('\r').to_owned();
                buffer.drain(..=line_end);

                let Some(data) = line.strip_prefix("data:") else {
                    continue;
                };
                let data = data.trim();
                if data.is_empty() {
                    continue;
                }
                if data == "[DONE]" {
                    return Ok(ChatCompletion {
                        content,
                        finish_reason,
                        usage,
                    });
                }

                let chunk = serde_json::from_str::<ChatCompletionsStreamResponse>(data)?;
                if let Some(next_usage) = chunk.usage {
                    usage = Some(next_usage);
                }

                for choice in chunk.choices {
                    if let Some(next_finish_reason) = choice.finish_reason {
                        finish_reason = Some(next_finish_reason);
                    }
                    if let Some(delta) = choice.delta.content {
                        content.push_str(&delta);
                        on_delta(&delta);
                    }
                }
            }
        }

        Ok(ChatCompletion {
            content,
            finish_reason,
            usage,
        })
    }

    pub async fn complete_text(
        &self,
        system: impl Into<String>,
        user: impl Into<String>,
        options: ChatOptions,
    ) -> Result<String, ClientError> {
        Ok(self
            .chat(
                ChatRequest::new(vec![ChatMessage::system(system), ChatMessage::user(user)])
                    .with_options(options),
            )
            .await?
            .content)
    }

    pub async fn complete_json<T>(
        &self,
        system: impl Into<String>,
        user: impl Into<String>,
        options: ChatOptions,
    ) -> Result<T, ClientError>
    where
        T: DeserializeOwned,
    {
        let content = self
            .complete_text(system, user, options.json_object())
            .await?;
        parse_json_output(&content)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub options: ChatOptions,
}

impl ChatRequest {
    pub fn new(messages: Vec<ChatMessage>) -> Self {
        Self {
            messages,
            options: ChatOptions::default(),
        }
    }

    pub fn with_options(mut self, options: ChatOptions) -> Self {
        self.options = options;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::System,
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Assistant,
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct ChatOptions {
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub response_format: Option<ResponseFormat>,
}

impl ChatOptions {
    pub fn temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }

    pub fn max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    pub fn json_object(mut self) -> Self {
        self.response_format = Some(ResponseFormat::JsonObject);
        self
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseFormat {
    JsonObject,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletion {
    pub content: String,
    pub finish_reason: Option<String>,
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

pub fn parse_json_output<T>(raw: &str) -> Result<T, ClientError>
where
    T: DeserializeOwned,
{
    serde_json::from_str(&extract_json_output(raw)?).map_err(ClientError::Json)
}

pub fn extract_json_output(raw: &str) -> Result<String, ClientError> {
    let trimmed = strip_markdown_fence(raw.trim());
    let Some((start, end)) = find_json_span(trimmed) else {
        return Err(ClientError::NoJson);
    };
    Ok(trimmed[start..end].to_owned())
}

fn strip_markdown_fence(raw: &str) -> &str {
    if !raw.starts_with("```") {
        return raw;
    }
    let Some((_, rest)) = raw.split_once('\n') else {
        return raw;
    };
    rest.trim().strip_suffix("```").unwrap_or(rest).trim()
}

fn find_json_span(value: &str) -> Option<(usize, usize)> {
    let start = value
        .char_indices()
        .find_map(|(index, ch)| matches!(ch, '{' | '[').then_some(index))?;
    let mut stack = Vec::new();
    let mut in_string = false;
    let mut escaped = false;

    for (offset, ch) in value[start..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if in_string {
            match ch {
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => stack.push('}'),
            '[' => stack.push(']'),
            '}' | ']' => {
                if stack.pop() != Some(ch) {
                    return None;
                }
                if stack.is_empty() {
                    let end = start + offset + ch.len_utf8();
                    return Some((start, end));
                }
            }
            _ => {}
        }
    }

    None
}

#[derive(Debug, Serialize)]
struct ChatCompletionsRequestBody<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormat>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionsResponse {
    choices: Vec<ChatChoice>,
    usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatChoiceMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatChoiceMessage {
    content: String,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionsStreamResponse {
    choices: Vec<ChatStreamChoice>,
    usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
struct ChatStreamChoice {
    delta: ChatStreamDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatStreamDelta {
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    #[serde(default)]
    data: Vec<ModelInfo>,
    #[serde(default)]
    models: Vec<LlamaCppModelInfo>,
}

impl ModelsResponse {
    fn into_models(self) -> Vec<ModelInfo> {
        if !self.data.is_empty() {
            return self.data;
        }

        self.models
            .into_iter()
            .filter_map(LlamaCppModelInfo::into_model)
            .collect()
    }
}

#[derive(Debug, Deserialize)]
struct LlamaCppModelInfo {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

impl LlamaCppModelInfo {
    fn into_model(self) -> Option<ModelInfo> {
        let id = self
            .model
            .as_deref()
            .or(self.name.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())?
            .to_owned();

        Some(ModelInfo {
            id,
            name: self.name,
            description: self.description,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_fenced_json() {
        let json = extract_json_output("```json\n{\"a\":1}\n```").unwrap();
        assert_eq!(json, "{\"a\":1}");
    }

    #[test]
    fn extracts_embedded_json_without_breaking_strings() {
        let json = extract_json_output("ok: {\"a\":\"}\"} done").unwrap();
        assert_eq!(json, "{\"a\":\"}\"}");
    }

    #[test]
    fn extracts_json_with_trailing_text() {
        let json = extract_json_output("{\"a\":1}\nextra").unwrap();
        assert_eq!(json, "{\"a\":1}");
    }

    #[test]
    fn parses_json_output() {
        let value: serde_json::Value = parse_json_output("```json\n{\"ok\":true}\n```").unwrap();
        assert_eq!(value["ok"], true);
    }

    #[test]
    fn bearer_authorization_is_redacted_in_debug() {
        let authorization = AuthorizationHeader::bearer("secret-token");

        assert_eq!(authorization.expose_value(), "Bearer secret-token");
        assert_eq!(
            format!("{authorization:?}"),
            "AuthorizationHeader { value: SecretString(<redacted>) }"
        );
    }

    #[test]
    fn parses_openai_models_response() {
        let response: ModelsResponse = serde_json::from_str(
            r#"{"object":"list","data":[{"id":"gemma-everyday","object":"model"}]}"#,
        )
        .unwrap();

        assert_eq!(
            response.into_models(),
            vec![ModelInfo {
                id: "gemma-everyday".to_owned(),
                name: None,
                description: None,
            }]
        );
    }

    #[test]
    fn parses_llama_cpp_models_response() {
        let response: ModelsResponse =
            serde_json::from_str(r#"{"models":[{"name":"Qwen","model":"Qwen/Qwen2.5-Coder"}]}"#)
                .unwrap();

        assert_eq!(
            response.into_models(),
            vec![ModelInfo {
                id: "Qwen/Qwen2.5-Coder".to_owned(),
                name: Some("Qwen".to_owned()),
                description: None,
            }]
        );
    }
}
