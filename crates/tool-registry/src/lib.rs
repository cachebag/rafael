use std::{collections::BTreeMap, fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("tool name must be non-empty")]
    EmptyToolName,
    #[error("tool name `{0}` contains unsupported characters")]
    InvalidToolName(String),
    #[error("tool `{0}` is already registered")]
    DuplicateTool(String),
    #[error("tool `{0}` is not registered")]
    UnknownTool(String),
    #[error("tool `{0}` must have a non-empty description")]
    EmptyDescription(String),
    #[error("tool `{0}` parameters must be a json schema object")]
    InvalidParameters(String),
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ToolName(String);

impl ToolName {
    pub fn parse(value: impl Into<String>) -> Result<Self, RegistryError> {
        let value = value.into();
        if value.is_empty() {
            return Err(RegistryError::EmptyToolName);
        }
        if value.starts_with('-')
            || !value
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':'))
        {
            return Err(RegistryError::InvalidToolName(value));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ToolName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ToolName").field(&self.0).finish()
    }
}

impl fmt::Display for ToolName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for ToolName {
    type Err = RegistryError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl Serialize for ToolName {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ToolName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: ToolName,
    pub description: String,
    pub parameters: serde_json::Value,
}

impl ToolDefinition {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: serde_json::Value,
    ) -> Result<Self, RegistryError> {
        let name = ToolName::parse(name.into())?;
        let description = description.into();
        if description.trim().is_empty() {
            return Err(RegistryError::EmptyDescription(name.to_string()));
        }
        if !parameters.is_object() {
            return Err(RegistryError::InvalidParameters(name.to_string()));
        }
        Ok(Self {
            name,
            description,
            parameters,
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct ToolRegistry {
    tools: BTreeMap<String, ToolDefinition>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_tools(
        tools: impl IntoIterator<Item = ToolDefinition>,
    ) -> Result<Self, RegistryError> {
        let mut registry = Self::new();
        for tool in tools {
            registry.register(tool)?;
        }
        Ok(registry)
    }

    pub fn register(&mut self, tool: ToolDefinition) -> Result<(), RegistryError> {
        let name = tool.name.as_str().to_owned();
        if self.tools.contains_key(&name) {
            return Err(RegistryError::DuplicateTool(name));
        }
        self.tools.insert(name, tool);
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&ToolDefinition> {
        self.tools.get(name)
    }

    pub fn definitions(&self) -> impl Iterator<Item = &ToolDefinition> {
        self.tools.values()
    }

    pub fn validate_invocation(&self, invocation: &ToolInvocation) -> Result<(), RegistryError> {
        if self.tools.contains_key(invocation.name.as_str()) {
            Ok(())
        } else {
            Err(RegistryError::UnknownTool(invocation.name.to_string()))
        }
    }

    pub fn openai_tools(&self) -> Vec<OpenAiToolSpec> {
        self.tools
            .values()
            .map(|tool| OpenAiToolSpec {
                tool_type: "function",
                function: OpenAiFunctionSpec {
                    name: tool.name.to_string(),
                    description: tool.description.clone(),
                    parameters: tool.parameters.clone(),
                },
            })
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInvocation {
    pub name: ToolName,
    #[serde(default)]
    pub arguments: serde_json::Value,
}

impl ToolInvocation {
    pub fn new(
        name: impl Into<String>,
        arguments: serde_json::Value,
    ) -> Result<Self, RegistryError> {
        Ok(Self {
            name: ToolName::parse(name.into())?,
            arguments,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenAiToolSpec {
    #[serde(rename = "type")]
    pub tool_type: &'static str,
    pub function: OpenAiFunctionSpec,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenAiFunctionSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

pub fn object_schema(
    properties: serde_json::Map<String, serde_json::Value>,
    required: Vec<String>,
) -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_rejects_duplicates() {
        let schema = serde_json::json!({"type": "object"});
        let mut registry = ToolRegistry::new();
        registry
            .register(ToolDefinition::new("read_file", "read a file", schema.clone()).unwrap())
            .unwrap();

        assert!(
            registry
                .register(ToolDefinition::new("read_file", "read a file", schema).unwrap())
                .is_err()
        );
    }

    #[test]
    fn validates_invocation_names() {
        let mut registry = ToolRegistry::new();
        registry
            .register(
                ToolDefinition::new(
                    "search",
                    "search text",
                    serde_json::json!({"type": "object"}),
                )
                .unwrap(),
            )
            .unwrap();

        assert!(
            registry
                .validate_invocation(&ToolInvocation::new("search", serde_json::json!({})).unwrap())
                .is_ok()
        );
        assert!(
            registry
                .validate_invocation(&ToolInvocation::new("write", serde_json::json!({})).unwrap())
                .is_err()
        );
    }
}
