use crate::types::StoredProvider;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptConfig {
    pub default_system_prompt: Option<String>,
}

impl PromptConfig {
    pub fn default_enabled(&self) -> bool {
        self.default_system_prompt
            .as_deref()
            .is_some_and(|prompt| !prompt.trim().is_empty())
    }

    #[cfg(test)]
    pub fn disabled() -> Self {
        Self {
            default_system_prompt: None,
        }
    }
}

impl Default for PromptConfig {
    fn default() -> Self {
        Self {
            default_system_prompt: Some(DEFAULT_RAFAEL_SYSTEM_PROMPT.to_owned()),
        }
    }
}

pub fn default_system_prompt_from_env(value: Option<String>) -> Option<String> {
    let Some(value) = value.map(|value| value.trim().to_owned()) else {
        return Some(DEFAULT_RAFAEL_SYSTEM_PROMPT.to_owned());
    };
    if value.is_empty() || value.eq_ignore_ascii_case("rafael") {
        return Some(DEFAULT_RAFAEL_SYSTEM_PROMPT.to_owned());
    }
    if matches!(
        value.to_ascii_lowercase().as_str(),
        "none" | "off" | "disabled"
    ) {
        return None;
    }
    Some(value)
}

pub fn effective_system_prompt(
    provider: &StoredProvider,
    prompt_config: &PromptConfig,
) -> Option<String> {
    provider
        .system_prompt
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            prompt_config
                .default_system_prompt
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        })
}

pub const DEFAULT_RAFAEL_SYSTEM_PROMPT: &str = "\
You are Rafael, a private local assistant for technical discussion, programming help, research, and practical planning.

Be direct, grounded, and useful. Prefer clear technical reasoning over polished filler. State assumptions when they matter. If a question has version, date, source, or recent-change sensitivity, use available web tools before answering. Prefer official documentation, source repositories, release notes, standards, and primary sources over blogs or summaries. When using web information, cite the source URLs.

For technical answers, identify relevant constraints, explain tradeoffs, and give concrete examples when they help. Do not bluff. If evidence is weak or conflicting, say exactly what is uncertain and what would verify it.

Match depth to the user's request. For quick questions, answer briefly. For complex design, debugging, or research questions, structure the answer, compare options, and end with a practical recommendation.";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ProviderKind;

    #[test]
    fn parses_default_system_prompt_env() {
        assert_eq!(
            default_system_prompt_from_env(None),
            Some(DEFAULT_RAFAEL_SYSTEM_PROMPT.to_owned())
        );
        assert_eq!(
            default_system_prompt_from_env(Some(" rafael ".to_owned())),
            Some(DEFAULT_RAFAEL_SYSTEM_PROMPT.to_owned())
        );
        assert_eq!(default_system_prompt_from_env(Some("off".to_owned())), None);
        assert_eq!(
            default_system_prompt_from_env(Some(" Custom prompt ".to_owned())),
            Some("Custom prompt".to_owned())
        );
    }

    #[test]
    fn provider_prompt_overrides_default_prompt() {
        let mut provider = test_provider();
        provider.system_prompt = Some(" Custom ".to_owned());
        let config = PromptConfig::default();

        assert_eq!(
            effective_system_prompt(&provider, &config),
            Some("Custom".to_owned())
        );
    }

    fn test_provider() -> StoredProvider {
        StoredProvider {
            id: "test".to_owned(),
            name: "Test".to_owned(),
            kind: ProviderKind::OpenAiCompatible,
            base_url: "http://localhost:8080/v1".to_owned(),
            model: "test-model".to_owned(),
            api_key: None,
            system_prompt: None,
        }
    }
}
