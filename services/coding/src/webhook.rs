use anyhow::{Context, bail};
use hmac::{Hmac, KeyInit, Mac};
use serde::Deserialize;
use sha2::Sha256;

use crate::{
    config::AppConfig,
    types::{IssueTrigger, PullRequestRevisionTrigger, RepoRef, TriggerKind},
};

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone)]
pub enum WebhookDecision {
    AcceptedIssue(IssueTrigger),
    AcceptedPullRequestRevision(PullRequestRevisionTrigger),
    Ignored { reason: String },
}

pub fn verify_signature(secret: &str, signature_header: &str, body: &[u8]) -> anyhow::Result<()> {
    let Some(hex_signature) = signature_header.strip_prefix("sha256=") else {
        bail!("GitHub webhook signature must start with sha256=");
    };

    let signature = hex::decode(hex_signature).context("invalid webhook signature hex")?;
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).context("invalid webhook secret")?;
    mac.update(body);
    mac.verify_slice(&signature)
        .context("GitHub webhook signature mismatch")?;
    Ok(())
}

pub fn evaluate_event(
    config: &AppConfig,
    event_name: &str,
    delivery_id: &str,
    body: &[u8],
) -> anyhow::Result<WebhookDecision> {
    match event_name {
        "issues" => evaluate_issues_event(config, delivery_id, body),
        "issue_comment" => evaluate_issue_comment_event(config, delivery_id, body),
        "pull_request_review" => evaluate_pull_request_review_event(config, delivery_id, body),
        "pull_request_review_comment" => {
            evaluate_pull_request_review_comment_event(config, delivery_id, body)
        }
        _ => Ok(WebhookDecision::Ignored {
            reason: format!("ignored unsupported event `{event_name}`"),
        }),
    }
}

fn evaluate_issues_event(
    config: &AppConfig,
    delivery_id: &str,
    body: &[u8],
) -> anyhow::Result<WebhookDecision> {
    let event: IssuesEvent =
        serde_json::from_slice(body).context("failed to parse issues webhook payload")?;
    let repo = repo_ref(&event.repository);
    let full_name = repo.full_name();

    if event.issue.pull_request.is_some() {
        return ignored("ignored pull request issue event");
    }

    if !config.github.is_allowed_repo(&full_name) {
        return ignored(format!("ignored repository `{full_name}`"));
    }

    if has_blocking_label(config, &event.issue.labels) {
        return ignored("ignored issue with blocking label");
    }

    let default_branch = Some(event.repository.default_branch.clone());
    let actor = Some(event.sender.login.clone());
    let installation_id = event.installation.map(|installation| installation.id);

    match event.action.as_str() {
        "labeled"
            if event.label.as_ref().is_some_and(|label| {
                label
                    .name
                    .eq_ignore_ascii_case(&config.github.implementation_label)
            }) =>
        {
            Ok(WebhookDecision::AcceptedIssue(IssueTrigger {
                repo,
                issue_number: event.issue.number,
                trigger: TriggerKind::Label,
                actor,
                installation_id,
                run_id: delivery_id.to_owned(),
                default_branch,
            }))
        }
        "assigned"
            if config.github.enable_assignment_trigger
                && event.assignee.as_ref().is_some_and(|assignee| {
                    assignee
                        .login
                        .eq_ignore_ascii_case(&config.github.collaborator_login)
                }) =>
        {
            Ok(WebhookDecision::AcceptedIssue(IssueTrigger {
                repo,
                issue_number: event.issue.number,
                trigger: TriggerKind::Assignment,
                actor,
                installation_id,
                run_id: delivery_id.to_owned(),
                default_branch,
            }))
        }
        _ => ignored(format!("ignored issues action `{}`", event.action)),
    }
}

