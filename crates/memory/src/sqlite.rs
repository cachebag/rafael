use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use chrono::{DateTime, Utc};
use common::{CommonError, SafeComponent, dedupe_preserving_order, slugify};
use serde::{Deserialize, Serialize};
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use thiserror::Error;

const SETTINGS_ROW_ID: i64 = 1;
const DEFAULT_MEMORY_BUDGET_CHARS: i64 = 4_000;
const DEFAULT_LIST_LIMIT: i64 = 200;
const MAX_LIST_LIMIT: i64 = 1_000;
const DEFAULT_RETRIEVAL_LIMIT: i64 = 12;

#[derive(Debug, Error)]
pub enum SqliteMemoryError {
    #[error("invalid memory component: {0}")]
    InvalidComponent(#[from] CommonError),
    #[error("invalid memory status: {0}")]
    InvalidStatus(String),
    #[error("invalid conversation memory mode: {0}")]
    InvalidConversationMode(String),
    #[error("memory content is required")]
    EmptyContent,
    #[error("invalid timestamp `{value}`: {source}")]
    InvalidTimestamp {
        value: String,
        #[source]
        source: chrono::ParseError,
    },
    #[error("memory path is a symlink: {0}")]
    Symlink(PathBuf),
    #[error("failed filesystem operation at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed sqlite operation for {path}: {source}")]
    Sqlite {
        path: PathBuf,
        #[source]
        source: sqlx::Error,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryStatus {
    Pending,
    Active,
    Archived,
}

impl MemoryStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Active => "active",
            Self::Archived => "archived",
        }
    }

