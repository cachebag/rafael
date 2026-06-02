use std::{
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Debug, Error)]
pub enum FilesystemError {
    #[error("unsafe path `{path}`: {reason}")]
    UnsafePath { path: String, reason: String },
    #[error("path escapes filesystem root: {0}")]
    EscapesRoot(PathBuf),
    #[error("path does not exist: {0}")]
    NotFound(PathBuf),
    #[error("path is not a file: {0}")]
    NotFile(PathBuf),
    #[error("path is not a directory: {0}")]
    NotDirectory(PathBuf),
    #[error("path is a symlink: {0}")]
    Symlink(PathBuf),
    #[error("content is too large: {bytes} bytes exceeds {max_bytes} bytes")]
    ContentTooLarge { bytes: usize, max_bytes: usize },
    #[error("old text was not found in {0}")]
    OldTextNotFound(String),
    #[error("old text appears more than once in {0}")]
    OldTextNotUnique(String),
    #[error("failed filesystem operation at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Clone)]
pub struct RootedFilesystem {
    root: PathBuf,
}

impl RootedFilesystem {
    pub async fn open(root: impl AsRef<Path>) -> Result<Self, FilesystemError> {
        let root = root.as_ref();
        reject_symlink(root).await?;
        let canonical =
            tokio::fs::canonicalize(root)
                .await
                .map_err(|source| FilesystemError::Io {
                    path: root.to_path_buf(),
                    source,
                })?;
        Ok(Self { root: canonical })
    }

