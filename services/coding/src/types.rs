use std::fmt;

use anyhow::{Context, bail};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RepoRef {
    pub owner: String,
    pub name: String,
}

impl RepoRef {
    pub fn parse(value: &str) -> anyhow::Result<Self> {
        let Some((owner, name)) = value.split_once('/') else {
            bail!("repository must be in owner/name form");
        };

        if owner.is_empty() || name.is_empty() {
            bail!("repository owner and name must be non-empty");
        }

        Ok(Self {
            owner: owner.to_owned(),
            name: name.to_owned(),
        })
    }

    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    pub fn safe_dir_name(&self) -> String {
        format!("{}__{}", self.owner, self.name)
    }
}

impl fmt::Display for RepoRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

impl std::str::FromStr for RepoRef {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value).with_context(|| format!("invalid repository `{value}`"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum TriggerKind {
    Label,
    Comment,
    Assignment,
    Manual,
    PullRequestReview,
    PullRequestComment,
    PullRequestReviewComment,
}

impl fmt::Display for TriggerKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Label => write!(f, "label"),
            Self::Comment => write!(f, "comment"),
            Self::Assignment => write!(f, "assignment"),
            Self::Manual => write!(f, "manual"),
            Self::PullRequestReview => write!(f, "pull_request_review"),
            Self::PullRequestComment => write!(f, "pull_request_comment"),
            Self::PullRequestReviewComment => write!(f, "pull_request_review_comment"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueTrigger {
    pub repo: RepoRef,
    pub issue_number: u64,
    pub trigger: TriggerKind,
    pub actor: Option<String>,
    pub installation_id: Option<u64>,
    pub run_id: String,
    pub default_branch: Option<String>,
}

impl IssueTrigger {
    pub fn local(
        repo: RepoRef,
        issue_number: u64,
        trigger: TriggerKind,
        actor: Option<String>,
        installation_id: Option<u64>,
        run_id: Option<String>,
    ) -> Self {
        Self {
            repo,
            issue_number,
            trigger,
            actor,
            installation_id,
            run_id: run_id.unwrap_or_else(new_run_id),
            default_branch: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullRequestRevisionTrigger {
    pub repo: RepoRef,
    pub pull_number: u64,
    pub trigger: TriggerKind,
    pub actor: Option<String>,
    pub installation_id: Option<u64>,
    pub run_id: String,
    pub default_branch: Option<String>,
    pub head_branch: Option<String>,
}

pub fn new_run_id() -> String {
    format!("run-{}", chrono::Utc::now().timestamp())
}
