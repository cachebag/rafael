use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, AeadCore, KeyInit, OsRng},
};
use anyhow::{Context, bail};
use chrono::Utc;
use common::SafeComponent;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::io::AsyncWriteExt;

use crate::types::{
    ChatConfigFile, ChatSettings, Conversation, ConversationSummary, StoredProvider, ThemeName,
};

#[derive(Debug, Clone)]
pub struct ChatStore {
    data_dir: PathBuf,
    config_path: PathBuf,
    conversations_dir: PathBuf,
    conversation_obfuscation_key: Option<[u8; 32]>,
}

impl ChatStore {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        let data_dir = data_dir.into();
        Self {
            config_path: data_dir.join("config.json"),
            conversations_dir: data_dir.join("conversations"),
            data_dir,
            conversation_obfuscation_key: None,
        }
    }

    pub fn with_conversation_obfuscation_key(mut self, key: [u8; 32]) -> Self {
        self.conversation_obfuscation_key = Some(key);
        self
    }

    pub async fn load_or_initialize_config(
        &self,
        default_provider: &StoredProvider,
    ) -> anyhow::Result<ChatConfigFile> {
        self.ensure_dirs().await?;
        match read_json::<ChatConfigFile>(&self.config_path).await {
            Ok(config) => {
                let (repaired, changed) = repair_config(config, default_provider);
                if changed {
                    self.save_config(&repaired).await?;
                }
                Ok(repaired)
            }
            Err(err) if is_not_found(&err) => {
                let config = ChatConfigFile {
                    providers: vec![default_provider.clone()],
                    settings: ChatSettings {
                        active_provider_id: default_provider.id.clone(),
                        theme: ThemeName::Charcoal,
                    },
                };
                self.save_config(&config).await?;
                Ok(config)
            }
            Err(err) => Err(err),
        }
    }

    pub async fn save_config(&self, config: &ChatConfigFile) -> anyhow::Result<()> {
        self.ensure_dirs().await?;
        write_json(&self.config_path, config).await
    }

    pub async fn list_conversations(&self) -> anyhow::Result<Vec<ConversationSummary>> {
        self.ensure_dirs().await?;
        let mut summaries = Vec::new();
        let mut entries = tokio::fs::read_dir(&self.conversations_dir)
            .await
            .with_context(|| {
                format!(
                    "failed to read conversations directory {}",
                    self.conversations_dir.display()
                )
            })?;

        while let Some(entry) = entries
            .next_entry()
            .await
            .context("failed to read directory entry")?
        {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            if !entry
                .file_type()
                .await
                .with_context(|| format!("failed to read file type for {}", path.display()))?
                .is_file()
            {
                continue;
            }

            let read = self.read_conversation_file(&path).await?;
            if read.migrate_to_obfuscated {
                self.save_conversation(&read.conversation).await?;
            }
            let conversation = read.conversation;
            summaries.push(ConversationSummary::from_conversation(&conversation));
        }

        summaries.sort_by(|left, right| {
            right
                .pinned
                .cmp(&left.pinned)
                .then_with(|| right.updated_at.cmp(&left.updated_at))
        });
        Ok(summaries)
    }

    pub async fn create_conversation(&self, title: Option<String>) -> anyhow::Result<Conversation> {
        self.ensure_dirs().await?;
        let now = Utc::now();
        let conversation = Conversation {
            id: new_id("conv"),
            title: clean_optional(title).unwrap_or_else(|| "New conversation".to_owned()),
            pinned: false,
            memory_mode: None,
            created_at: now,
            updated_at: now,
            messages: Vec::new(),
        };
        self.save_conversation(&conversation).await?;
        Ok(conversation)
    }

    pub async fn get_conversation(&self, id: &str) -> anyhow::Result<Option<Conversation>> {
        let path = self.conversation_path(id)?;
        match self.read_conversation_file(&path).await {
            Ok(read) => {
                if read.migrate_to_obfuscated {
                    self.save_conversation(&read.conversation).await?;
                }
                Ok(Some(read.conversation))
            }
            Err(err) if is_not_found(&err) => Ok(None),
            Err(err) => Err(err),
        }
    }

    pub async fn save_conversation(&self, conversation: &Conversation) -> anyhow::Result<()> {
        self.ensure_dirs().await?;
        let path = self.conversation_path(&conversation.id)?;
        match self.conversation_obfuscation_key {
            Some(key) => write_obfuscated_conversation(&path, conversation, &key).await,
            None => write_json(&path, conversation).await,
        }
    }

    pub async fn delete_conversation(&self, id: &str) -> anyhow::Result<bool> {
        let path = self.conversation_path(id)?;
        match tokio::fs::remove_file(&path).await {
            Ok(()) => Ok(true),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(source) => Err(source)
                .with_context(|| format!("failed to delete conversation file {}", path.display())),
        }
    }

    pub async fn delete_all_conversations(&self) -> anyhow::Result<usize> {
        self.ensure_dirs().await?;
        let mut deleted = 0;
        let mut entries = tokio::fs::read_dir(&self.conversations_dir)
            .await
            .with_context(|| {
                format!(
                    "failed to read conversations directory {}",
                    self.conversations_dir.display()
                )
            })?;

        while let Some(entry) = entries
            .next_entry()
            .await
            .context("failed to read directory entry")?
        {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            if !entry
                .file_type()
                .await
                .with_context(|| format!("failed to read file type for {}", path.display()))?
                .is_file()
            {
                continue;
            }

            match tokio::fs::remove_file(&path).await {
                Ok(()) => deleted += 1,
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
                Err(source) => {
                    return Err(source).with_context(|| {
                        format!("failed to delete conversation file {}", path.display())
                    });
                }
            }
        }

        Ok(deleted)
    }

    fn conversation_path(&self, id: &str) -> anyhow::Result<PathBuf> {
        let id = SafeComponent::parse(id.to_owned()).context("invalid conversation id")?;
        let path = self.conversations_dir.join(format!("{}.json", id.as_str()));
        if !path.starts_with(&self.conversations_dir) {
            bail!("conversation path escaped data directory");
        }
        Ok(path)
    }

    async fn ensure_dirs(&self) -> anyhow::Result<()> {
        tokio::fs::create_dir_all(&self.data_dir)
            .await
            .with_context(|| {
                format!(
                    "failed to create data directory {}",
                    self.data_dir.display()
                )
            })?;
        tokio::fs::create_dir_all(&self.conversations_dir)
            .await
            .with_context(|| {
                format!(
                    "failed to create conversations directory {}",
                    self.conversations_dir.display()
                )
            })?;
        reject_symlink(&self.data_dir).await?;
        reject_symlink(&self.conversations_dir).await
    }

    async fn read_conversation_file(&self, path: &Path) -> anyhow::Result<ReadConversation> {
        reject_symlink(path).await?;
        let bytes = tokio::fs::read(path)
            .await
            .with_context(|| format!("failed to read conversation file {}", path.display()))?;

        if let Some(key) = self.conversation_obfuscation_key {
            if let Some(envelope) = parse_obfuscated_envelope(&bytes)? {
                return Ok(ReadConversation {
                    conversation: decrypt_conversation(&envelope, &key).with_context(|| {
                        format!("failed to decrypt conversation {}", path.display())
                    })?,
                    migrate_to_obfuscated: false,
                });
            }

            let conversation = serde_json::from_slice(&bytes)
                .with_context(|| format!("failed to parse conversation file {}", path.display()))?;
            return Ok(ReadConversation {
                conversation,
                migrate_to_obfuscated: true,
            });
        }

        Ok(ReadConversation {
            conversation: serde_json::from_slice(&bytes)
                .with_context(|| format!("failed to parse conversation file {}", path.display()))?,
            migrate_to_obfuscated: false,
        })
    }
}