    pub async fn open_or_create(root: impl AsRef<Path>) -> Result<Self, FilesystemError> {
        let root = root.as_ref();
        tokio::fs::create_dir_all(root)
            .await
            .map_err(|source| FilesystemError::Io {
                path: root.to_path_buf(),
                source,
            })?;
        Self::open(root).await
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub async fn read_text(
        &self,
        path: &str,
        options: ReadOptions,
    ) -> Result<FileRead, FilesystemError> {
        let absolute = self.resolve_existing_file(path).await?;
        let file =
            tokio::fs::File::open(&absolute)
                .await
                .map_err(|source| FilesystemError::Io {
                    path: absolute.clone(),
                    source,
                })?;
        let mut bytes = Vec::new();
        file.take(options.max_bytes as u64 + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|source| FilesystemError::Io {
                path: absolute.clone(),
                source,
            })?;
        let truncated = bytes.len() > options.max_bytes;
        if truncated {
            bytes.truncate(options.max_bytes);
        }
        let content = String::from_utf8_lossy(&bytes).into_owned();

        Ok(FileRead {
            path: path_to_string(&self.relative_to_root(&absolute)?),
            content,
            bytes: bytes.len(),
            truncated,
        })
    }

    pub async fn read_text_range(
        &self,
        path: &str,
        start_line: usize,
        end_line: usize,
        options: ReadOptions,
    ) -> Result<FileRead, FilesystemError> {
        if start_line == 0 || end_line < start_line {
            return Err(FilesystemError::UnsafePath {
                path: path.to_owned(),
                reason: "line ranges are one-based and end_line must be >= start_line".to_owned(),
            });
        }

        let read = self.read_text(path, options).await?;
        let content = read
            .content
            .lines()
            .enumerate()
            .filter_map(|(index, line)| {
                let line_number = index + 1;
                (line_number >= start_line && line_number <= end_line)
                    .then(|| format!("{line_number}: {line}"))
            })
            .collect::<Vec<_>>()
            .join("\n");

        Ok(FileRead {
            bytes: content.len(),
            content,
            ..read
        })
    }

    pub async fn write_text(
        &self,
        path: &str,
        content: &str,
        options: WriteOptions,
    ) -> Result<FileWrite, FilesystemError> {
        if content.len() > options.max_bytes {
            return Err(FilesystemError::ContentTooLarge {
                bytes: content.len(),
                max_bytes: options.max_bytes,
            });
        }

        let absolute = self.write_target(path, options.create_parent_dirs).await?;
        atomic_write(&absolute, content.as_bytes()).await?;
        Ok(FileWrite {
            path: path_to_string(&self.relative_to_root(&absolute)?),
            bytes: content.len(),
        })
    }

    pub async fn edit_text(
        &self,
        path: &str,
        old_text: &str,
        new_text: &str,
        options: WriteOptions,
    ) -> Result<FileWrite, FilesystemError> {
        let read = self
            .read_text(
                path,
                ReadOptions {
                    max_bytes: options.max_bytes,
                },
            )
            .await?;
        if read.truncated {
            return Err(FilesystemError::ContentTooLarge {
                bytes: read.bytes,
                max_bytes: options.max_bytes,
            });
        }

        let matches = read.content.matches(old_text).count();
        if matches == 0 {
            return Err(FilesystemError::OldTextNotFound(path.to_owned()));
        }
        if matches > 1 {
            return Err(FilesystemError::OldTextNotUnique(path.to_owned()));
        }

        let edited = read.content.replacen(old_text, new_text, 1);
        self.write_text(path, &edited, options).await
    }

    pub async fn search_text(
        &self,
        query: &str,
        scope: Option<&str>,
        options: SearchOptions,
    ) -> Result<Vec<SearchMatch>, FilesystemError> {
        if query.is_empty() {
            return Err(FilesystemError::UnsafePath {
                path: String::new(),
                reason: "search query must be non-empty".to_owned(),
            });
        }

        let base = match scope {
            Some(scope) => self.resolve_existing_path(scope).await?,
            None => self.root.clone(),
        };
        let mut matches = Vec::new();
        let mut stack = vec![base];
        let needle = if options.case_sensitive {
            query.to_owned()
        } else {
            query.to_ascii_lowercase()
        };

        while let Some(path) = stack.pop() {
            if matches.len() >= options.max_matches {
                break;
            }
            let metadata =
                tokio::fs::symlink_metadata(&path)
                    .await
                    .map_err(|source| FilesystemError::Io {
                        path: path.clone(),
                        source,
                    })?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                let mut read_dir =
                    tokio::fs::read_dir(&path)
                        .await
                        .map_err(|source| FilesystemError::Io {
                            path: path.clone(),
                            source,
                        })?;
                while let Some(entry) =
                    read_dir
                        .next_entry()
                        .await
                        .map_err(|source| FilesystemError::Io {
                            path: path.clone(),
                            source,
                        })?
                {
                    stack.push(entry.path());
                }
                continue;
            }
            if !metadata.is_file() || metadata.len() > options.max_file_bytes as u64 {
                continue;
            }

            let relative = path_to_string(&self.relative_to_root(&path)?);
            let read = self
                .read_text(
                    &relative,
                    ReadOptions {
                        max_bytes: options.max_file_bytes,
                    },
                )
                .await?;
            for (index, line) in read.content.lines().enumerate() {
                let haystack = if options.case_sensitive {
                    line.to_owned()
                } else {
                    line.to_ascii_lowercase()
                };
                if haystack.contains(&needle) {
                    matches.push(SearchMatch {
                        path: relative.clone(),
                        line: index + 1,
                        text: line.to_owned(),
                    });
                    if matches.len() >= options.max_matches {
                        break;
                    }
                }
            }
        }

        matches.sort_by(|left, right| left.path.cmp(&right.path).then(left.line.cmp(&right.line)));
        Ok(matches)
    }

    pub async fn list_dir(
        &self,
        path: Option<&str>,
        max_entries: usize,
    ) -> Result<Vec<DirectoryEntry>, FilesystemError> {
        let absolute = match path {
            Some(path) => self.resolve_existing_path(path).await?,
            None => self.root.clone(),
        };
        let metadata =
            tokio::fs::metadata(&absolute)
                .await
                .map_err(|source| FilesystemError::Io {
                    path: absolute.clone(),
                    source,
                })?;
        if !metadata.is_dir() {
            return Err(FilesystemError::NotDirectory(absolute));
        }

        let mut entries = Vec::new();
        let mut read_dir =
            tokio::fs::read_dir(&absolute)
                .await
                .map_err(|source| FilesystemError::Io {
                    path: absolute.clone(),
                    source,
                })?;
        while let Some(entry) =
            read_dir
                .next_entry()
                .await
                .map_err(|source| FilesystemError::Io {
                    path: absolute.clone(),
                    source,
                })?
        {
            if entries.len() >= max_entries {
                break;
            }
            let path = entry.path();
            let file_type = entry
                .file_type()
                .await
                .map_err(|source| FilesystemError::Io {
                    path: path.clone(),
                    source,
                })?;
            let kind = if file_type.is_dir() {
                DirectoryEntryKind::Directory
            } else if file_type.is_file() {
                DirectoryEntryKind::File
            } else if file_type.is_symlink() {
                DirectoryEntryKind::Symlink
            } else {
                DirectoryEntryKind::Other
            };
            entries.push(DirectoryEntry {
                path: path_to_string(&self.relative_to_root(&path)?),
                kind,
            });
        }
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(entries)
    }

