use std::time::Duration;

use anyhow::{Context, bail};
use client::{
    ChatMessage, ChatOptions, ChatRequest, LocalModelClient, ModelClientConfig, ModelInfo,
};

use crate::tools::ChatToolRuntime;
use crate::types::{ChatMessageRecord, ChatRole, ProviderKind, StoredProvider};

pub async fn complete_chat(
    provider: &StoredProvider,
    messages: &[ChatMessageRecord],
    timeout: Duration,
) -> anyhow::Result<String> {
    match provider.kind {
        ProviderKind::OpenAiCompatible => openai_compatible_chat(provider, messages, timeout).await,
        ProviderKind::Anthropic => {
            bail!("anthropic providers can be saved but are not chat-enabled yet")
        }
    }
}

pub async fn stream_chat<F>(
    provider: &StoredProvider,
    messages: &[ChatMessageRecord],
    timeout: Duration,
    tools: Option<&ChatToolRuntime>,
    on_delta: F,
) -> anyhow::Result<String>
where
    F: FnMut(&str),
{
    match provider.kind {
        ProviderKind::OpenAiCompatible => {
            openai_compatible_stream_chat(provider, messages, timeout, tools, on_delta).await
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
) -> anyhow::Result<String> {
    let client = LocalModelClient::new(model_client_config(provider, timeout))
        .context("failed to create model client")?;
    let response = client
        .chat(
            ChatRequest::new(model_messages(provider, messages))
                .with_options(ChatOptions::default()),
        )
        .await
        .context("model endpoint returned an error")?;

    Ok(response.content)
}

async fn openai_compatible_stream_chat<F>(
    provider: &StoredProvider,
    messages: &[ChatMessageRecord],
    timeout: Duration,
    tools: Option<&ChatToolRuntime>,
    on_delta: F,
) -> anyhow::Result<String>
where
    F: FnMut(&str),
{
    if let Some(tools) = tools.filter(|tools| tools.enabled()) {
        return openai_compatible_stream_chat_with_tools(
            provider, messages, timeout, tools, on_delta,
        )
        .await;
    }

    let client = LocalModelClient::new(model_client_config(provider, timeout))
        .context("failed to create model client")?;
    let response = client
        .chat_stream(
            ChatRequest::new(model_messages(provider, messages)),
            on_delta,
        )
        .await
        .context("model endpoint returned an error")?;

    Ok(response.content)
}

async fn openai_compatible_stream_chat_with_tools<F>(
    provider: &StoredProvider,
    messages: &[ChatMessageRecord],
    timeout: Duration,
    tools: &ChatToolRuntime,
    mut on_delta: F,
) -> anyhow::Result<String>
where
    F: FnMut(&str),
{
    let client = LocalModelClient::new(model_client_config(provider, timeout))
        .context("failed to create model client")?;
    let tool_specs = tools.openai_tools()?;
    let tool_options = ChatOptions::default().tools(tool_specs);
    let mut request_messages = model_messages(provider, messages);
    request_messages.insert(0, ChatMessage::system(WEB_TOOL_SYSTEM_PROMPT));

    for _ in 0..tools.max_invocations() {
        let response = client
            .chat(ChatRequest::new(request_messages.clone()).with_options(tool_options.clone()))
            .await
            .context("model endpoint returned an error")?;

        if response.tool_calls.is_empty() {
            if !response.content.is_empty() {
                on_delta(&response.content);
            }
            return Ok(response.content);
        }

        request_messages.push(ChatMessage::assistant_tool_calls(
            response.tool_calls.clone(),
        ));
        for tool_call in response.tool_calls {
            let result = tools.invoke(&tool_call).await;
            request_messages.push(ChatMessage::tool_result(tool_call.id, result));
        }
    }

    let response = client
        .chat_stream(ChatRequest::new(request_messages), on_delta)
        .await
        .context("model endpoint returned an error")?;

    Ok(response.content)
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

fn model_messages(provider: &StoredProvider, messages: &[ChatMessageRecord]) -> Vec<ChatMessage> {
    let mut request_messages = Vec::new();
    if let Some(system_prompt) = provider
        .system_prompt
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        request_messages.push(ChatMessage::system(system_prompt));
    }
    request_messages.extend(messages.iter().filter_map(to_model_message));
    request_messages
}

fn to_model_message(message: &ChatMessageRecord) -> Option<ChatMessage> {
    match message.role {
        ChatRole::User => Some(ChatMessage::user(message.content.clone())),
        ChatRole::Assistant => Some(ChatMessage::assistant(message.content.clone())),
        ChatRole::System => Some(ChatMessage::system(message.content.clone())),
    }
}

const WEB_TOOL_SYSTEM_PROMPT: &str = "\
You may use tools for current or source-grounded information. \
Use web_search for current events, recent facts, or questions that need external sources. \
Use fetch_url only for public http(s) URLs from the user or from search results when you need more detail. \
Do not fetch localhost, private-network, or admin URLs. \
When using web information, cite the source URLs in the final answer.";