    fn parse(value: &str) -> Result<Self, SqliteMemoryError> {
        match value {
            "pending" => Ok(Self::Pending),
            "active" => Ok(Self::Active),
            "archived" => Ok(Self::Archived),
            other => Err(SqliteMemoryError::InvalidStatus(other.to_owned())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationMemoryMode {
    Normal,
    NoMemory,
}

impl ConversationMemoryMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::NoMemory => "no_memory",
        }
    }

    fn parse(value: &str) -> Result<Self, SqliteMemoryError> {
        match value {
            "normal" => Ok(Self::Normal),
            "no_memory" => Ok(Self::NoMemory),
            other => Err(SqliteMemoryError::InvalidConversationMode(other.to_owned())),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySettings {
    pub enabled: bool,
    pub auto_capture: bool,
    pub require_approval: bool,
    pub default_conversation_mode: ConversationMemoryMode,
    pub memory_budget_chars: i64,
    pub updated_at: DateTime<Utc>,
}

impl Default for MemorySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            auto_capture: false,
            require_approval: true,
            default_conversation_mode: ConversationMemoryMode::Normal,
            memory_budget_chars: DEFAULT_MEMORY_BUDGET_CHARS,
            updated_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryRecord {
    pub id: String,
    pub kind: String,
    pub content: String,
    pub status: MemoryStatus,
    pub tags: Vec<String>,
    pub source_conversation_id: Option<String>,
    pub source_message_ids: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub confidence: Option<f64>,
    pub user_edited: bool,
}

#[derive(Debug, Clone)]
pub struct NewMemoryRecord {
    pub kind: String,
    pub content: String,
    pub status: MemoryStatus,
    pub tags: Vec<String>,
    pub source_conversation_id: Option<String>,
    pub source_message_ids: Vec<String>,
    pub confidence: Option<f64>,
}

#[derive(Debug, Default, Clone)]
pub struct MemoryRecordPatch {
    pub kind: Option<String>,
    pub content: Option<String>,
    pub status: Option<MemoryStatus>,
    pub tags: Option<Vec<String>>,
    pub source_conversation_id: Option<Option<String>>,
    pub source_message_ids: Option<Vec<String>>,
    pub confidence: Option<Option<f64>>,
}

#[derive(Debug, Default, Clone)]
pub struct MemorySettingsPatch {
    pub enabled: Option<bool>,
    pub auto_capture: Option<bool>,
    pub require_approval: Option<bool>,
    pub default_conversation_mode: Option<ConversationMemoryMode>,
    pub memory_budget_chars: Option<i64>,
}

#[derive(Debug, Default, Clone)]
pub struct MemoryListFilter {
    pub query: Option<String>,
    pub status: Option<MemoryStatus>,
    pub limit: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct SqliteMemoryStore {
    db_path: PathBuf,
    pool: SqlitePool,
}

impl SqliteMemoryStore {
    pub async fn connect(root: impl Into<PathBuf>) -> Result<Self, SqliteMemoryError> {
        let root = root.into();
        tokio::fs::create_dir_all(&root)
            .await
            .map_err(|source| SqliteMemoryError::Io {
                path: root.clone(),
                source,
            })?;
        reject_symlink(&root).await?;

        let db_path = root.join("memory.db");
        reject_symlink(&db_path).await?;
        let options = SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await
            .map_err(|source| SqliteMemoryError::Sqlite {
                path: db_path.clone(),
                source,
            })?;
        let store = Self { db_path, pool };
        store.migrate().await?;
        Ok(store)
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub async fn settings(&self) -> Result<MemorySettings, SqliteMemoryError> {
        self.ensure_default_settings().await?;
        let row = sqlx::query(
            "SELECT enabled, auto_capture, require_approval, default_conversation_mode, \
             memory_budget_chars, updated_at FROM memory_settings WHERE id = ?",
        )
        .bind(SETTINGS_ROW_ID)
        .fetch_one(&self.pool)
        .await
        .map_err(|source| self.sql_error(source))?;

        Ok(MemorySettings {
            enabled: int_bool(
                row.try_get("enabled")
                    .map_err(|source| self.sql_error(source))?,
            ),
            auto_capture: int_bool(
                row.try_get("auto_capture")
                    .map_err(|source| self.sql_error(source))?,
            ),
            require_approval: int_bool(
                row.try_get("require_approval")
                    .map_err(|source| self.sql_error(source))?,
            ),
            default_conversation_mode: ConversationMemoryMode::parse(
                row.try_get::<String, _>("default_conversation_mode")
                    .map_err(|source| self.sql_error(source))?
                    .as_str(),
            )?,
            memory_budget_chars: row
                .try_get("memory_budget_chars")
                .map_err(|source| self.sql_error(source))?,
            updated_at: parse_time(
                &row.try_get::<String, _>("updated_at")
                    .map_err(|source| self.sql_error(source))?,
            )?,
        })
    }

    pub async fn update_settings(
        &self,
        patch: MemorySettingsPatch,
    ) -> Result<MemorySettings, SqliteMemoryError> {
        let mut settings = self.settings().await?;
        if let Some(enabled) = patch.enabled {
            settings.enabled = enabled;
        }
        if let Some(auto_capture) = patch.auto_capture {
            settings.auto_capture = auto_capture;
        }
        if let Some(require_approval) = patch.require_approval {
            settings.require_approval = require_approval;
        }
        if let Some(default_conversation_mode) = patch.default_conversation_mode {
            settings.default_conversation_mode = default_conversation_mode;
        }
        if let Some(memory_budget_chars) = patch.memory_budget_chars {
            settings.memory_budget_chars = memory_budget_chars.clamp(512, 32_768);
        }
        settings.updated_at = Utc::now();

        sqlx::query(
            "INSERT INTO memory_settings \
             (id, enabled, auto_capture, require_approval, default_conversation_mode, \
              memory_budget_chars, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
              enabled = excluded.enabled, \
              auto_capture = excluded.auto_capture, \
              require_approval = excluded.require_approval, \
              default_conversation_mode = excluded.default_conversation_mode, \
              memory_budget_chars = excluded.memory_budget_chars, \
              updated_at = excluded.updated_at",
        )
        .bind(SETTINGS_ROW_ID)
        .bind(bool_int(settings.enabled))
        .bind(bool_int(settings.auto_capture))
        .bind(bool_int(settings.require_approval))
        .bind(settings.default_conversation_mode.as_str())
        .bind(settings.memory_budget_chars)
        .bind(format_time(settings.updated_at))
        .execute(&self.pool)
        .await
        .map_err(|source| self.sql_error(source))?;

        Ok(settings)
    }

    pub async fn conversation_mode(
        &self,
        conversation_id: &str,
    ) -> Result<ConversationMemoryMode, SqliteMemoryError> {
        let conversation_id = safe_id(conversation_id)?;
        if let Some(row) =
            sqlx::query("SELECT mode FROM conversation_memory_policy WHERE conversation_id = ?")
                .bind(conversation_id.as_str())
                .fetch_optional(&self.pool)
                .await
                .map_err(|source| self.sql_error(source))?
        {
            return ConversationMemoryMode::parse(
                row.try_get::<String, _>("mode")
                    .map_err(|source| self.sql_error(source))?
                    .as_str(),
            );
        }

        Ok(self.settings().await?.default_conversation_mode)
    }

    pub async fn set_conversation_mode(
        &self,
        conversation_id: &str,
        mode: ConversationMemoryMode,
    ) -> Result<ConversationMemoryMode, SqliteMemoryError> {
        let conversation_id = safe_id(conversation_id)?;
        sqlx::query(
            "INSERT INTO conversation_memory_policy (conversation_id, mode, updated_at) \
             VALUES (?, ?, ?) \
             ON CONFLICT(conversation_id) DO UPDATE SET \
              mode = excluded.mode, updated_at = excluded.updated_at",
        )
        .bind(conversation_id.as_str())
        .bind(mode.as_str())
        .bind(format_time(Utc::now()))
        .execute(&self.pool)
        .await
        .map_err(|source| self.sql_error(source))?;
        Ok(mode)
    }

    pub async fn create_memory(
        &self,
        new_record: NewMemoryRecord,
    ) -> Result<MemoryRecord, SqliteMemoryError> {
        let now = Utc::now();
        let id = new_id("mem");
        let kind = clean_kind(&new_record.kind);
        let content = clean_content(&new_record.content)?;
        let tags = clean_tags(new_record.tags);
        let source_conversation_id = clean_optional_safe(new_record.source_conversation_id)?;
        let source_message_ids = clean_safe_list(new_record.source_message_ids)?;

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|source| self.sql_error(source))?;
        sqlx::query(
            "INSERT INTO memories \
             (id, kind, content, status, source_conversation_id, created_at, updated_at, \
              last_used_at, confidence, user_edited) \
             VALUES (?, ?, ?, ?, ?, ?, ?, NULL, ?, 0)",
        )
        .bind(&id)
        .bind(&kind)
        .bind(&content)
        .bind(new_record.status.as_str())
        .bind(source_conversation_id.as_deref())
        .bind(format_time(now))
        .bind(format_time(now))
        .bind(new_record.confidence)
        .execute(&mut *tx)
        .await
        .map_err(|source| self.sql_error(source))?;

        replace_tags(&mut tx, &id, &tags, &self.db_path).await?;
        replace_source_messages(&mut tx, &id, &source_message_ids, &self.db_path).await?;
        replace_fts(&mut tx, &id, &kind, &content, &tags, &self.db_path).await?;
        tx.commit().await.map_err(|source| self.sql_error(source))?;

        Ok(MemoryRecord {
            id,
            kind,
            content,
            status: new_record.status,
            tags,
            source_conversation_id,
            source_message_ids,
            created_at: now,
            updated_at: now,
            last_used_at: None,
            confidence: new_record.confidence,
            user_edited: false,
        })
    }

    pub async fn get_memory(&self, id: &str) -> Result<Option<MemoryRecord>, SqliteMemoryError> {
        let id = safe_id(id)?;
        let row = sqlx::query(
            "SELECT id, kind, content, status, source_conversation_id, created_at, updated_at, \
             last_used_at, confidence, user_edited FROM memories WHERE id = ?",
        )
        .bind(id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|source| self.sql_error(source))?;

        match row {
            Some(row) => Ok(Some(self.record_from_row(row).await?)),
            None => Ok(None),
        }
    }

    pub async fn list_memories(
        &self,
        filter: MemoryListFilter,
    ) -> Result<Vec<MemoryRecord>, SqliteMemoryError> {
        let limit = filter
            .limit
            .unwrap_or(DEFAULT_LIST_LIMIT)
            .clamp(1, MAX_LIST_LIMIT);
        let status = filter.status.map(|status| status.as_str().to_owned());
        let query = filter.query.as_deref().and_then(fts_query);

        let rows = match (query, status) {
            (Some(query), Some(status)) => sqlx::query(
                "SELECT m.id, m.kind, m.content, m.status, m.source_conversation_id, \
                 m.created_at, m.updated_at, m.last_used_at, m.confidence, m.user_edited \
                 FROM memories m JOIN memory_fts f ON f.memory_id = m.id \
                 WHERE memory_fts MATCH ? AND m.status = ? \
                 ORDER BY bm25(memory_fts), m.updated_at DESC LIMIT ?",
            )
            .bind(query)
            .bind(status)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(|source| self.sql_error(source))?,
            (Some(query), None) => sqlx::query(
                "SELECT m.id, m.kind, m.content, m.status, m.source_conversation_id, \
                 m.created_at, m.updated_at, m.last_used_at, m.confidence, m.user_edited \
                 FROM memories m JOIN memory_fts f ON f.memory_id = m.id \
                 WHERE memory_fts MATCH ? \
                 ORDER BY bm25(memory_fts), m.updated_at DESC LIMIT ?",
            )
            .bind(query)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(|source| self.sql_error(source))?,
            (None, Some(status)) => sqlx::query(
                "SELECT id, kind, content, status, source_conversation_id, created_at, \
                 updated_at, last_used_at, confidence, user_edited \
                 FROM memories WHERE status = ? ORDER BY updated_at DESC LIMIT ?",
            )
            .bind(status)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(|source| self.sql_error(source))?,
            (None, None) => sqlx::query(
                "SELECT id, kind, content, status, source_conversation_id, created_at, \
                 updated_at, last_used_at, confidence, user_edited \
                 FROM memories ORDER BY updated_at DESC LIMIT ?",
            )
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(|source| self.sql_error(source))?,
        };

        let mut records = Vec::with_capacity(rows.len());
        for row in rows {
            records.push(self.record_from_row(row).await?);
        }
        Ok(records)
    }

    pub async fn retrieve_memories(
        &self,
        query: &str,
        budget_chars: i64,
        limit: Option<i64>,
    ) -> Result<Vec<MemoryRecord>, SqliteMemoryError> {
        let candidates = self
            .list_memories(MemoryListFilter {
                query: Some(query.to_owned()),
                status: Some(MemoryStatus::Active),
                limit: Some(limit.unwrap_or(DEFAULT_RETRIEVAL_LIMIT)),
            })
            .await?;

        let candidates = if candidates.is_empty() {
            self.list_memories(MemoryListFilter {
                query: None,
                status: Some(MemoryStatus::Active),
                limit: Some(limit.unwrap_or(DEFAULT_RETRIEVAL_LIMIT)),
            })
            .await?
        } else {
            candidates
        };

        let mut selected = Vec::new();
        let mut remaining = budget_chars.clamp(0, 64_000) as usize;
        for record in candidates {
            let cost = record.content.chars().count() + record.kind.chars().count() + 12;
            if cost > remaining && !selected.is_empty() {
                continue;
            }
            remaining = remaining.saturating_sub(cost);
            selected.push(record);
            if remaining == 0 {
                break;
            }
        }

        Ok(selected)
    }

    pub async fn update_memory(
        &self,
        id: &str,
        patch: MemoryRecordPatch,
    ) -> Result<Option<MemoryRecord>, SqliteMemoryError> {
        let Some(mut record) = self.get_memory(id).await? else {
            return Ok(None);
        };

        if let Some(kind) = patch.kind {
            record.kind = clean_kind(&kind);
        }
        if let Some(content) = patch.content {
            record.content = clean_content(&content)?;
        }
        if let Some(status) = patch.status {
            record.status = status;
        }
        if let Some(tags) = patch.tags {
            record.tags = clean_tags(tags);
        }
        if let Some(source_conversation_id) = patch.source_conversation_id {
            record.source_conversation_id = clean_optional_safe(source_conversation_id)?;
        }
        if let Some(source_message_ids) = patch.source_message_ids {
            record.source_message_ids = clean_safe_list(source_message_ids)?;
        }
        if let Some(confidence) = patch.confidence {
            record.confidence = confidence;
        }
        record.updated_at = Utc::now();
        record.user_edited = true;

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|source| self.sql_error(source))?;
        sqlx::query(
            "UPDATE memories SET kind = ?, content = ?, status = ?, source_conversation_id = ?, \
             updated_at = ?, confidence = ?, user_edited = 1 WHERE id = ?",
        )
        .bind(&record.kind)
        .bind(&record.content)
        .bind(record.status.as_str())
        .bind(record.source_conversation_id.as_deref())
        .bind(format_time(record.updated_at))
        .bind(record.confidence)
        .bind(&record.id)
        .execute(&mut *tx)
        .await
        .map_err(|source| self.sql_error(source))?;
        replace_tags(&mut tx, &record.id, &record.tags, &self.db_path).await?;
        replace_source_messages(
            &mut tx,
            &record.id,
            &record.source_message_ids,
            &self.db_path,
        )
        .await?;
        replace_fts(
            &mut tx,
            &record.id,
            &record.kind,
            &record.content,
            &record.tags,
            &self.db_path,
        )
        .await?;
        tx.commit().await.map_err(|source| self.sql_error(source))?;
        Ok(Some(record))
    }

    pub async fn delete_memory(&self, id: &str) -> Result<bool, SqliteMemoryError> {
        let id = safe_id(id)?;
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|source| self.sql_error(source))?;
        sqlx::query("DELETE FROM memory_fts WHERE memory_id = ?")
            .bind(id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(|source| self.sql_error(source))?;
        sqlx::query("DELETE FROM memory_tags WHERE memory_id = ?")
            .bind(id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(|source| self.sql_error(source))?;
        sqlx::query("DELETE FROM memory_source_messages WHERE memory_id = ?")
            .bind(id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(|source| self.sql_error(source))?;
        sqlx::query("DELETE FROM memory_usage WHERE memory_id = ?")
            .bind(id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(|source| self.sql_error(source))?;
        let deleted = sqlx::query("DELETE FROM memories WHERE id = ?")
            .bind(id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(|source| self.sql_error(source))?
            .rows_affected()
            > 0;
        tx.commit().await.map_err(|source| self.sql_error(source))?;
        Ok(deleted)
    }

    pub async fn record_usage(
        &self,
        memory_ids: &[String],
        conversation_id: &str,
        message_id: Option<&str>,
    ) -> Result<(), SqliteMemoryError> {
        if memory_ids.is_empty() {
            return Ok(());
        }
        let conversation_id = safe_id(conversation_id)?;
        let message_id = match message_id {
            Some(id) => Some(safe_id(id)?.into_string()),
            None => None,
        };
        let now = Utc::now();
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|source| self.sql_error(source))?;
        for memory_id in memory_ids {
            let memory_id = safe_id(memory_id)?;
            sqlx::query("UPDATE memories SET last_used_at = ? WHERE id = ?")
                .bind(format_time(now))
                .bind(memory_id.as_str())
                .execute(&mut *tx)
                .await
                .map_err(|source| self.sql_error(source))?;
            sqlx::query(
                "INSERT INTO memory_usage (memory_id, conversation_id, message_id, used_at) \
                 VALUES (?, ?, ?, ?)",
            )
            .bind(memory_id.as_str())
            .bind(conversation_id.as_str())
            .bind(message_id.as_deref())
            .bind(format_time(now))
            .execute(&mut *tx)
            .await
            .map_err(|source| self.sql_error(source))?;
        }
        tx.commit().await.map_err(|source| self.sql_error(source))?;
        Ok(())
    }

    async fn migrate(&self) -> Result<(), SqliteMemoryError> {
        let statements = [
            "PRAGMA journal_mode = WAL",
            "CREATE TABLE IF NOT EXISTS memory_settings (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                enabled INTEGER NOT NULL DEFAULT 0,
                auto_capture INTEGER NOT NULL DEFAULT 0,
                require_approval INTEGER NOT NULL DEFAULT 1,
                default_conversation_mode TEXT NOT NULL DEFAULT 'normal'
                    CHECK (default_conversation_mode IN ('normal', 'no_memory')),
                memory_budget_chars INTEGER NOT NULL DEFAULT 4000,
                updated_at TEXT NOT NULL
            )",
            "CREATE TABLE IF NOT EXISTS conversation_memory_policy (
                conversation_id TEXT PRIMARY KEY NOT NULL,
                mode TEXT NOT NULL CHECK (mode IN ('normal', 'no_memory')),
                updated_at TEXT NOT NULL
            )",
            "CREATE TABLE IF NOT EXISTS memories (
                id TEXT PRIMARY KEY NOT NULL,
                kind TEXT NOT NULL,
                content TEXT NOT NULL,
                status TEXT NOT NULL CHECK (status IN ('pending', 'active', 'archived')),
                source_conversation_id TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                last_used_at TEXT,
                confidence REAL,
                user_edited INTEGER NOT NULL DEFAULT 0
            )",
            "CREATE TABLE IF NOT EXISTS memory_tags (
                memory_id TEXT NOT NULL,
                tag TEXT NOT NULL,
                PRIMARY KEY (memory_id, tag)
            )",
            "CREATE TABLE IF NOT EXISTS memory_source_messages (
                memory_id TEXT NOT NULL,
                message_id TEXT NOT NULL,
                position INTEGER NOT NULL,
                PRIMARY KEY (memory_id, message_id)
            )",
            "CREATE TABLE IF NOT EXISTS memory_usage (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                memory_id TEXT NOT NULL,
                conversation_id TEXT NOT NULL,
                message_id TEXT,
                used_at TEXT NOT NULL
            )",
            "CREATE INDEX IF NOT EXISTS memories_status_updated_idx
                ON memories(status, updated_at)",
            "CREATE INDEX IF NOT EXISTS memories_source_conversation_idx
                ON memories(source_conversation_id)",
            "CREATE INDEX IF NOT EXISTS memory_tags_tag_idx
                ON memory_tags(tag)",
            "CREATE INDEX IF NOT EXISTS memory_usage_memory_idx
                ON memory_usage(memory_id, used_at)",
            "CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts
                USING fts5(memory_id UNINDEXED, kind, content, tags)",
        ];

        for statement in statements {
            sqlx::query(statement)
                .execute(&self.pool)
                .await
                .map_err(|source| self.sql_error(source))?;
        }
        self.ensure_default_settings().await
    }

    async fn ensure_default_settings(&self) -> Result<(), SqliteMemoryError> {
        let defaults = MemorySettings::default();
        sqlx::query(
            "INSERT OR IGNORE INTO memory_settings \
             (id, enabled, auto_capture, require_approval, default_conversation_mode, \
              memory_budget_chars, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(SETTINGS_ROW_ID)
        .bind(bool_int(defaults.enabled))
        .bind(bool_int(defaults.auto_capture))
        .bind(bool_int(defaults.require_approval))
        .bind(defaults.default_conversation_mode.as_str())
        .bind(defaults.memory_budget_chars)
        .bind(format_time(defaults.updated_at))
        .execute(&self.pool)
        .await
        .map_err(|source| self.sql_error(source))?;
        Ok(())
    }

    async fn record_from_row(
        &self,
        row: sqlx::sqlite::SqliteRow,
    ) -> Result<MemoryRecord, SqliteMemoryError> {
        let id: String = row.try_get("id").map_err(|source| self.sql_error(source))?;
        Ok(MemoryRecord {
            tags: self.tags_for(&id).await?,
            source_message_ids: self.source_message_ids_for(&id).await?,
            id,
            kind: row
                .try_get("kind")
                .map_err(|source| self.sql_error(source))?,
            content: row
                .try_get("content")
                .map_err(|source| self.sql_error(source))?,
            status: MemoryStatus::parse(
                row.try_get::<String, _>("status")
                    .map_err(|source| self.sql_error(source))?
                    .as_str(),
            )?,
            source_conversation_id: row
                .try_get("source_conversation_id")
                .map_err(|source| self.sql_error(source))?,
            created_at: parse_time(
                &row.try_get::<String, _>("created_at")
                    .map_err(|source| self.sql_error(source))?,
            )?,
            updated_at: parse_time(
                &row.try_get::<String, _>("updated_at")
                    .map_err(|source| self.sql_error(source))?,
            )?,
            last_used_at: parse_optional_time(
                row.try_get("last_used_at")
                    .map_err(|source| self.sql_error(source))?,
            )?,
            confidence: row
                .try_get("confidence")
                .map_err(|source| self.sql_error(source))?,
            user_edited: int_bool(
                row.try_get("user_edited")
                    .map_err(|source| self.sql_error(source))?,
            ),
        })
    }

    async fn tags_for(&self, memory_id: &str) -> Result<Vec<String>, SqliteMemoryError> {
        let rows = sqlx::query("SELECT tag FROM memory_tags WHERE memory_id = ? ORDER BY tag")
            .bind(memory_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|source| self.sql_error(source))?;
        rows.into_iter()
            .map(|row| row.try_get("tag").map_err(|source| self.sql_error(source)))
            .collect()
    }

    async fn source_message_ids_for(
        &self,
        memory_id: &str,
    ) -> Result<Vec<String>, SqliteMemoryError> {
        let rows = sqlx::query(
            "SELECT message_id FROM memory_source_messages \
             WHERE memory_id = ? ORDER BY position",
        )
        .bind(memory_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|source| self.sql_error(source))?;
        rows.into_iter()
            .map(|row| {
                row.try_get("message_id")
                    .map_err(|source| self.sql_error(source))
            })
            .collect()
    }

    fn sql_error(&self, source: sqlx::Error) -> SqliteMemoryError {
        SqliteMemoryError::Sqlite {
            path: self.db_path.clone(),
            source,
        }
    }
}

async fn replace_tags(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    memory_id: &str,
    tags: &[String],
    db_path: &Path,
) -> Result<(), SqliteMemoryError> {
    sqlx::query("DELETE FROM memory_tags WHERE memory_id = ?")
        .bind(memory_id)
        .execute(&mut **tx)
        .await
        .map_err(|source| sql_error(db_path, source))?;
    for tag in tags {
        sqlx::query("INSERT INTO memory_tags (memory_id, tag) VALUES (?, ?)")
            .bind(memory_id)
            .bind(tag)
            .execute(&mut **tx)
            .await
            .map_err(|source| sql_error(db_path, source))?;
    }
    Ok(())
}

async fn replace_source_messages(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    memory_id: &str,
    message_ids: &[String],
    db_path: &Path,
) -> Result<(), SqliteMemoryError> {
    sqlx::query("DELETE FROM memory_source_messages WHERE memory_id = ?")
        .bind(memory_id)
        .execute(&mut **tx)
        .await
        .map_err(|source| sql_error(db_path, source))?;
    for (position, message_id) in message_ids.iter().enumerate() {
        sqlx::query(
            "INSERT INTO memory_source_messages (memory_id, message_id, position) \
             VALUES (?, ?, ?)",
        )
        .bind(memory_id)
        .bind(message_id)
        .bind(position as i64)
        .execute(&mut **tx)
        .await
        .map_err(|source| sql_error(db_path, source))?;
    }
    Ok(())
}

async fn replace_fts(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    memory_id: &str,
    kind: &str,
    content: &str,
    tags: &[String],
    db_path: &Path,
) -> Result<(), SqliteMemoryError> {
    sqlx::query("DELETE FROM memory_fts WHERE memory_id = ?")
        .bind(memory_id)
        .execute(&mut **tx)
        .await
        .map_err(|source| sql_error(db_path, source))?;
    sqlx::query("INSERT INTO memory_fts (memory_id, kind, content, tags) VALUES (?, ?, ?, ?)")
        .bind(memory_id)
        .bind(kind)
        .bind(content)
        .bind(tags.join(" "))
        .execute(&mut **tx)
        .await
        .map_err(|source| sql_error(db_path, source))?;
    Ok(())
}

fn sql_error(path: &Path, source: sqlx::Error) -> SqliteMemoryError {
    SqliteMemoryError::Sqlite {
        path: path.to_path_buf(),
        source,
    }
}

fn clean_kind(value: &str) -> String {
    slugify(value.trim(), 32, "note").replace('-', "_")
}

fn clean_content(value: &str) -> Result<String, SqliteMemoryError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(SqliteMemoryError::EmptyContent);
    }
    Ok(value.to_owned())
}