    async fn resolve_existing_file(&self, path: &str) -> Result<PathBuf, FilesystemError> {
        let absolute = self.resolve_existing_path(path).await?;
        let metadata =
            tokio::fs::metadata(&absolute)
                .await
                .map_err(|source| FilesystemError::Io {
                    path: absolute.clone(),
                    source,
                })?;
        if !metadata.is_file() {
            return Err(FilesystemError::NotFile(absolute));
        }
        Ok(absolute)
    }

    async fn resolve_existing_path(&self, path: &str) -> Result<PathBuf, FilesystemError> {
        let relative = sanitize_relative_path(path)?;
        let absolute = self.root.join(relative);
        reject_symlink(&absolute).await?;
        let canonical = tokio::fs::canonicalize(&absolute)
            .await
            .map_err(|source| match source.kind() {
                std::io::ErrorKind::NotFound => FilesystemError::NotFound(absolute.clone()),
                _ => FilesystemError::Io {
                    path: absolute.clone(),
                    source,
                },
            })?;
        if !canonical.starts_with(&self.root) {
            return Err(FilesystemError::EscapesRoot(canonical));
        }
        Ok(canonical)
    }

    async fn write_target(
        &self,
        path: &str,
        create_parent_dirs: bool,
    ) -> Result<PathBuf, FilesystemError> {
        let relative = sanitize_relative_path(path)?;
        let absolute = self.root.join(relative);
        let parent = absolute
            .parent()
            .ok_or_else(|| FilesystemError::UnsafePath {
                path: path.to_owned(),
                reason: "write target has no parent directory".to_owned(),
            })?;
        if create_parent_dirs {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|source| FilesystemError::Io {
                    path: parent.to_path_buf(),
                    source,
                })?;
        }
        reject_symlink(parent).await?;
        let canonical_parent =
            tokio::fs::canonicalize(parent)
                .await
                .map_err(|source| FilesystemError::Io {
                    path: parent.to_path_buf(),
                    source,
                })?;
        if !canonical_parent.starts_with(&self.root) {
            return Err(FilesystemError::EscapesRoot(canonical_parent));
        }

        if let Ok(metadata) = tokio::fs::symlink_metadata(&absolute).await {
            if metadata.file_type().is_symlink() {
                return Err(FilesystemError::Symlink(absolute));
            }
            let canonical =
                tokio::fs::canonicalize(&absolute)
                    .await
                    .map_err(|source| FilesystemError::Io {
                        path: absolute.clone(),
                        source,
                    })?;
            if !canonical.starts_with(&self.root) {
                return Err(FilesystemError::EscapesRoot(canonical));
            }
            return Ok(canonical);
        }

        let file_name = absolute
            .file_name()
            .ok_or_else(|| FilesystemError::UnsafePath {
                path: path.to_owned(),
                reason: "write target has no file name".to_owned(),
            })?;
        Ok(canonical_parent.join(file_name))
    }

