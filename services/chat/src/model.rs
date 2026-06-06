use std::time::Duration;

use anyhow::{Context, bail};
use client::{
    ChatMessage, ChatOptions, ChatRequest, LocalModelClient, ModelClientConfig, ModelInfo,
};

use crate::tools::ChatToolRuntime;
use crate::types::{
    ChatMessageMetadata, ChatMessageRecord, ChatRole, ProviderKind, StoredProvider,
};

#[derive(Debug, Clone)]
pub struct StreamedChatResponse {
    pub content: String,
    pub metadata: Option<ChatMessageMetadata>,
}

pub async fn complete_chat(
    provider: &StoredProvider,
    messages: &[ChatMessageRecord],
    timeout: Duration,
    max_context_chars: usize,
) -> anyhow::Result<String> {
    match provider.kind {
        ProviderKind::OpenAiCompatible => {
            openai_compatible_chat(provider, messages, timeout, max_context_chars).await
        }
        ProviderKind::Anthropic => {
            bail!("anthropic providers can be saved but are not chat-enabled yet")
        }
    }
}

pub async fn stream_chat<F, G>(
    provider: &StoredProvider,
    messages: &[ChatMessageRecord],
    timeout: Duration,
    max_context_chars: usize,
    tools: Option<&ChatToolRuntime>,
    on_delta: F,
    on_tool: G,
) -> anyhow::Result<StreamedChatResponse>
where
    F: FnMut(&str),
    G: FnMut(&str),
{
    match provider.kind {
        ProviderKind::OpenAiCompatible => {
            openai_compatible_stream_chat(
                provider,
                messages,
                timeout,
                max_context_chars,
                tools,
                on_delta,
                on_tool,
            )
            .await
        }
        ProviderKind::Anthropic => {
            bail!("anthropic providers can be saved but are not chat-enabled yet")
        }
    }
}

pub async fn list_models(
    provider: &StoredProvider,
    timeout: Duration,
) -> anyhow::Result<Vec<ModelInfo>> {
    match provider.kind {
        ProviderKind::OpenAiCompatible => {
            let client = LocalModelClient::new(model_client_config(provider, timeout))
                .context("failed to create model client")?;
            client
                .list_models()
                .await
                .context("model endpoint returned an error")
        }
        ProviderKind::Anthropic => Ok(Vec::new()),
    }
}

async fn openai_compatible_chat(
    provider: &StoredProvider,
    messages: &[ChatMessageRecord],
    timeout: Duration,
    max_context_chars: usize,
) -> anyhow::Result<String> {
    let client = LocalModelClient::new(model_client_config(provider, timeout))
        .context("failed to create model client")?;
    let response = client
        .chat(
            ChatRequest::new(model_messages(provider, messages, max_context_chars, &[]))
                .with_options(ChatOptions::default()),
        )
        .await
        .context("model endpoint returned an error")?;

    Ok(response.content)
}

async fn openai_compatible_stream_chat<F, G>(
    provider: &StoredProvider,
    messages: &[ChatMessageRecord],
    timeout: Duration,
    max_context_chars: usize,
    tools: Option<&ChatToolRuntime>,
    on_delta: F,
    on_tool: G,
) -> anyhow::Result<StreamedChatResponse>
where
    F: FnMut(&str),
    G: FnMut(&str),
{
    if let Some(tools) = tools.filter(|tools| tools.enabled()) {
        return openai_compatible_stream_chat_with_tools(
            provider,
            messages,
            timeout,
            tools,
            on_delta,
            on_tool,
            max_context_chars,
        )
        .await;
    }

    let client = LocalModelClient::new(model_client_config(provider, timeout))
        .context("failed to create model client")?;
    let response = client
        .chat_stream(
            ChatRequest::new(model_messages(provider, messages, max_context_chars, &[])),
            on_delta,
        )
        .await
        .context("model endpoint returned an error")?;

    Ok(StreamedChatResponse {
        content: response.content,
        metadata: None,
    })
}