fn evaluate_issue_comment_event(
    config: &AppConfig,
    delivery_id: &str,
    body: &[u8],
) -> anyhow::Result<WebhookDecision> {
    let event: IssueCommentEvent =
        serde_json::from_slice(body).context("failed to parse issue_comment webhook payload")?;
    let repo = repo_ref(&event.repository);
    let full_name = repo.full_name();

    if event.action != "created" {
        return ignored(format!("ignored issue_comment action `{}`", event.action));
    }

    if !config.github.is_allowed_repo(&full_name) {
        return ignored(format!("ignored repository `{full_name}`"));
    }

    if has_blocking_label(config, &event.issue.labels) {
        return ignored("ignored issue with blocking label");
    }

    if !config.github.is_trusted_user(&event.sender.login) {
        return ignored(format!(
            "ignored untrusted command sender `{}`",
            event.sender.login
        ));
    }

    if event.issue.pull_request.is_some() {
        if !contains_revision_command(&event.comment.body, &config.github.command_mention) {
            return ignored("ignored pull request comment without collaborator revision command");
        }

        return Ok(WebhookDecision::AcceptedPullRequestRevision(
            PullRequestRevisionTrigger {
                repo,
                pull_number: event.issue.number,
                trigger: TriggerKind::PullRequestComment,
                actor: Some(event.sender.login),
                installation_id: event.installation.map(|installation| installation.id),
                run_id: delivery_id.to_owned(),
                default_branch: Some(event.repository.default_branch),
                head_branch: None,
            },
        ));
    }

    if !contains_command(&event.comment.body, &config.github.command_mention) {
        return ignored("ignored comment without collaborator command");
    }

    Ok(WebhookDecision::AcceptedIssue(IssueTrigger {
        repo,
        issue_number: event.issue.number,
        trigger: TriggerKind::Comment,
        actor: Some(event.sender.login),
        installation_id: event.installation.map(|installation| installation.id),
        run_id: delivery_id.to_owned(),
        default_branch: Some(event.repository.default_branch),
    }))
}

fn evaluate_pull_request_review_event(
    config: &AppConfig,
    delivery_id: &str,
    body: &[u8],
) -> anyhow::Result<WebhookDecision> {
    let event: PullRequestReviewEvent =
        serde_json::from_slice(body).context("failed to parse pull_request_review payload")?;
    let repo = repo_ref(&event.repository);
    let full_name = repo.full_name();

    if event.action != "submitted" {
        return ignored(format!(
            "ignored pull_request_review action `{}`",
            event.action
        ));
    }

    if !config.github.is_allowed_repo(&full_name) {
        return ignored(format!("ignored repository `{full_name}`"));
    }

    if !config.github.is_trusted_user(&event.sender.login) {
        return ignored(format!(
            "ignored untrusted review sender `{}`",
            event.sender.login
        ));
    }

    if !event.pull_request.state.eq_ignore_ascii_case("open") {
        return ignored("ignored non-open pull request review event");
    }

    if !pull_request_head_is_same_repo(&event.pull_request, &full_name) {
        return ignored("ignored pull request from a different head repository");
    }

    if event.review.state.eq_ignore_ascii_case("approved") {
        return ignored("ignored approving pull request review");
    }
    if !matches!(
        event.review.state.to_ascii_lowercase().as_str(),
        "changes_requested" | "commented"
    ) {
        return ignored(format!(
            "ignored pull_request_review state `{}`",
            event.review.state
        ));
    }

    Ok(WebhookDecision::AcceptedPullRequestRevision(
        PullRequestRevisionTrigger {
            repo,
            pull_number: event.pull_request.number,
            trigger: TriggerKind::PullRequestReview,
            actor: Some(event.sender.login),
            installation_id: event.installation.map(|installation| installation.id),
            run_id: delivery_id.to_owned(),
            default_branch: Some(event.repository.default_branch),
            head_branch: Some(event.pull_request.head.ref_name),
        },
    ))
}

