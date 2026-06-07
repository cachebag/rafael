use chrono::{DateTime, Utc};
use memory::sqlite::{ConversationMemoryMode, MemoryRecord, MemorySettings, MemoryStatus};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicUser {
    pub id: String,
    pub username: String,
    pub first_name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterRequest {
    pub username: String,
    pub password: String,
    pub first_name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthSessionResponse {
    pub token: String,
    pub user: PublicUser,
}

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
    CharcoalLight,
    Gruvbox,
    GruvboxLight,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_mode: Option<ConversationMemoryMode>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<ChatMessageMetadata>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessageMetadata {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_uses: Vec<ChatToolUse>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<ChatSource>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memories: Vec<ChatMemoryUse>,
}

impl ChatMessageMetadata {
    pub fn is_empty(&self) -> bool {
        self.tool_uses.is_empty() && self.sources.is_empty() && self.memories.is_empty()
    }

    pub fn merge(&mut self, other: Self) {
        for tool_use in other.tool_uses {
            if !self.tool_uses.iter().any(|existing| existing == &tool_use) {
                self.tool_uses.push(tool_use);
            }
        }
        for source in other.sources {
            if !self
                .sources
                .iter()
                .any(|existing| existing.url == source.url)
            {
                self.sources.push(source);
            }
        }
        for memory in other.memories {
            if !self
                .memories
                .iter()
                .any(|existing| existing.id == memory.id)
            {
                self.memories.push(memory);
            }
        }
    }

    pub fn into_option(self) -> Option<Self> {
        if self.is_empty() { None } else { Some(self) }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatToolUse {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSource {
    pub title: Option<String>,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMemoryUse {
    pub id: String,
    pub kind: String,
    pub content: String,
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
    #[serde(default)]
    pub memory_mode: Option<ConversationMemoryMode>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateConversationRequest {
    pub pinned: Option<bool>,
    #[serde(default)]
    pub memory_mode: Option<ConversationMemoryMode>,
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
    pub memory: MemoryStateResponse,
    pub conversations: Vec<ConversationSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryStateResponse {
    pub settings: MemorySettings,
    pub counts: MemoryCounts,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryCounts {
    pub pending: usize,
    pub active: usize,
    pub archived: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMemorySettingsRequest {
    pub enabled: Option<bool>,
    pub auto_capture: Option<bool>,
    pub require_approval: Option<bool>,
    pub default_conversation_mode: Option<ConversationMemoryMode>,
    pub memory_budget_chars: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateMemoryRequest {
    pub kind: String,
    pub content: String,
    #[serde(default)]
    pub status: Option<MemoryStatus>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub source_conversation_id: Option<String>,
    #[serde(default)]
    pub source_message_ids: Vec<String>,
    #[serde(default)]
    pub confidence: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMemoryRequest {
    pub kind: Option<String>,
    pub content: Option<String>,
    pub status: Option<MemoryStatus>,
    pub tags: Option<Vec<String>>,
    pub confidence: Option<Option<f64>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListMemoriesQuery {
    pub query: Option<String>,
    pub status: Option<MemoryStatus>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryListResponse {
    pub memories: Vec<MemoryRecord>,
}