    fn relative_to_root(&self, absolute: &Path) -> Result<PathBuf, FilesystemError> {
        absolute
            .strip_prefix(&self.root)
            .map(Path::to_path_buf)
            .map_err(|_| FilesystemError::EscapesRoot(absolute.to_path_buf()))
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ReadOptions {
    pub max_bytes: usize,
}

impl Default for ReadOptions {
    fn default() -> Self {
        Self {
            max_bytes: 128 * 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct WriteOptions {
    pub max_bytes: usize,
    pub create_parent_dirs: bool,
}

impl Default for WriteOptions {
    fn default() -> Self {
        Self {
            max_bytes: 256 * 1024,
            create_parent_dirs: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SearchOptions {
    pub max_matches: usize,
    pub max_file_bytes: usize,
    pub case_sensitive: bool,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            max_matches: 100,
            max_file_bytes: 128 * 1024,
            case_sensitive: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRead {
    pub path: String,
    pub content: String,
    pub bytes: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileWrite {
    pub path: String,
    pub bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchMatch {
    pub path: String,
    pub line: usize,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectoryEntry {
    pub path: String,
    pub kind: DirectoryEntryKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectoryEntryKind {
    File,
    Directory,
    Symlink,
    Other,
}

pub fn sanitize_relative_path(path: &str) -> Result<PathBuf, FilesystemError> {
    if path.is_empty() || path.contains('\0') {
        return Err(FilesystemError::UnsafePath {
            path: path.to_owned(),
            reason: "path must be non-empty and must not contain NUL bytes".to_owned(),
        });
    }

    let path_value = Path::new(path);
    if path_value.is_absolute() {
        return Err(FilesystemError::UnsafePath {
            path: path.to_owned(),
            reason: "absolute paths are not allowed".to_owned(),
        });
    }

    let mut clean = PathBuf::new();
    for component in path_value.components() {
        match component {
            Component::Normal(part) => {
                if part == ".git" {
                    return Err(FilesystemError::UnsafePath {
                        path: path.to_owned(),
                        reason: "paths under .git are not allowed".to_owned(),
                    });
                }
                clean.push(part);
            }
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(FilesystemError::UnsafePath {
                    path: path.to_owned(),
                    reason: "parent traversal is not allowed".to_owned(),
                });
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(FilesystemError::UnsafePath {
                    path: path.to_owned(),
                    reason: "absolute paths are not allowed".to_owned(),
                });
            }
        }
    }

    if clean.as_os_str().is_empty() {
        return Err(FilesystemError::UnsafePath {
            path: path.to_owned(),
            reason: "path must name a file or directory".to_owned(),
        });
    }
    let clean_string = path_to_string(&clean);
    if is_sensitive_path(&clean_string) {
        return Err(FilesystemError::UnsafePath {
            path: path.to_owned(),
            reason: "known secret-bearing paths are not allowed".to_owned(),
        });
    }

    Ok(clean)
}

pub fn is_sensitive_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let file_name = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();

    if matches!(
        file_name.as_str(),
        ".env.example" | ".env.sample" | ".env.template"
    ) {
        return false;
    }

    lower == ".env"
        || lower.starts_with(".env.")
        || lower.contains("/.env")
        || lower.ends_with(".pem")
        || lower.ends_with(".key")
        || lower.contains("secret")
        || lower.contains("token")
        || lower.contains("credential")
        || matches!(
            file_name.as_str(),
            "id_rsa" | "id_ed25519" | "credentials" | "netrc" | ".netrc"
        )
}

fn path_to_string(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

async fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), FilesystemError> {
    let tmp_path = path.with_extension(format!("tmp-{}-{}", std::process::id(), now_nanos()));
    let mut file =
        tokio::fs::File::create(&tmp_path)
            .await
            .map_err(|source| FilesystemError::Io {
                path: tmp_path.clone(),
                source,
            })?;
    file.write_all(bytes)
        .await
        .map_err(|source| FilesystemError::Io {
            path: tmp_path.clone(),
            source,
        })?;
    file.flush().await.map_err(|source| FilesystemError::Io {
        path: tmp_path.clone(),
        source,
    })?;
    tokio::fs::rename(&tmp_path, path)
        .await
        .map_err(|source| FilesystemError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    Ok(())
}

async fn reject_symlink(path: &Path) -> Result<(), FilesystemError> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(FilesystemError::Symlink(path.to_path_buf()))
        }
        Ok(_) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(FilesystemError::Io {
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
    fn rejects_unsafe_paths() {
        assert!(sanitize_relative_path("../Cargo.toml").is_err());
        assert!(sanitize_relative_path(".git/config").is_err());
        assert!(sanitize_relative_path("/tmp/file").is_err());
        assert!(sanitize_relative_path(".env").is_err());
        assert!(sanitize_relative_path(".env.example").is_ok());
        assert!(sanitize_relative_path("src/main.rs").is_ok());
    }
}