fn evaluate_pull_request_review_comment_event(
    config: &AppConfig,
    delivery_id: &str,
    body: &[u8],
) -> anyhow::Result<WebhookDecision> {
    let event: PullRequestReviewCommentEvent = serde_json::from_slice(body)
        .context("failed to parse pull_request_review_comment payload")?;
    let repo = repo_ref(&event.repository);
    let full_name = repo.full_name();

    if event.action != "created" {
        return ignored(format!(
            "ignored pull_request_review_comment action `{}`",
            event.action
        ));
    }

    if !config.github.is_allowed_repo(&full_name) {
        return ignored(format!("ignored repository `{full_name}`"));
    }

    if !config.github.is_trusted_user(&event.sender.login) {
        return ignored(format!(
            "ignored untrusted review comment sender `{}`",
            event.sender.login
        ));
    }

    if !event.pull_request.state.eq_ignore_ascii_case("open") {
        return ignored("ignored non-open pull request review comment event");
    }

    if !pull_request_head_is_same_repo(&event.pull_request, &full_name) {
        return ignored("ignored pull request from a different head repository");
    }

    if !contains_revision_command(&event.comment.body, &config.github.command_mention) {
        return ignored(
            "ignored pull request review comment without collaborator revision command",
        );
    }

    Ok(WebhookDecision::AcceptedPullRequestRevision(
        PullRequestRevisionTrigger {
            repo,
            pull_number: event.pull_request.number,
            trigger: TriggerKind::PullRequestReviewComment,
            actor: Some(event.sender.login),
            installation_id: event.installation.map(|installation| installation.id),
            run_id: delivery_id.to_owned(),
            default_branch: Some(event.repository.default_branch),
            head_branch: Some(event.pull_request.head.ref_name),
        },
    ))
}

fn contains_command(body: &str, mention: &str) -> bool {
    let mention = mention.to_ascii_lowercase();

    body.lines().any(|line| {
        let lower = line.trim().to_ascii_lowercase();
        if !lower.starts_with(&mention) {
            return false;
        }

        let mut parts = lower.split_whitespace();
        matches!(parts.next(), Some(value) if value == mention)
            && matches!(parts.next(), Some("implement" | "retry"))
    })
}

fn contains_revision_command(body: &str, mention: &str) -> bool {
    let mention = mention.to_ascii_lowercase();

    body.lines().any(|line| {
        let lower = line.trim().to_ascii_lowercase();
        if !lower.starts_with(&mention) {
            return false;
        }

        let mut parts = lower.split_whitespace();
        matches!(parts.next(), Some(value) if value == mention)
            && matches!(parts.next(), Some("revise" | "retry"))
    })
}

fn has_blocking_label(config: &AppConfig, labels: &[Label]) -> bool {
    labels
        .iter()
        .any(|label| config.github.is_blocking_label(&label.name))
}

fn pull_request_head_is_same_repo(pull_request: &PullRequestPayload, full_name: &str) -> bool {
    pull_request
        .head
        .repo
        .as_ref()
        .is_some_and(|repo| repo.full_name.eq_ignore_ascii_case(full_name))
}

fn ignored(reason: impl Into<String>) -> anyhow::Result<WebhookDecision> {
    Ok(WebhookDecision::Ignored {
        reason: reason.into(),
    })
}

fn repo_ref(repository: &RepositoryPayload) -> RepoRef {
    RepoRef {
        owner: repository.owner.login.clone(),
        name: repository.name.clone(),
    }
}

#[derive(Debug, Deserialize)]
struct IssuesEvent {
    action: String,
    issue: IssuePayload,
    repository: RepositoryPayload,
    installation: Option<InstallationPayload>,
    sender: UserPayload,
    label: Option<Label>,
    assignee: Option<UserPayload>,
}

#[derive(Debug, Deserialize)]
struct IssueCommentEvent {
    action: String,
    issue: IssuePayload,
    comment: CommentPayload,
    repository: RepositoryPayload,
    installation: Option<InstallationPayload>,
    sender: UserPayload,
}

#[derive(Debug, Deserialize)]
struct PullRequestReviewEvent {
    action: String,
    review: ReviewPayload,
    pull_request: PullRequestPayload,
    repository: RepositoryPayload,
    installation: Option<InstallationPayload>,
    sender: UserPayload,
}

#[derive(Debug, Deserialize)]
struct PullRequestReviewCommentEvent {
    action: String,
    comment: CommentPayload,
    pull_request: PullRequestPayload,
    repository: RepositoryPayload,
    installation: Option<InstallationPayload>,
    sender: UserPayload,
}

