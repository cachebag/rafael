use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use chrono::{DateTime, Utc};
use common::{SafeComponent, dedupe_preserving_order};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use thiserror::Error;
use tokio::io::AsyncWriteExt;

#[derive(Debug, Error)]
pub enum MemoryError {
    #[error("invalid memory component: {0}")]
    InvalidComponent(#[from] common::CommonError),
    #[error("memory path escaped root")]
    EscapedRoot,
    #[error("memory path is a symlink: {0}")]
    Symlink(PathBuf),
    #[error("failed filesystem operation at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed json operation for {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryAddress {
    pub namespace: SafeComponent,
    pub key: SafeComponent,
}

impl MemoryAddress {
    pub fn new(namespace: impl Into<String>, key: impl Into<String>) -> Result<Self, MemoryError> {
        Ok(Self {
            namespace: SafeComponent::parse(namespace.into())?,
            key: SafeComponent::parse(key.into())?,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewMemoryEntry {
    pub address: MemoryAddress,
    pub value: serde_json::Value,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub source: Option<String>,
}

impl NewMemoryEntry {
    pub fn json<T>(
        namespace: impl Into<String>,
        key: impl Into<String>,
        value: T,
    ) -> Result<Self, MemoryError>
    where
        T: Serialize,
    {
        let value = serde_json::to_value(value).map_err(|source| MemoryError::Json {
            path: PathBuf::from("<memory-entry>"),
            source,
        })?;
        Ok(Self {
            address: MemoryAddress::new(namespace, key)?,
            value,
            tags: Vec::new(),
            source: None,
        })
    }

    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = dedupe_preserving_order(tags);
        self
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub address: MemoryAddress,
    pub value: serde_json::Value,
    pub tags: Vec<String>,
    pub source: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl MemoryEntry {
    pub fn decode<T>(&self) -> Result<T, serde_json::Error>
    where
        T: DeserializeOwned,
    {
        serde_json::from_value(self.value.clone())
    }
}

#[derive(Debug, Clone)]
pub struct JsonMemoryStore {
    root: PathBuf,
}

impl JsonMemoryStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub async fn put(&self, entry: NewMemoryEntry) -> Result<MemoryEntry, MemoryError> {
        self.ensure_root().await?;
        let path = self.entry_path(&entry.address)?;
        let namespace_dir = path
            .parent()
            .map(Path::to_path_buf)
            .ok_or(MemoryError::EscapedRoot)?;
        self.ensure_namespace_dir(&namespace_dir).await?;
        reject_symlink(&path).await?;

        let now = Utc::now();
        let created_at = match self.get(&entry.address).await? {
            Some(existing) => existing.created_at,
            None => now,
        };
        let stored = MemoryEntry {
            address: entry.address,
            value: entry.value,
            tags: dedupe_preserving_order(entry.tags),
            source: entry.source,
            created_at,
            updated_at: now,
        };
        let bytes = serde_json::to_vec_pretty(&stored).map_err(|source| MemoryError::Json {
            path: path.clone(),
            source,
        })?;
        atomic_write(&path, &bytes).await?;
        Ok(stored)
    }

    pub async fn get(&self, address: &MemoryAddress) -> Result<Option<MemoryEntry>, MemoryError> {
        let path = self.entry_path(address)?;
        reject_symlink(&path).await?;
        match tokio::fs::read(&path).await {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|source| MemoryError::Json { path, source }),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(MemoryError::Io { path, source }),
        }
    }

    pub async fn list(
        &self,
        namespace: impl Into<String>,
    ) -> Result<Vec<MemoryEntry>, MemoryError> {
        let namespace = SafeComponent::parse(namespace.into())?;
        let dir = self.root.join(namespace.as_str());
        reject_symlink(&dir).await?;
        let mut entries = Vec::new();
        let mut read_dir = match tokio::fs::read_dir(&dir).await {
            Ok(read_dir) => read_dir,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(entries),
            Err(source) => return Err(MemoryError::Io { path: dir, source }),
        };

        while let Some(entry) = read_dir
            .next_entry()
            .await
            .map_err(|source| MemoryError::Io {
                path: self.root.clone(),
                source,
            })?
        {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            if entry
                .file_type()
                .await
                .map_err(|source| MemoryError::Io {
                    path: path.clone(),
                    source,
                })?
                .is_file()
            {
                let bytes = tokio::fs::read(&path)
                    .await
                    .map_err(|source| MemoryError::Io {
                        path: path.clone(),
                        source,
                    })?;
                entries.push(serde_json::from_slice(&bytes).map_err(|source| {
                    MemoryError::Json {
                        path: path.clone(),
                        source,
                    }
                })?);
            }
        }

        entries.sort_by(|left: &MemoryEntry, right: &MemoryEntry| {
            left.address.key.as_str().cmp(right.address.key.as_str())
        });
        Ok(entries)
    }

    pub async fn delete(&self, address: &MemoryAddress) -> Result<bool, MemoryError> {
        let path = self.entry_path(address)?;
        reject_symlink(&path).await?;
        match tokio::fs::remove_file(&path).await {
            Ok(()) => Ok(true),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(source) => Err(MemoryError::Io { path, source }),
        }
    }

    fn entry_path(&self, address: &MemoryAddress) -> Result<PathBuf, MemoryError> {
        let path = self
            .root
            .join(address.namespace.as_str())
            .join(format!("{}.json", address.key.as_str()));
        if !path.starts_with(&self.root) {
            return Err(MemoryError::EscapedRoot);
        }
        Ok(path)
    }

    async fn ensure_root(&self) -> Result<(), MemoryError> {
        tokio::fs::create_dir_all(&self.root)
            .await
            .map_err(|source| MemoryError::Io {
                path: self.root.clone(),
                source,
            })?;
        reject_symlink(&self.root).await
    }

    async fn ensure_namespace_dir(&self, dir: &Path) -> Result<(), MemoryError> {
        if !dir.starts_with(&self.root) {
            return Err(MemoryError::EscapedRoot);
        }
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|source| MemoryError::Io {
                path: dir.to_path_buf(),
                source,
            })?;
        reject_symlink(dir).await
    }
}

async fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), MemoryError> {
    let tmp_path = path.with_extension(format!("tmp-{}-{}", std::process::id(), now_nanos()));
    let mut file = tokio::fs::File::create(&tmp_path)
        .await
        .map_err(|source| MemoryError::Io {
            path: tmp_path.clone(),
            source,
        })?;
    file.write_all(bytes)
        .await
        .map_err(|source| MemoryError::Io {
            path: tmp_path.clone(),
            source,
        })?;
    file.flush().await.map_err(|source| MemoryError::Io {
        path: tmp_path.clone(),
        source,
    })?;
    tokio::fs::rename(&tmp_path, path)
        .await
        .map_err(|source| MemoryError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    Ok(())
}

async fn reject_symlink(path: &Path) -> Result<(), MemoryError> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(MemoryError::Symlink(path.to_path_buf()))
        }
        Ok(_) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(MemoryError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_rejects_path_components() {
        assert!(MemoryAddress::new("global", "facts").is_ok());
        assert!(MemoryAddress::new("../global", "facts").is_err());
        assert!(MemoryAddress::new("global", "a/b").is_err());
    }
}