struct ReadConversation {
    conversation: Conversation,
    migrate_to_obfuscated: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ObfuscatedConversationFile {
    format: String,
    nonce: String,
    ciphertext: String,
}

const OBFUSCATED_CONVERSATION_FORMAT: &str = "rafael-chat-conversation-v1";

async fn write_obfuscated_conversation(
    path: &Path,
    conversation: &Conversation,
    key: &[u8; 32],
) -> anyhow::Result<()> {
    reject_symlink(path).await?;
    let plaintext = serde_json::to_vec(conversation)
        .with_context(|| format!("failed to serialize conversation {}", conversation.id))?;
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|error| anyhow::anyhow!("invalid conversation obfuscation key: {error}"))?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_ref())
        .map_err(|error| anyhow::anyhow!("failed to encrypt conversation: {error}"))?;
    let envelope = ObfuscatedConversationFile {
        format: OBFUSCATED_CONVERSATION_FORMAT.to_owned(),
        nonce: hex_encode(&nonce),
        ciphertext: hex_encode(&ciphertext),
    };
    write_json(path, &envelope).await
}

fn parse_obfuscated_envelope(bytes: &[u8]) -> anyhow::Result<Option<ObfuscatedConversationFile>> {
    let Ok(envelope) = serde_json::from_slice::<ObfuscatedConversationFile>(bytes) else {
        return Ok(None);
    };
    if envelope.format == OBFUSCATED_CONVERSATION_FORMAT {
        Ok(Some(envelope))
    } else {
        Ok(None)
    }
}