#[derive(Debug, Deserialize)]
struct IssuePayload {
    number: u64,
    #[serde(default)]
    labels: Vec<Label>,
    #[serde(default)]
    pull_request: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct CommentPayload {
    body: String,
}

#[derive(Debug, Deserialize)]
struct ReviewPayload {
    state: String,
}

#[derive(Debug, Deserialize)]
struct PullRequestPayload {
    number: u64,
    state: String,
    head: PullRequestHeadPayload,
}

#[derive(Debug, Deserialize)]
struct PullRequestHeadPayload {
    #[serde(rename = "ref")]
    ref_name: String,
    repo: Option<PullRequestRepoPayload>,
}

#[derive(Debug, Deserialize)]
struct PullRequestRepoPayload {
    full_name: String,
}

#[derive(Debug, Deserialize)]
struct RepositoryPayload {
    name: String,
    default_branch: String,
    owner: UserPayload,
}

#[derive(Debug, Deserialize)]
struct InstallationPayload {
    id: u64,
}

#[derive(Debug, Deserialize)]
struct UserPayload {
    login: String,
}

#[derive(Debug, Deserialize)]
struct Label {
    name: String,
}

#[cfg(test)]
mod tests {
    use std::{net::SocketAddr, path::PathBuf};

    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;

    use super::*;
    use crate::config::{GitHubConfig, ModelConfig, ServerConfig, WorkspaceConfig};

    fn test_config() -> AppConfig {
        AppConfig {
            model: ModelConfig {
                base_url: "http://localhost:8080/v1".to_owned(),
                name: "test-model".to_owned(),
            },
            github: GitHubConfig {
                app_id: 1,
                installation_id: Some(2),
                private_key_path: PathBuf::from("/tmp/key.pem"),
                webhook_secret: Some("secret".to_owned()),
                app_slug: "netshared".to_owned(),
                collaborator_login: "netshared[bot]".to_owned(),
                allowed_repos: vec!["cachebag/rafael".to_owned()],
                implementation_label: "netshared:implement".to_owned(),
                command_mention: "@netshared".to_owned(),
                trusted_users: vec!["cachebag".to_owned()],
                blocking_labels: vec!["blocked".to_owned()],
                enable_assignment_trigger: false,
                quiet_comments: false,
                api_base_url: "https://api.github.com".to_owned(),
                git_author_name: "netshared[bot]".to_owned(),
                git_author_email: "1+netshared[bot]@users.noreply.github.com".to_owned(),
            },
            workspace: WorkspaceConfig {
                workdir: PathBuf::from("/tmp/work"),
                runs_dir: PathBuf::from("/tmp/runs"),
                max_run_minutes: 45,
                verify_commands: Vec::new(),
                max_tool_iterations: 24,
                max_tool_runtime_seconds: 900,
                max_file_read_bytes: 131_072,
                max_write_bytes: 262_144,
                max_changed_files: 12,
                verification_command_timeout_seconds: 600,
                verification_total_timeout_seconds: 1_200,
                allow_unverified_publish: false,
            },
            server: ServerConfig {
                bind: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
            },
        }
    }

    #[test]
    fn verifies_valid_signature() {
        let body = br#"{"ok":true}"#;
        let mut mac = Hmac::<Sha256>::new_from_slice(b"secret").unwrap();
        mac.update(body);
        let signature = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

        verify_signature("secret", &signature, body).unwrap();
    }

    #[test]
    fn rejects_invalid_signature() {
        let body = br#"{"ok":true}"#;
        let err = verify_signature("secret", "sha256=00", body).unwrap_err();
        assert!(err.to_string().contains("mismatch"));
    }

    #[test]
    fn accepts_implementation_label() {
        let body = br#"{
            "action": "labeled",
            "issue": {
                "number": 7,
                "labels": [{"name":"netshared:implement"}]
            },
            "repository": {
                "name": "rafael",
                "default_branch": "master",
                "owner": {"login":"cachebag"}
            },
            "installation": {"id": 42},
            "sender": {"login":"cachebag"},
            "label": {"name":"netshared:implement"}
        }"#;

        let decision = evaluate_event(&test_config(), "issues", "delivery-1", body).unwrap();
        match decision {
            WebhookDecision::AcceptedIssue(trigger) => {
                assert_eq!(trigger.issue_number, 7);
                assert_eq!(trigger.trigger, TriggerKind::Label);
                assert_eq!(trigger.installation_id, Some(42));
            }
            WebhookDecision::AcceptedPullRequestRevision(_) => {
                panic!("unexpected pull request revision trigger")
            }
            WebhookDecision::Ignored { reason } => panic!("unexpected ignore: {reason}"),
        }
    }

