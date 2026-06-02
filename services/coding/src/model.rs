use anyhow::Context;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::{
    config::ModelConfig,
    github::{IssueInfo, RepositoryInfo},
    repo_context::RepoContext,
};

#[derive(Clone)]
pub struct ModelClient {
    http: Client,
    base_url: String,
    model: String,
}

impl ModelClient {
    pub fn new(config: &ModelConfig) -> anyhow::Result<Self> {
        Ok(Self {
            http: Client::builder()
                .build()
                .context("failed to build model HTTP client")?,
            base_url: config.base_url.trim_end_matches('/').to_owned(),
            model: config.name.clone(),
        })
    }

    pub async fn issue_plan(
        &self,
        repo: &RepositoryInfo,
        issue: &IssueInfo,
        branch_name: &str,
        context: &RepoContext,
    ) -> anyhow::Result<String> {
        let body = context.issue.body.as_deref().unwrap_or("(no issue body)");
        let labels = issue
            .labels
            .iter()
            .map(|label| label.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let context_json = serde_json::to_string(context)
            .context("failed to serialize repository context for model prompt")?;

        let prompt = format!(
            "Repository: {}\nDefault branch: {}\nTarget branch: {}\nIssue #{}: {}\nLabels: {}\nIssue body:\n{}\n\nRepository context JSON:\n{}\n\nWrite a concise implementation plan with these sections:\n- Files likely affected\n- Implementation steps\n- Verification commands\n- Blockers or questions\n\nDo not include code. If the likely files are unclear, state why.",
            repo.full_name,
            repo.default_branch,
            branch_name,
            issue.number,
            issue.title,
            if labels.is_empty() { "(none)" } else { &labels },
            body,
            context_json
        );

        self.chat(
            "You are planning a scoped code change for an AI coding worker. Keep the plan concrete, conservative, and reviewable.",
            &prompt,
            0.2,
        )
        .await
    }

    pub async fn change_action(&self, prompt: &str) -> anyhow::Result<String> {
        self.chat(
            "You are implementing a scoped local code change through a constrained JSON tool loop. Return exactly one JSON object for the next action and no other text.",
            prompt,
            0.0,
        )
        .await
    }

    async fn chat(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        temperature: f32,
    ) -> anyhow::Result<String> {
        let response = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .json(&ChatCompletionsRequest {
                model: &self.model,
                temperature,
                stream: false,
                messages: vec![
                    ChatMessage {
                        role: "system",
                        content: system_prompt,
                    },
                    ChatMessage {
                        role: "user",
                        content: user_prompt,
                    },
                ],
            })
            .send()
            .await
            .context("failed to call model endpoint")?
            .error_for_status()
            .context("model endpoint returned an error")?
            .json::<ChatCompletionsResponse>()
            .await
            .context("failed to parse model response")?;

        Ok(response
            .choices
            .into_iter()
            .next()
            .map(|choice| choice.message.content)
            .unwrap_or_else(|| "No response returned by the model.".to_owned()))
    }
}

#[derive(Debug, Serialize)]
struct ChatCompletionsRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    temperature: f32,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionsResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatChoiceMessage,
}

#[derive(Debug, Deserialize)]
struct ChatChoiceMessage {
    content: String,
}
