use std::time::Duration;

use anyhow::{Context, bail};
use client::{ChatMessage, ChatOptions, ChatRequest, LocalModelClient, ModelClientConfig};

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
    on_delta: F,
) -> anyhow::Result<String>
where
    F: FnMut(&str),
{
    match provider.kind {
        ProviderKind::OpenAiCompatible => {
            openai_compatible_stream_chat(provider, messages, timeout, on_delta).await
        }
        ProviderKind::Anthropic => {
            bail!("anthropic providers can be saved but are not chat-enabled yet")
        }
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
    on_delta: F,
) -> anyhow::Result<String>
where
    F: FnMut(&str),
{
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