    #[test]
    fn accepts_trusted_comment_command() {
        let body = br#"{
            "action": "created",
            "issue": {
                "number": 8,
                "labels": []
            },
            "comment": {"body":"@netshared implement"},
            "repository": {
                "name": "rafael",
                "default_branch": "master",
                "owner": {"login":"cachebag"}
            },
            "installation": {"id": 42},
            "sender": {"login":"cachebag"}
        }"#;

        let decision = evaluate_event(&test_config(), "issue_comment", "delivery-2", body).unwrap();
        match decision {
            WebhookDecision::AcceptedIssue(trigger) => {
                assert_eq!(trigger.issue_number, 8);
                assert_eq!(trigger.trigger, TriggerKind::Comment);
            }
            WebhookDecision::AcceptedPullRequestRevision(_) => {
                panic!("unexpected pull request revision trigger")
            }
            WebhookDecision::Ignored { reason } => panic!("unexpected ignore: {reason}"),
        }
    }

    #[test]
    fn ignores_untrusted_comment_command() {
        let body = br#"{
            "action": "created",
            "issue": {
                "number": 8,
                "labels": []
            },
            "comment": {"body":"@netshared implement"},
            "repository": {
                "name": "rafael",
                "default_branch": "master",
                "owner": {"login":"cachebag"}
            },
            "installation": {"id": 42},
            "sender": {"login":"someone-else"}
        }"#;

        let decision = evaluate_event(&test_config(), "issue_comment", "delivery-2", body).unwrap();
        match decision {
            WebhookDecision::AcceptedIssue(_) | WebhookDecision::AcceptedPullRequestRevision(_) => {
                panic!("unexpected accept")
            }
            WebhookDecision::Ignored { reason } => {
                assert!(reason.contains("untrusted command sender"));
            }
        }
    }

    #[test]
    fn accepts_trusted_pull_request_changes_requested_review() {
        let body = br#"{
            "action": "submitted",
            "review": {"state": "changes_requested"},
            "pull_request": {
                "number": 11,
                "state": "open",
                "head": {
                    "ref": "work/issue-10-services-coding-operations-setup",
                    "repo": {"full_name": "cachebag/rafael"}
                }
            },
            "repository": {
                "name": "rafael",
                "default_branch": "master",
                "owner": {"login":"cachebag"}
            },
            "installation": {"id": 42},
            "sender": {"login":"cachebag"}
        }"#;

        let decision =
            evaluate_event(&test_config(), "pull_request_review", "delivery-3", body).unwrap();
        match decision {
            WebhookDecision::AcceptedPullRequestRevision(trigger) => {
                assert_eq!(trigger.pull_number, 11);
                assert_eq!(trigger.trigger, TriggerKind::PullRequestReview);
                assert_eq!(
                    trigger.head_branch.as_deref(),
                    Some("work/issue-10-services-coding-operations-setup")
                );
            }
            WebhookDecision::AcceptedIssue(_) => panic!("unexpected issue trigger"),
            WebhookDecision::Ignored { reason } => panic!("unexpected ignore: {reason}"),
        }
    }

    #[test]
    fn ignores_approving_pull_request_review() {
        let body = br#"{
            "action": "submitted",
            "review": {"state": "approved"},
            "pull_request": {
                "number": 11,
                "state": "open",
                "head": {
                    "ref": "work/issue-10-services-coding-operations-setup",
                    "repo": {"full_name": "cachebag/rafael"}
                }
            },
            "repository": {
                "name": "rafael",
                "default_branch": "master",
                "owner": {"login":"cachebag"}
            },
            "installation": {"id": 42},
            "sender": {"login":"cachebag"}
        }"#;

        let decision =
            evaluate_event(&test_config(), "pull_request_review", "delivery-4", body).unwrap();
        match decision {
            WebhookDecision::AcceptedIssue(_) | WebhookDecision::AcceptedPullRequestRevision(_) => {
                panic!("unexpected accept")
            }
            WebhookDecision::Ignored { reason } => {
                assert!(reason.contains("approving"));
            }
        }
    }

    #[test]
    fn ignores_pull_request_review_comment_without_revision_command() {
        let body = br#"{
            "action": "created",
            "comment": {"body":"This should be adjusted."},
            "pull_request": {
                "number": 11,
                "state": "open",
                "head": {
                    "ref": "work/issue-10-services-coding-operations-setup",
                    "repo": {"full_name": "cachebag/rafael"}
                }
            },
            "repository": {
                "name": "rafael",
                "default_branch": "master",
                "owner": {"login":"cachebag"}
            },
            "installation": {"id": 42},
            "sender": {"login":"cachebag"}
        }"#;

        let decision = evaluate_event(
            &test_config(),
            "pull_request_review_comment",
            "delivery-5",
            body,
        )
        .unwrap();
        match decision {
            WebhookDecision::AcceptedIssue(_) | WebhookDecision::AcceptedPullRequestRevision(_) => {
                panic!("unexpected accept")
            }
            WebhookDecision::Ignored { reason } => {
                assert!(reason.contains("revision command"));
            }
        }
    }

    #[test]
    fn accepts_trusted_pull_request_review_comment_revision_command() {
        let body = br#"{
            "action": "created",
            "comment": {"body":"@netshared revise\n\nPlease apply this inline feedback."},
            "pull_request": {
                "number": 11,
                "state": "open",
                "head": {
                    "ref": "work/issue-10-services-coding-operations-setup",
                    "repo": {"full_name": "cachebag/rafael"}
                }
            },
            "repository": {
                "name": "rafael",
                "default_branch": "master",
                "owner": {"login":"cachebag"}
            },
            "installation": {"id": 42},
            "sender": {"login":"cachebag"}
        }"#;

        let decision = evaluate_event(
            &test_config(),
            "pull_request_review_comment",
            "delivery-5",
            body,
        )
        .unwrap();
        match decision {
            WebhookDecision::AcceptedPullRequestRevision(trigger) => {
                assert_eq!(trigger.pull_number, 11);
                assert_eq!(trigger.trigger, TriggerKind::PullRequestReviewComment);
            }
            WebhookDecision::AcceptedIssue(_) => panic!("unexpected issue trigger"),
            WebhookDecision::Ignored { reason } => panic!("unexpected ignore: {reason}"),
        }
    }

    #[test]
    fn accepts_trusted_pull_request_conversation_revision_command() {
        let body = br#"{
            "action": "created",
            "issue": {
                "number": 11,
                "labels": [],
                "pull_request": {"url": "https://api.github.test/repos/cachebag/rafael/pulls/11"}
            },
            "comment": {"body":"@netshared revise\n\nPlease fix the unit file."},
            "repository": {
                "name": "rafael",
                "default_branch": "master",
                "owner": {"login":"cachebag"}
            },
            "installation": {"id": 42},
            "sender": {"login":"cachebag"}
        }"#;

        let decision = evaluate_event(&test_config(), "issue_comment", "delivery-6", body).unwrap();
        match decision {
            WebhookDecision::AcceptedPullRequestRevision(trigger) => {
                assert_eq!(trigger.pull_number, 11);
                assert_eq!(trigger.trigger, TriggerKind::PullRequestComment);
                assert_eq!(trigger.head_branch, None);
            }
            WebhookDecision::AcceptedIssue(_) => panic!("unexpected issue trigger"),
            WebhookDecision::Ignored { reason } => panic!("unexpected ignore: {reason}"),
        }
    }

    #[test]
    fn ignores_pull_request_conversation_comment_without_revision_command() {
        let body = br#"{
            "action": "created",
            "issue": {
                "number": 11,
                "labels": [],
                "pull_request": {"url": "https://api.github.test/repos/cachebag/rafael/pulls/11"}
            },
            "comment": {"body":"Looks close."},
            "repository": {
                "name": "rafael",
                "default_branch": "master",
                "owner": {"login":"cachebag"}
            },
            "installation": {"id": 42},
            "sender": {"login":"cachebag"}
        }"#;

        let decision = evaluate_event(&test_config(), "issue_comment", "delivery-7", body).unwrap();
        match decision {
            WebhookDecision::AcceptedIssue(_) | WebhookDecision::AcceptedPullRequestRevision(_) => {
                panic!("unexpected accept")
            }
            WebhookDecision::Ignored { reason } => {
                assert!(reason.contains("revision command"));
            }
        }
    }
}
