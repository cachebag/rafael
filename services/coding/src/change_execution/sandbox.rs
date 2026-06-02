use std::path::{Component, Path, PathBuf};

use anyhow::Context;

#[derive(Debug)]
pub(super) struct RepoSandbox {
    pub(super) checkout_path: PathBuf,
    canonical_checkout_path: PathBuf,
}

impl RepoSandbox {
    pub(super) async fn new(checkout_path: &Path) -> anyhow::Result<Self> {
        let canonical_checkout_path = tokio::fs::canonicalize(checkout_path)
            .await
            .with_context(|| format!("failed to canonicalize {}", checkout_path.display()))?;

        Ok(Self {
            checkout_path: checkout_path.to_path_buf(),
            canonical_checkout_path,
        })
    }

    pub(super) async fn existing_file(&self, path: &str) -> Result<PathBuf, PathRejection> {
        let relative_path = sanitize_repo_path(path).map_err(PathRejection::Unsafe)?;
        let absolute_path = self.checkout_path.join(&relative_path);
        let canonical_path = self
            .canonical_existing_path(&absolute_path)
            .await
            .map_err(path_rejection_for_canonical_error)?;

        let metadata = tokio::fs::metadata(&canonical_path)
            .await
            .map_err(|err| PathRejection::NotFound(format!("failed to stat {path}: {err}")))?;
        if !metadata.is_file() {
            return Err(PathRejection::NotFound(format!(
                "path is not a file: {path}"
            )));
        }

        Ok(canonical_path)
    }

    pub(super) async fn optional_existing_path(
        &self,
        path: Option<&str>,
    ) -> Result<SearchScope, PathRejection> {
        let Some(path) = path else {
            return Ok(SearchScope::All);
        };
        let relative_path = sanitize_repo_path(path).map_err(PathRejection::Unsafe)?;
        let absolute_path = self.checkout_path.join(&relative_path);
        self.canonical_existing_path(&absolute_path)
            .await
            .map_err(path_rejection_for_canonical_error)?;

        Ok(SearchScope::Path(path_to_repo_string(&relative_path)))
    }

    pub(super) async fn write_target(
        &self,
        path: &str,
    ) -> Result<(PathBuf, PathBuf), PathRejection> {
        let relative_path = sanitize_repo_path(path).map_err(PathRejection::Unsafe)?;
        let absolute_path = self.checkout_path.join(&relative_path);
        let Some(parent) = absolute_path.parent() else {
            return Err(PathRejection::Unsafe(format!(
                "write target has no parent directory: {path}"
            )));
        };
        let Some(file_name) = absolute_path.file_name() else {
            return Err(PathRejection::Unsafe(format!(
                "write target has no file name: {path}"
            )));
        };

        let canonical_parent = self
            .canonical_existing_path(parent)
            .await
            .map_err(path_rejection_for_canonical_error)?;
        if !canonical_parent.is_dir() {
            return Err(PathRejection::NotFound(format!(
                "write target parent is not a directory: {}",
                parent.display()
            )));
        }

        if let Ok(metadata) = tokio::fs::symlink_metadata(&absolute_path).await {
            if metadata.file_type().is_symlink() {
                return Err(PathRejection::Unsafe(
                    "write targets must not be symlinks".to_owned(),
                ));
            }
            let canonical_target = self
                .canonical_existing_path(&absolute_path)
                .await
                .map_err(path_rejection_for_canonical_error)?;
            return Ok((relative_path, canonical_target));
        }

        Ok((relative_path, canonical_parent.join(Path::new(file_name))))
    }

    async fn canonical_existing_path(&self, path: &Path) -> Result<PathBuf, String> {
        let canonical_path = tokio::fs::canonicalize(path)
            .await
            .map_err(|err| format!("path does not exist or cannot be accessed: {err}"))?;
        if !canonical_path.starts_with(&self.canonical_checkout_path) {
            return Err(PATH_ESCAPES_REPOSITORY_CHECKOUT.to_owned());
        }

        Ok(canonical_path)
    }
}

#[derive(Debug)]
pub(super) enum PathRejection {
    Unsafe(String),
    NotFound(String),
}

fn path_rejection_for_canonical_error(message: String) -> PathRejection {
    if message == PATH_ESCAPES_REPOSITORY_CHECKOUT {
        PathRejection::Unsafe(message)
    } else {
        PathRejection::NotFound(message)
    }
}

#[derive(Debug)]
pub(super) enum SearchScope {
    All,
    Path(String),
}

impl SearchScope {
    pub(super) fn contains(&self, path: &str) -> bool {
        match self {
            SearchScope::All => true,
            SearchScope::Path(scope) => path == scope || path.starts_with(&format!("{scope}/")),
        }
    }
}

pub(super) fn sanitize_repo_path(path: &str) -> Result<PathBuf, String> {
    if path.is_empty() || path.contains('\0') {
        return Err("path must be non-empty and must not contain NUL bytes".to_owned());
    }

    let path = Path::new(path);
    if path.is_absolute() {
        return Err("absolute paths are not allowed".to_owned());
    }

    let mut clean_path = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                if part == ".git" {
                    return Err("paths under .git are not allowed".to_owned());
                }
                clean_path.push(part);
            }
            Component::CurDir => {}
            Component::ParentDir => return Err("parent traversal is not allowed".to_owned()),
            Component::RootDir | Component::Prefix(_) => {
                return Err("absolute paths are not allowed".to_owned());
            }
        }
    }

    if clean_path.as_os_str().is_empty() {
        return Err("path must name a repository file".to_owned());
    }

    let clean = path_to_repo_string(&clean_path);
    if is_sensitive_path(&clean) {
        return Err("known secret-bearing paths are not allowed".to_owned());
    }

    Ok(clean_path)
}

pub(super) fn path_to_repo_string(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

pub(super) fn is_sensitive_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let file_name = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();

    if is_allowed_env_example(&file_name) {
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

fn is_allowed_env_example(file_name: &str) -> bool {
    matches!(file_name, ".env.example" | ".env.sample" | ".env.template")
}

const PATH_ESCAPES_REPOSITORY_CHECKOUT: &str = "path escapes repository checkout";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_paths() {
        assert!(sanitize_repo_path("../Cargo.toml").is_err());
        assert!(sanitize_repo_path(".git/config").is_err());
        assert!(sanitize_repo_path("/tmp/file").is_err());
        assert!(sanitize_repo_path(".env").is_err());
        assert!(sanitize_repo_path(".env.example").is_ok());
        assert!(sanitize_repo_path("services/coding/.env.example").is_ok());
        assert!(sanitize_repo_path("services/coding/.env.local").is_err());
        assert!(sanitize_repo_path("src/main.rs").is_ok());
    }
}
