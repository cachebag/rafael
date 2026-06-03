use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, bail};
use chrono::Utc;
use common::SafeComponent;
use serde::{Serialize, de::DeserializeOwned};
use tokio::io::AsyncWriteExt;

use crate::types::{
    ChatConfigFile, ChatSettings, Conversation, ConversationSummary, StoredProvider, ThemeName,
};

#[derive(Debug, Clone)]
pub struct ChatStore {
    data_dir: PathBuf,
    config_path: PathBuf,
    conversations_dir: PathBuf,
}

impl ChatStore {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        let data_dir = data_dir.into();
        Self {
            config_path: data_dir.join("config.json"),
            conversations_dir: data_dir.join("conversations"),
            data_dir,
        }
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

            let conversation = read_json::<Conversation>(&path).await?;
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
            created_at: now,
            updated_at: now,
            messages: Vec::new(),
        };
        self.save_conversation(&conversation).await?;
        Ok(conversation)
    }

    pub async fn get_conversation(&self, id: &str) -> anyhow::Result<Option<Conversation>> {
        let path = self.conversation_path(id)?;
        match read_json::<Conversation>(&path).await {
            Ok(conversation) => Ok(Some(conversation)),
            Err(err) if is_not_found(&err) => Ok(None),
            Err(err) => Err(err),
        }
    }

    pub async fn save_conversation(&self, conversation: &Conversation) -> anyhow::Result<()> {
        self.ensure_dirs().await?;
        let path = self.conversation_path(&conversation.id)?;
        write_json(&path, conversation).await
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

    if !config
        .providers
        .iter()
        .any(|provider| provider.id == config.settings.active_provider_id)
    {
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

static NEXT_ID: AtomicU64 = AtomicU64::new(0);
