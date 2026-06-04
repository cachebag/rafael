use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, bail};
use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};
use chrono::{Duration, Utc};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use rand_core::RngCore;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::io::AsyncWriteExt;

use crate::store::ChatStore;
use crate::types::PublicUser;

const ALLOWED_FIRST_NAMES: &[(&str, &str)] =
    &[("akrm", "Akrm"), ("nowar", "Nowar"), ("sofia", "Sofia")];
const MIN_PASSWORD_LEN: usize = 8;

#[derive(Debug, Clone)]
pub struct AuthStore {
    data_dir: PathBuf,
    users_path: PathBuf,
    token_secret: Arc<String>,
    token_ttl: Duration,
}

impl AuthStore {
    pub async fn new(data_dir: impl Into<PathBuf>, token_ttl: Duration) -> anyhow::Result<Self> {
        let data_dir = data_dir.into();
        let token_secret = load_or_create_secret(&data_dir.join("auth_secret")).await?;

        Ok(Self {
            users_path: data_dir.join("users.json"),
            data_dir,
            token_secret: Arc::new(token_secret),
            token_ttl,
        })
    }

    pub async fn register(
        &self,
        username: &str,
        first_name: &str,
        password: &str,
    ) -> Result<AuthSession, AuthFailure> {
        let username = normalize_username(username);
        if username.is_empty() {
            return Err(AuthFailure::InvalidUsername);
        }
        let allowed = allowed_first_name(first_name).ok_or(AuthFailure::NotAllowed)?;
        validate_password(password)?;
        let mut users = self.load_users().await?;
        if users
            .users
            .iter()
            .any(|user| normalize_username(&user.username) == username)
        {
            return Err(AuthFailure::AlreadyRegistered);
        }

        let password_hash = hash_password(password)?;
        let user = StoredUser {
            id: unique_user_id(&username, &users.users),
            username,
            first_name: allowed.first_name.to_owned(),
            password_hash,
            created_at: Utc::now(),
        };
        users.users.push(user.clone());
        users
            .users
            .sort_by(|left, right| left.username.cmp(&right.username));
        self.save_users(&users).await?;
        self.session_for(user).map_err(AuthFailure::from)
    }

    pub async fn login(&self, username: &str, password: &str) -> Result<AuthSession, AuthFailure> {
        let username = normalize_username(username);
        let users = self.load_users().await?;
        let user = users
            .users
            .into_iter()
            .find(|user| normalize_username(&user.username) == username)
            .ok_or(AuthFailure::InvalidCredentials)?;
        verify_password(password, &user.password_hash)?;
        self.session_for(user).map_err(AuthFailure::from)
    }

    pub fn verify_token(&self, token: &str) -> Result<PublicUser, AuthFailure> {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.validate_exp = true;
        let data = decode::<Claims>(
            token,
            &DecodingKey::from_secret(self.token_secret.as_bytes()),
            &validation,
        )?;
        let username = if data.claims.username.trim().is_empty() {
            normalize_username(&data.claims.sub)
        } else {
            normalize_username(&data.claims.username)
        };
        let first_name = if data.claims.first_name.trim().is_empty() {
            display_first_name(&username)
        } else {
            display_first_name(&data.claims.first_name)
        };
        if username.is_empty() || first_name.is_empty() {
            return Err(AuthFailure::InvalidToken);
        }

        Ok(PublicUser {
            id: data.claims.sub,
            username,
            first_name,
        })
    }

    pub fn user_chat_store(&self, user: &PublicUser) -> ChatStore {
        ChatStore::new(self.data_dir.join("users").join(&user.id).join("chat"))
    }

    fn session_for(&self, user: StoredUser) -> anyhow::Result<AuthSession> {
        let now = Utc::now();
        let exp = now
            .checked_add_signed(self.token_ttl)
            .context("auth token expiration overflowed")?;
        let token = encode(
            &Header::new(Algorithm::HS256),
            &Claims {
                sub: user.id.clone(),
                username: user.username.clone(),
                first_name: user.first_name.clone(),
                iat: now.timestamp() as usize,
                exp: exp.timestamp() as usize,
            },
            &EncodingKey::from_secret(self.token_secret.as_bytes()),
        )
        .context("failed to sign auth token")?;

        let first_name = user_display_first_name(&user);
        Ok(AuthSession {
            token,
            user: PublicUser {
                id: user.id,
                username: user.username,
                first_name,
            },
        })
    }

    async fn load_users(&self) -> Result<UserFile, AuthFailure> {
        match read_json::<UserFile>(&self.users_path).await {
            Ok(users) => Ok(users),
            Err(err) if is_not_found(&err) => Ok(UserFile::default()),
            Err(err) => Err(AuthFailure::from(err)),
        }
    }

    async fn save_users(&self, users: &UserFile) -> Result<(), AuthFailure> {
        if let Some(parent) = self.users_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("failed to create auth directory {}", parent.display()))?;
        }
        write_json(&self.users_path, users).await?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct AuthSession {
    pub token: String,
    pub user: PublicUser,
}

#[derive(Debug)]
pub enum AuthFailure {
    NotAllowed,
    AlreadyRegistered,
    InvalidUsername,
    InvalidCredentials,
    InvalidToken,
    WeakPassword,
    Internal(anyhow::Error),
}

impl AuthFailure {
    pub fn user_message(&self) -> &'static str {
        match self {
            Self::NotAllowed => "that first name is not allowed to register",
            Self::AlreadyRegistered => "that username is already registered",
            Self::InvalidUsername => "username is required",
            Self::InvalidCredentials => "invalid username or password",
            Self::InvalidToken => "invalid or expired session",
            Self::WeakPassword => "password must be at least 8 characters",
            Self::Internal(_) => "authentication failed",
        }
    }
}

