use std::{collections::HashSet, fmt, hash::Hash, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CommonError {
    #[error("safe component must be non-empty")]
    EmptyComponent,
    #[error("safe component `{value}` must not be `.` or `..`")]
    DotComponent { value: String },
    #[error("safe component `{value}` must not contain path separators")]
    ComponentContainsSeparator { value: String },
    #[error("safe component must not contain NUL bytes")]
    ComponentContainsNul,
    #[error("invalid boolean value `{value}`")]
    InvalidBool { value: String },
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SafeComponent(String);

impl SafeComponent {
    pub fn parse(value: impl Into<String>) -> Result<Self, CommonError> {
        let value = value.into();
        if value.is_empty() {
            return Err(CommonError::EmptyComponent);
        }
        if value == "." || value == ".." {
            return Err(CommonError::DotComponent { value });
        }
        if value.contains('\0') {
            return Err(CommonError::ComponentContainsNul);
        }
        if value.contains('/') || value.contains('\\') {
            return Err(CommonError::ComponentContainsSeparator { value });
        }

        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Debug for SafeComponent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("SafeComponent").field(&self.0).finish()
    }
}

impl fmt::Display for SafeComponent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for SafeComponent {
    type Err = CommonError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl Serialize for SafeComponent {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for SafeComponent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(de::Error::custom)
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose_secret(&self) -> &str {
        &self.0
    }

    pub fn into_secret(self) -> String {
        self.0
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretString(<redacted>)")
    }
}

impl fmt::Display for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TruncatedText {
    pub text: String,
    pub original_bytes: usize,
    pub truncated: bool,
}

pub fn truncate_utf8(value: &str, max_bytes: usize) -> TruncatedText {
    if value.len() <= max_bytes {
        return TruncatedText {
            text: value.to_owned(),
            original_bytes: value.len(),
            truncated: false,
        };
    }

    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }

    TruncatedText {
        text: value[..end].to_owned(),
        original_bytes: value.len(),
        truncated: true,
    }
}

pub fn parse_bool(value: &str) -> Result<bool, CommonError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "y" | "on" => Ok(true),
        "0" | "false" | "no" | "n" | "off" => Ok(false),
        _ => Err(CommonError::InvalidBool {
            value: value.to_owned(),
        }),
    }
}

pub fn split_delimited(value: &str, delimiters: &[char]) -> Vec<String> {
    value
        .split(|ch| delimiters.contains(&ch))
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

pub fn dedupe_preserving_order<T>(items: impl IntoIterator<Item = T>) -> Vec<T>
where
    T: Clone + Eq + Hash,
{
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();

    for item in items {
        if seen.insert(item.clone()) {
            deduped.push(item);
        }
    }

    deduped
}

pub fn slugify(value: &str, max_len: usize, fallback: &str) -> String {
    let mut slug = String::new();
    let mut last_was_dash = false;

    for ch in value.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            last_was_dash = false;
        } else if !last_was_dash && !slug.is_empty() {
            slug.push('-');
            last_was_dash = true;
        }

        if slug.len() >= max_len {
            break;
        }
    }

    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        fallback.to_owned()
    } else {
        slug.to_owned()
    }
}

pub fn looks_sensitive_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper.contains("TOKEN")
        || upper.contains("SECRET")
        || upper.contains("PASSWORD")
        || upper.contains("PRIVATE_KEY")
        || upper.contains("CREDENTIAL")
        || upper.ends_with("_KEY")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_component_rejects_path_input() {
        assert!(SafeComponent::parse("").is_err());
        assert!(SafeComponent::parse("..").is_err());
        assert!(SafeComponent::parse("a/b").is_err());
        assert!(SafeComponent::parse("a\\b").is_err());
        assert!(SafeComponent::parse("run-1").is_ok());
    }

    #[test]
    fn truncate_preserves_utf8_boundary() {
        let truncated = truncate_utf8("abé", 3);
        assert_eq!(truncated.text, "ab");
        assert!(truncated.truncated);
    }

    #[test]
    fn dedupe_preserves_first_seen_order() {
        let values = dedupe_preserving_order(["a", "b", "a", "c"]);
        assert_eq!(values, ["a", "b", "c"]);
    }

    #[test]
    fn slugify_uses_fallback_for_empty_output() {
        assert_eq!(slugify("?!", 32, "item"), "item");
    }
}
