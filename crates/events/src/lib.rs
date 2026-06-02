use std::{fmt, str::FromStr};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::DeserializeOwned, de};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EventError {
    #[error("event kind must be non-empty")]
    EmptyKind,
    #[error("event kind `{0}` contains unsupported characters")]
    InvalidKind(String),
    #[error("failed to serialize event as json line: {0}")]
    Serialize(#[from] serde_json::Error),
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventKind(String);

impl EventKind {
    pub fn parse(value: impl Into<String>) -> Result<Self, EventError> {
        let value = value.into();
        if value.is_empty() {
            return Err(EventError::EmptyKind);
        }
        if !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | ':'))
        {
            return Err(EventError::InvalidKind(value));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for EventKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("EventKind").field(&self.0).finish()
    }
}

impl fmt::Display for EventKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for EventKind {
    type Err = EventError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl Serialize for EventKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for EventKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventEnvelope<T> {
    pub id: String,
    pub kind: EventKind,
    pub source: String,
    pub occurred_at: DateTime<Utc>,
    pub data: T,
}

impl<T> EventEnvelope<T> {
    pub fn new(
        id: impl Into<String>,
        kind: EventKind,
        source: impl Into<String>,
        occurred_at: DateTime<Utc>,
        data: T,
    ) -> Self {
        Self {
            id: id.into(),
            kind,
            source: source.into(),
            occurred_at,
            data,
        }
    }

    pub fn new_now(
        id: impl Into<String>,
        kind: EventKind,
        source: impl Into<String>,
        data: T,
    ) -> Self {
        Self::new(id, kind, source, Utc::now(), data)
    }

    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> EventEnvelope<U> {
        EventEnvelope {
            id: self.id,
            kind: self.kind,
            source: self.source,
            occurred_at: self.occurred_at,
            data: f(self.data),
        }
    }
}

pub type JsonEvent = EventEnvelope<serde_json::Value>;

#[derive(Debug, Clone, Default)]
pub struct EventFilter {
    pub kind: Option<EventKind>,
    pub source: Option<String>,
}

impl EventFilter {
    pub fn matches<T>(&self, event: &EventEnvelope<T>) -> bool {
        self.kind.as_ref().is_none_or(|kind| kind == &event.kind)
            && self
                .source
                .as_ref()
                .is_none_or(|source| source == &event.source)
    }
}

pub fn to_json_line<T: Serialize>(event: &EventEnvelope<T>) -> Result<String, EventError> {
    let mut line = serde_json::to_string(event)?;
    line.push('\n');
    Ok(line)
}

pub fn from_json_line<T: DeserializeOwned>(line: &str) -> Result<EventEnvelope<T>, EventError> {
    serde_json::from_str(line.trim_end()).map_err(EventError::Serialize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_kind_rejects_spaces() {
        assert!(EventKind::parse("model.output").is_ok());
        assert!(EventKind::parse("model output").is_err());
    }

    #[test]
    fn json_line_round_trips() {
        let event = EventEnvelope::new(
            "1",
            EventKind::parse("model.output").unwrap(),
            "test",
            Utc::now(),
            serde_json::json!({"ok": true}),
        );
        let line = to_json_line(&event).unwrap();
        let parsed: JsonEvent = from_json_line(&line).unwrap();

        assert_eq!(parsed.id, "1");
        assert_eq!(parsed.kind.as_str(), "model.output");
    }
}