impl From<anyhow::Error> for AuthFailure {
    fn from(error: anyhow::Error) -> Self {
        Self::Internal(error)
    }
}

impl From<jsonwebtoken::errors::Error> for AuthFailure {
    fn from(error: jsonwebtoken::errors::Error) -> Self {
        match error.kind() {
            jsonwebtoken::errors::ErrorKind::ExpiredSignature
            | jsonwebtoken::errors::ErrorKind::InvalidToken
            | jsonwebtoken::errors::ErrorKind::InvalidSignature => Self::InvalidToken,
            _ => Self::Internal(error.into()),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct AllowedUser {
    first_name: &'static str,
}

fn allowed_first_name(value: &str) -> Option<AllowedUser> {
    let normalized = normalize_first_name(value);
    ALLOWED_FIRST_NAMES
        .iter()
        .find(|(id, _)| *id == normalized)
        .map(|(_, first_name)| AllowedUser { first_name })
}

fn normalize_username(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn normalize_first_name(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn display_first_name(value: &str) -> String {
    value.trim().to_owned()
}

fn user_display_first_name(user: &StoredUser) -> String {
    let first_name = user.first_name.trim();
    if first_name.is_empty() {
        user.username.clone()
    } else {
        first_name.to_owned()
    }
}

fn unique_user_id(username: &str, users: &[StoredUser]) -> String {
    let base = sanitize_user_id(username);
    if !users.iter().any(|user| user.id == base) {
        return base;
    }

    let mut index = 2;
    loop {
        let candidate = format!("{base}-{index}");
        if !users.iter().any(|user| user.id == candidate) {
            return candidate;
        }
        index += 1;
    }
}

fn sanitize_user_id(username: &str) -> String {
    let mut id = String::with_capacity(username.len());
    for ch in username.chars() {
        if ch.is_ascii_alphanumeric() {
            id.push(ch.to_ascii_lowercase());
        } else if matches!(ch, '-' | '_' | '.') {
            id.push(ch);
        } else {
            id.push('-');
        }
    }
    let id = id.trim_matches(['-', '.', '_']).to_owned();
    if id.is_empty() { "user".to_owned() } else { id }
}

fn validate_password(password: &str) -> Result<(), AuthFailure> {
    if password.len() < MIN_PASSWORD_LEN {
        Err(AuthFailure::WeakPassword)
    } else {
        Ok(())
    }
}

fn hash_password(password: &str) -> Result<String, AuthFailure> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| AuthFailure::Internal(anyhow::anyhow!(error.to_string())))
}

fn verify_password(password: &str, password_hash: &str) -> Result<(), AuthFailure> {
    let parsed_hash = PasswordHash::new(password_hash)
        .map_err(|error| AuthFailure::Internal(anyhow::anyhow!(error.to_string())))?;
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .map_err(|_| AuthFailure::InvalidCredentials)
}

async fn load_or_create_secret(path: &Path) -> anyhow::Result<String> {
    match tokio::fs::read_to_string(path).await {
        Ok(value) => {
            let secret = value.trim().to_owned();
            if secret.is_empty() {
                bail!("auth secret file is empty: {}", path.display());
            }
            Ok(secret)
        }
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent).await.with_context(|| {
                    format!(
                        "failed to create auth secret directory {}",
                        parent.display()
                    )
                })?;
            }
            let mut bytes = [0_u8; 32];
            OsRng.fill_bytes(&mut bytes);
            let secret = hex_encode(&bytes);
            let mut file = tokio::fs::File::create(path)
                .await
                .with_context(|| format!("failed to create auth secret {}", path.display()))?;
            file.write_all(secret.as_bytes())
                .await
                .with_context(|| format!("failed to write auth secret {}", path.display()))?;
            file.flush()
                .await
                .with_context(|| format!("failed to flush auth secret {}", path.display()))?;
            Ok(secret)
        }
        Err(source) => {
            Err(source).with_context(|| format!("failed to read auth secret {}", path.display()))
        }
    }
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

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserFile {
    #[serde(default)]
    users: Vec<StoredUser>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredUser {
    id: String,
    username: String,
    #[serde(default = "legacy_first_name")]
    first_name: String,
    password_hash: String,
    created_at: chrono::DateTime<Utc>,
}

fn legacy_first_name() -> String {
    String::new()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Claims {
    sub: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    first_name: String,
    iat: usize,
    exp: usize,
}

async fn read_json<T>(path: &Path) -> anyhow::Result<T>
where
    T: DeserializeOwned,
{
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
    let bytes = serde_json::to_vec_pretty(value)
        .with_context(|| format!("failed to serialize json file {}", path.display()))?;
    let tmp_path = path.with_extension(format!("tmp-{}", std::process::id()));
    let mut file = tokio::fs::File::create(&tmp_path)
        .await
        .with_context(|| format!("failed to create temp file {}", tmp_path.display()))?;
    file.write_all(&bytes)
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

fn is_not_found(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|source| source.kind() == std::io::ErrorKind::NotFound)
    })
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[tokio::test]
    async fn register_returns_token_that_verifies() {
        let data_dir = unique_test_dir("register-token");
        let auth = AuthStore::new(&data_dir, Duration::minutes(5))
            .await
            .expect("create auth store");

        let session = auth
            .register("cachebag", "Akrm", "correct horse")
            .await
            .expect("register user");
        let verified = auth.verify_token(&session.token).expect("verify token");

        assert_eq!(verified.id, session.user.id);
        assert_eq!(verified.username, "cachebag");
        assert_eq!(verified.first_name, "Akrm");

        let _ = tokio::fs::remove_dir_all(data_dir).await;
    }

    fn unique_test_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "rafael-chat-auth-{name}-{}-{nanos}",
            std::process::id()
        ))
    }
}