async fn openai_compatible_stream_chat_with_tools<F, G>(
    provider: &StoredProvider,
    messages: &[ChatMessageRecord],
    timeout: Duration,
    tools: &ChatToolRuntime,
    mut on_delta: F,
    mut on_tool: G,
    max_context_chars: usize,
) -> anyhow::Result<StreamedChatResponse>
where
    F: FnMut(&str),
    G: FnMut(&str),
{
    let client = LocalModelClient::new(model_client_config(provider, timeout))
        .context("failed to create model client")?;
    let tool_specs = tools.openai_tools()?;
    let tool_options = ChatOptions::default().tools(tool_specs);
    let mut request_messages = model_messages(
        provider,
        messages,
        max_context_chars,
        &[WEB_TOOL_SYSTEM_PROMPT],
    );
    let mut metadata = ChatMessageMetadata::default();

    for _ in 0..tools.max_invocations() {
        let response = client
            .chat_stream(
                ChatRequest::new(request_messages.clone()).with_options(tool_options.clone()),
                &mut on_delta,
            )
            .await
            .context("model endpoint returned an error")?;

        if response.tool_calls.is_empty() {
            return Ok(StreamedChatResponse {
                content: response.content,
                metadata: metadata.into_option(),
            });
        }

        request_messages.push(ChatMessage::assistant_tool_calls(
            response.tool_calls.clone(),
        ));
        for tool_call in response.tool_calls {
            on_tool(&tool_call.function.name);
            let result = tools.invoke(&tool_call).await;
            metadata.merge(result.metadata);
            request_messages.push(ChatMessage::tool_result(tool_call.id, result.content));
        }
    }

    let response = client
        .chat_stream(ChatRequest::new(request_messages), on_delta)
        .await
        .context("model endpoint returned an error")?;

    Ok(StreamedChatResponse {
        content: response.content,
        metadata: metadata.into_option(),
    })
}

fn model_client_config(provider: &StoredProvider, timeout: Duration) -> ModelClientConfig {
    let mut config =
        ModelClientConfig::new(&provider.base_url, &provider.model).with_timeout(timeout);
    if let Some(api_key) = provider
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        config = config.with_bearer_token(api_key);
    }
    config
}

fn model_messages(
    provider: &StoredProvider,
    messages: &[ChatMessageRecord],
    max_context_chars: usize,
    extra_system_prompts: &[&str],
) -> Vec<ChatMessage> {
    let mut request_messages = Vec::new();
    let mut remaining_chars = max_context_chars;

    if let Some(system_prompt) = provider
        .system_prompt
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        push_budgeted_message(
            &mut request_messages,
            ChatRole::System,
            system_prompt,
            &mut remaining_chars,
        );
    }

    for system_prompt in extra_system_prompts {
        push_budgeted_message(
            &mut request_messages,
            ChatRole::System,
            system_prompt,
            &mut remaining_chars,
        );
    }

    let mut packed_history = Vec::new();
    for message in messages.iter().rev() {
        let allow_truncate = packed_history.is_empty();
        let Some(chat_message) = to_model_message(message, &mut remaining_chars, allow_truncate)
        else {
            continue;
        };
        packed_history.push(chat_message);
        if remaining_chars == 0 {
            break;
        }
    }
    packed_history.reverse();
    request_messages.extend(packed_history);
    request_messages
}

fn to_model_message(
    message: &ChatMessageRecord,
    remaining_chars: &mut usize,
    allow_truncate: bool,
) -> Option<ChatMessage> {
    match message.role {
        ChatRole::User => budgeted_message(
            ChatRole::User,
            &message.content,
            remaining_chars,
            allow_truncate,
        ),
        ChatRole::Assistant => budgeted_message(
            ChatRole::Assistant,
            &message.content,
            remaining_chars,
            allow_truncate,
        ),
        ChatRole::System => budgeted_message(
            ChatRole::System,
            &message.content,
            remaining_chars,
            allow_truncate,
        ),
    }
}