fn clean_tags(tags: Vec<String>) -> Vec<String> {
    dedupe_preserving_order(
        tags.into_iter()
            .map(|tag| slugify(tag.trim(), 48, "tag"))
            .filter(|tag| !tag.is_empty()),
    )
}

fn clean_optional_safe(value: Option<String>) -> Result<Option<String>, SqliteMemoryError> {
    value
        .map(|value| safe_id(&value).map(SafeComponent::into_string))
        .transpose()
}

fn clean_safe_list(values: Vec<String>) -> Result<Vec<String>, SqliteMemoryError> {
    values
        .into_iter()
        .map(|value| safe_id(&value).map(SafeComponent::into_string))
        .collect::<Result<Vec<_>, _>>()
        .map(dedupe_preserving_order)
}

fn safe_id(value: &str) -> Result<SafeComponent, SqliteMemoryError> {
    SafeComponent::parse(value.trim().to_owned()).map_err(SqliteMemoryError::from)
}

fn fts_query(raw: &str) -> Option<String> {
    let mut terms = Vec::new();
    let mut current = String::new();

    for ch in raw.chars().flat_map(char::to_lowercase) {
        if ch.is_alphanumeric() {
            current.push(ch);
        } else if !current.is_empty() {
            if current.chars().count() >= 2 {
                terms.push(format!("\"{}\"", current.replace('"', "\"\"")));
            }
            current.clear();
        }
        if terms.len() >= 16 {
            break;
        }
    }
    if !current.is_empty() && current.chars().count() >= 2 && terms.len() < 16 {
        terms.push(format!("\"{}\"", current.replace('"', "\"\"")));
    }

    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" OR "))
    }
}