fn decrypt_conversation(
    envelope: &ObfuscatedConversationFile,
    key: &[u8; 32],
) -> anyhow::Result<Conversation> {
    let nonce: [u8; 12] = hex_decode(&envelope.nonce)
        .context("invalid conversation nonce")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("invalid conversation nonce length"))?;
    let ciphertext = hex_decode(&envelope.ciphertext).context("invalid conversation ciphertext")?;
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|error| anyhow::anyhow!("invalid conversation obfuscation key: {error}"))?;
    let nonce = Nonce::from(nonce);
    let plaintext = cipher
        .decrypt(&nonce, ciphertext.as_ref())
        .map_err(|error| anyhow::anyhow!("failed to decrypt conversation: {error}"))?;
    serde_json::from_slice(&plaintext).context("failed to parse decrypted conversation")
}

pub fn clean_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

pub fn new_id(prefix: &str) -> String {
    let sequence = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{prefix}-{nanos}-{sequence}")
}

fn repair_config(
    mut config: ChatConfigFile,
    default_provider: &StoredProvider,
) -> (ChatConfigFile, bool) {
    let mut changed = false;

    if config.providers.is_empty() {
        config.providers.push(default_provider.clone());
        changed = true;
    }

    if config.settings.active_provider_id.trim().is_empty() {
        config.settings.active_provider_id = config
            .providers
            .first()
            .map(|provider| provider.id.clone())
            .unwrap_or_else(|| default_provider.id.clone());
        changed = true;
    }

    (config, changed)
}

async fn read_json<T>(path: &Path) -> anyhow::Result<T>
where
    T: DeserializeOwned,
{
    reject_symlink(path).await?;
    let bytes = tokio::fs::read(path)
        .await
        .with_context(|| format!("failed to read json file {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse json file {}", path.display()))
}

async fn write_json<T>(path: &Path, value: &T) -> anyhow::Result<()>
where
    T: Serialize,
{
    reject_symlink(path).await?;
    let bytes = serde_json::to_vec_pretty(value)
        .with_context(|| format!("failed to serialize json file {}", path.display()))?;
    atomic_write(path, &bytes).await
}

async fn atomic_write(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let tmp_path = path.with_extension(format!("tmp-{}-{}", std::process::id(), new_id("write")));
    let mut file = tokio::fs::File::create(&tmp_path)
        .await
        .with_context(|| format!("failed to create temp file {}", tmp_path.display()))?;
    file.write_all(bytes)
        .await
        .with_context(|| format!("failed to write temp file {}", tmp_path.display()))?;
    file.flush()
        .await
        .with_context(|| format!("failed to flush temp file {}", tmp_path.display()))?;
    tokio::fs::rename(&tmp_path, path).await.with_context(|| {
        format!(
            "failed to replace {} with {}",
            path.display(),
            tmp_path.display()
        )
    })?;
    Ok(())
}

async fn reject_symlink(path: &Path) -> anyhow::Result<()> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!("refusing to use symlink path {}", path.display())
        }
        Ok(_) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => {
            Err(source).with_context(|| format!("failed to inspect path {}", path.display()))
        }
    }
}