fn push_budgeted_message(
    messages: &mut Vec<ChatMessage>,
    role: ChatRole,
    content: &str,
    remaining_chars: &mut usize,
) {
    if let Some(message) = budgeted_message(role, content, remaining_chars, true) {
        messages.push(message);
    }
}

fn budgeted_message(
    role: ChatRole,
    content: &str,
    remaining_chars: &mut usize,
    allow_truncate: bool,
) -> Option<ChatMessage> {
    let content = content.trim();
    if content.is_empty() || *remaining_chars == 0 {
        return None;
    }

    let content_chars = content.chars().count();
    if content_chars > *remaining_chars && !allow_truncate {
        return None;
    }

    let content = if content_chars > *remaining_chars {
        truncate_message_content(content, *remaining_chars)
    } else {
        content.to_owned()
    };
    *remaining_chars = remaining_chars.saturating_sub(content.chars().count());

    match role {
        ChatRole::User => Some(ChatMessage::user(content)),
        ChatRole::Assistant => Some(ChatMessage::assistant(content)),
        ChatRole::System => Some(ChatMessage::system(content)),
    }
}

fn truncate_message_content(content: &str, max_chars: usize) -> String {
    const MARKER: &str = "\n\n[Earlier content omitted to fit the model context.]\n\n";
    if max_chars == 0 {
        return String::new();
    }
    if max_chars <= MARKER.chars().count() + 1 {
        return content.chars().take(max_chars).collect();
    }

    let marker_chars = MARKER.chars().count();
    let available = max_chars - marker_chars;
    let head_chars = available / 2;
    let tail_chars = available - head_chars;
    let head = content.chars().take(head_chars).collect::<String>();
    let tail = content
        .chars()
        .rev()
        .take(tail_chars)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("{head}{MARKER}{tail}")
}

const WEB_TOOL_SYSTEM_PROMPT: &str = "\
You may use tools for current or source-grounded information. \
Use web_search for current events, recent facts, or questions that need external sources. \
When web_search returns fetched_pages, use those extracts as source material before relying on snippets. \
Use fetch_url only for public http(s) URLs from the user or from search results when you need more detail. \
Do not fetch localhost, private-network, or admin URLs. \
When using web information, cite the source URLs in the final answer. \
Do not add generic accuracy warnings, disclaimers, or warning banners; state uncertainty briefly only when it directly affects the answer.";

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;

    #[test]
    fn model_messages_keeps_newest_context_within_budget() {
        let provider = test_provider();
        let messages = vec![
            message(ChatRole::User, "old user message that should not fit"),
            message(
                ChatRole::Assistant,
                "old assistant message that should not fit",
            ),
            message(ChatRole::User, "new question"),
        ];

        let packed = model_messages(&provider, &messages, 20, &[]);

        assert_eq!(packed.len(), 1);
        assert_eq!(packed[0].content, "new question");
    }

    #[test]
    fn model_messages_truncates_oversized_latest_message() {
        let provider = test_provider();
        let messages = vec![message(
            ChatRole::User,
            "this one message is too large for the configured context budget",
        )];

        let packed = model_messages(&provider, &messages, 32, &[]);

        assert_eq!(packed.len(), 1);
        assert!(packed[0].content.chars().count() <= 32);
        assert!(packed[0].content.starts_with("this"));
    }

    fn test_provider() -> StoredProvider {
        StoredProvider {
            id: "test".to_owned(),
            name: "Test".to_owned(),
            kind: ProviderKind::OpenAiCompatible,
            base_url: "http://localhost:8080/v1".to_owned(),
            model: "test-model".to_owned(),
            api_key: None,
            system_prompt: None,
        }
    }

    fn message(role: ChatRole, content: &str) -> ChatMessageRecord {
        ChatMessageRecord {
            id: "msg".to_owned(),
            role,
            content: content.to_owned(),
            created_at: Utc::now(),
            provider_id: None,
            metadata: None,
        }
    }
}