fn parse_time(value: &str) -> Result<DateTime<Utc>, SqliteMemoryError> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|source| SqliteMemoryError::InvalidTimestamp {
            value: value.to_owned(),
            source,
        })
}

fn parse_optional_time(value: Option<String>) -> Result<Option<DateTime<Utc>>, SqliteMemoryError> {
    value.map(|value| parse_time(&value)).transpose()
}

fn format_time(value: DateTime<Utc>) -> String {
    value.to_rfc3339()
}

fn bool_int(value: bool) -> i64 {
    if value { 1 } else { 0 }
}

fn int_bool(value: i64) -> bool {
    value != 0
}

async fn reject_symlink(path: &Path) -> Result<(), SqliteMemoryError> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(SqliteMemoryError::Symlink(path.to_path_buf()))
        }
        Ok(_) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(SqliteMemoryError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn new_id(prefix: &str) -> String {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let sequence = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{prefix}-{nanos}-{sequence}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sqlite_memory_round_trips_records() {
        let store = SqliteMemoryStore::connect(unique_test_dir("round-trip"))
            .await
            .expect("connect store");
        let created = store
            .create_memory(NewMemoryRecord {
                kind: "Preference".to_owned(),
                content: "Prefers Rust implementations.".to_owned(),
                status: MemoryStatus::Active,
                tags: vec!["Rust".to_owned(), "rust".to_owned()],
                source_conversation_id: Some("conv-1".to_owned()),
                source_message_ids: vec!["msg-1".to_owned()],
                confidence: Some(0.9),
            })
            .await
            .expect("create memory");

        assert_eq!(created.kind, "preference");
        assert_eq!(created.tags, vec!["rust"]);

        let found = store
            .list_memories(MemoryListFilter {
                query: Some("rust implementation".to_owned()),
                status: Some(MemoryStatus::Active),
                limit: None,
            })
            .await
            .expect("search memories");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, created.id);
    }

    #[tokio::test]
    async fn conversation_mode_defaults_to_settings() {
        let store = SqliteMemoryStore::connect(unique_test_dir("mode-default"))
            .await
            .expect("connect store");
        store
            .update_settings(MemorySettingsPatch {
                default_conversation_mode: Some(ConversationMemoryMode::NoMemory),
                ..MemorySettingsPatch::default()
            })
            .await
            .expect("update settings");

        assert_eq!(
            store
                .conversation_mode("conv-1")
                .await
                .expect("conversation mode"),
            ConversationMemoryMode::NoMemory
        );
    }

    fn unique_test_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "rafael-memory-{name}-{}-{nanos}",
            std::process::id()
        ))
    }
}
