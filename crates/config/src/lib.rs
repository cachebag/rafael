use std::{collections::BTreeMap, env, path::PathBuf, str::FromStr};

use common::{looks_sensitive_name, parse_bool, split_delimited};
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ConfigError {
    #[error("{name} is required")]
    Missing { name: String },
    #[error("{name} has invalid value {value}: {reason}")]
    InvalidValue {
        name: String,
        value: String,
        reason: String,
    },
}

#[derive(Debug, Clone, Default)]
pub struct EnvConfig {
    vars: BTreeMap<String, String>,
}

impl EnvConfig {
    pub fn from_current_env() -> Self {
        Self {
            vars: env::vars().collect(),
        }
    }

    pub fn reader(&self) -> EnvReader<'_> {
        EnvReader {
            vars: &self.vars,
            prefix: None,
        }
    }

    pub fn prefixed(&self, prefix: impl Into<String>) -> EnvReader<'_> {
        EnvReader {
            vars: &self.vars,
            prefix: Some(prefix.into()),
        }
    }
}

impl<K, V> FromIterator<(K, V)> for EnvConfig
where
    K: Into<String>,
    V: Into<String>,
{
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        Self {
            vars: iter
                .into_iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct EnvReader<'a> {
    vars: &'a BTreeMap<String, String>,
    prefix: Option<String>,
}

impl<'a> EnvReader<'a> {
    pub fn optional_string(&self, name: &str) -> Option<String> {
        self.raw(name)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    }

    pub fn required_string(&self, name: &str) -> Result<String, ConfigError> {
        self.optional_string(name)
            .ok_or_else(|| ConfigError::Missing {
                name: self.full_name(name),
            })
    }

    pub fn string_or(&self, name: &str, default: impl Into<String>) -> String {
        self.optional_string(name).unwrap_or_else(|| default.into())
    }

    pub fn required_parse<T>(&self, name: &str) -> Result<T, ConfigError>
    where
        T: FromStr,
        T::Err: std::fmt::Display,
    {
        let value = self.required_string(name)?;
        self.parse_value(name, &value)
    }

    pub fn parse_or<T>(&self, name: &str, default: T) -> Result<T, ConfigError>
    where
        T: FromStr,
        T::Err: std::fmt::Display,
    {
        let Some(value) = self.optional_string(name) else {
            return Ok(default);
        };
        self.parse_value(name, &value)
    }

    pub fn bool_or(&self, name: &str, default: bool) -> Result<bool, ConfigError> {
        let Some(value) = self.optional_string(name) else {
            return Ok(default);
        };
        parse_bool(&value).map_err(|err| ConfigError::InvalidValue {
            name: self.full_name(name),
            value: display_value(&self.full_name(name), &value),
            reason: err.to_string(),
        })
    }

    pub fn list(&self, name: &str, delimiters: &[char]) -> Vec<String> {
        self.optional_string(name)
            .map(|value| split_delimited(&value, delimiters))
            .unwrap_or_default()
    }

    pub fn csv(&self, name: &str) -> Vec<String> {
        self.list(name, &[','])
    }

    pub fn path_or(&self, name: &str, default: impl Into<PathBuf>) -> PathBuf {
        self.optional_string(name)
            .map(PathBuf::from)
            .unwrap_or_else(|| default.into())
    }

    pub fn required_path(&self, name: &str) -> Result<PathBuf, ConfigError> {
        self.required_string(name).map(PathBuf::from)
    }

    fn raw(&self, name: &str) -> Option<&str> {
        self.vars.get(&self.full_name(name)).map(String::as_str)
    }

    fn full_name(&self, name: &str) -> String {
        match self.prefix.as_deref() {
            Some(prefix) => format!("{prefix}{name}"),
            None => name.to_owned(),
        }
    }

    fn parse_value<T>(&self, name: &str, value: &str) -> Result<T, ConfigError>
    where
        T: FromStr,
        T::Err: std::fmt::Display,
    {
        value.parse().map_err(|err: T::Err| {
            let full_name = self.full_name(name);
            ConfigError::InvalidValue {
                name: full_name.clone(),
                value: display_value(&full_name, value),
                reason: err.to_string(),
            }
        })
    }
}

fn display_value(name: &str, value: &str) -> String {
    if looks_sensitive_name(name) {
        "<redacted>".to_owned()
    } else {
        format!("`{value}`")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_prefixed_values() {
        let env: EnvConfig = [("APP_PORT", "8080"), ("APP_DEBUG", "yes")]
            .into_iter()
            .collect();
        let reader = env.prefixed("APP_");

        assert_eq!(reader.required_parse::<u16>("PORT").unwrap(), 8080);
        assert!(reader.bool_or("DEBUG", false).unwrap());
    }

    #[test]
    fn trims_and_splits_lists() {
        let env: EnvConfig = [("ITEMS", "a, b,,c")].into_iter().collect();

        assert_eq!(env.reader().csv("ITEMS"), ["a", "b", "c"]);
    }

    #[test]
    fn redacts_sensitive_invalid_values() {
        let env: EnvConfig = [("API_TOKEN", "abc")].into_iter().collect();
        let error = env.reader().required_parse::<u64>("API_TOKEN").unwrap_err();

        assert!(error.to_string().contains("<redacted>"));
        assert!(!error.to_string().contains("abc"));
    }
}
