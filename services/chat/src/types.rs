use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatConfigFile {
    pub providers: Vec<StoredProvider>,
    pub settings: ChatSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSettings {
    pub active_provider_id: String,
    pub theme: ThemeName,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeName {
    Charcoal,
    Gruvbox,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    OpenAiCompatible,
    Anthropic,
}

impl ProviderKind {
    pub fn chat_supported(&self) -> bool {
        matches!(self, Self::OpenAiCompatible)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredProvider {
    pub id: String,
    pub name: String,
    pub kind: ProviderKind,
    pub base_url: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicProvider {
    pub id: String,
    pub name: String,
    pub kind: ProviderKind,
    pub base_url: String,
    pub model: String,
    pub has_api_key: bool,
    pub system_prompt: Option<String>,
    pub chat_supported: bool,
}

impl PublicProvider {
    pub fn from_stored(provider: &StoredProvider) -> Self {
        Self {
            id: provider.id.clone(),
            name: provider.name.clone(),
            kind: provider.kind.clone(),
            base_url: provider.base_url.clone(),
            model: provider.model.clone(),
            has_api_key: provider
                .api_key
                .as_deref()
                .is_some_and(|value| !value.is_empty()),
            system_prompt: provider.system_prompt.clone(),
            chat_supported: provider.kind.chat_supported(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub pinned: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub messages: Vec<ChatMessageRecord>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
    pub pinned: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub message_count: usize,
}

impl ConversationSummary {
    pub fn from_conversation(conversation: &Conversation) -> Self {
        Self {
            id: conversation.id.clone(),
            title: conversation.title.clone(),
            pinned: conversation.pinned,
            created_at: conversation.created_at,
            updated_at: conversation.updated_at,
            message_count: conversation.messages.len(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessageRecord {
    pub id: String,
    pub role: ChatRole,
    pub content: String,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    User,
    Assistant,
    System,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateConversationRequest {
    pub title: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateConversationRequest {
    pub pinned: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendMessageRequest {
    pub content: String,
    pub provider_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveProviderRequest {
    pub id: Option<String>,
    pub name: String,
    pub kind: ProviderKind,
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
    pub system_prompt: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSettingsRequest {
    pub active_provider_id: Option<String>,
    pub theme: Option<ThemeName>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatStateResponse {
    pub providers: Vec<PublicProvider>,
    pub active_provider_id: String,
    pub theme: ThemeName,
    pub conversations: Vec<ConversationSummary>,
}