fn is_not_found(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|source| source.kind() == std::io::ErrorKind::NotFound)
    })
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn hex_decode(value: &str) -> anyhow::Result<Vec<u8>> {
    let value = value.trim();
    if value.len() % 2 != 0 {
        bail!("hex value has odd length");
    }

    let mut bytes = Vec::with_capacity(value.len() / 2);
    for chunk in value.as_bytes().chunks_exact(2) {
        let high = hex_value(chunk[0])?;
        let low = hex_value(chunk[1])?;
        bytes.push((high << 4) | low);
    }
    Ok(bytes)
}

fn hex_value(byte: u8) -> anyhow::Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => bail!("invalid hex digit"),
    }
}

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChatMessageRecord, ChatRole, ProviderKind};

    #[test]
    fn repair_config_preserves_runtime_model_active_provider() {
        let default_provider = StoredProvider {
            id: "local-llama-swap".to_owned(),
            name: "Local llama-swap".to_owned(),
            kind: ProviderKind::OpenAiCompatible,
            base_url: "http://rafael:8080/v1".to_owned(),
            model: "gemma-everyday".to_owned(),
            api_key: None,
            system_prompt: None,
        };
        let config = ChatConfigFile {
            providers: vec![default_provider.clone()],
            settings: ChatSettings {
                active_provider_id: "qwen3-coder".to_owned(),
                theme: ThemeName::Charcoal,
            },
        };

        let (repaired, changed) = repair_config(config, &default_provider);

        assert!(!changed);
        assert_eq!(repaired.settings.active_provider_id, "qwen3-coder");
    }

    #[tokio::test]
    async fn obfuscated_store_hides_conversation_content_on_disk() {
        let data_dir = unique_test_dir("obfuscated-save");
        let store = ChatStore::new(&data_dir).with_conversation_obfuscation_key([7; 32]);
        let mut conversation = store
            .create_conversation(Some("Private title".to_owned()))
            .await
            .expect("create conversation");
        conversation.messages.push(ChatMessageRecord {
            id: "msg-1".to_owned(),
            role: ChatRole::User,
            content: "private family chat".to_owned(),
            created_at: Utc::now(),
            provider_id: None,
            metadata: None,
        });
        store
            .save_conversation(&conversation)
            .await
            .expect("save conversation");

        let path = data_dir
            .join("conversations")
            .join(format!("{}.json", conversation.id));
        let raw = tokio::fs::read_to_string(&path)
            .await
            .expect("read raw conversation file");
        assert!(raw.contains(OBFUSCATED_CONVERSATION_FORMAT));
        assert!(!raw.contains("private family chat"));
        assert!(!raw.contains("Private title"));

        let loaded = store
            .get_conversation(&conversation.id)
            .await
            .expect("load conversation")
            .expect("conversation exists");
        assert_eq!(loaded.title, "Private title");
        assert_eq!(loaded.messages[0].content, "private family chat");

        let _ = tokio::fs::remove_dir_all(data_dir).await;
    }

    #[tokio::test]
    async fn obfuscated_store_migrates_legacy_plaintext_conversation() {
        let data_dir = unique_test_dir("obfuscated-migrate");
        let plain_store = ChatStore::new(&data_dir);
        let conversation = plain_store
            .create_conversation(Some("Legacy private title".to_owned()))
            .await
            .expect("create plaintext conversation");
        let path = data_dir
            .join("conversations")
            .join(format!("{}.json", conversation.id));
        let raw_before = tokio::fs::read_to_string(&path)
            .await
            .expect("read plaintext conversation");
        assert!(raw_before.contains("Legacy private title"));

        let protected_store = ChatStore::new(&data_dir).with_conversation_obfuscation_key([11; 32]);
        let loaded = protected_store
            .get_conversation(&conversation.id)
            .await
            .expect("load legacy conversation")
            .expect("conversation exists");
        assert_eq!(loaded.title, "Legacy private title");

        let raw_after = tokio::fs::read_to_string(&path)
            .await
            .expect("read migrated conversation");
        assert!(raw_after.contains(OBFUSCATED_CONVERSATION_FORMAT));
        assert!(!raw_after.contains("Legacy private title"));

        let _ = tokio::fs::remove_dir_all(data_dir).await;
    }

    fn unique_test_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "rafael-chat-store-{name}-{}-{nanos}",
            std::process::id()
        ))
    }
}
