mod config;
mod github;
mod model;
mod server;
mod types;
mod webhook;
mod worker;

use anyhow::Context;
use clap::{Parser, Subcommand};
use tracing::info;

use crate::{
    config::AppConfig,
    types::{IssueTrigger, RepoRef, TriggerKind},
};

#[derive(Debug, Parser)]
#[command(version, about = "Personal AI coding collaborator")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the GitHub App webhook receiver.
    Serve,
    /// Run the phase-one worker for an already accepted issue trigger.
    IssueTriggered(IssueTriggeredArgs),
    /// Show the latest local run state for an issue.
    Status(IssueSelectionArgs),
    /// Request cancellation for the active local run on an issue.
    Cancel(IssueSelectionArgs),
    /// Validate environment configuration and exit.
    CheckConfig,
}

#[derive(Debug, Parser)]
struct IssueTriggeredArgs {
    #[arg(long)]
    repo: RepoRef,
    #[arg(long)]
    issue: u64,
    #[arg(long, value_enum, default_value_t = TriggerKind::Manual)]
    trigger: TriggerKind,
    #[arg(long)]
    actor: Option<String>,
    #[arg(long)]
    installation_id: Option<u64>,
    #[arg(long)]
    run_id: Option<String>,
}

#[derive(Debug, Parser)]
struct IssueSelectionArgs {
    #[arg(long)]
    repo: RepoRef,
    #[arg(long)]
    issue: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let cli = Cli::parse();
    let config = AppConfig::from_env().context("failed to load configuration")?;

    match cli.command {
        Command::Serve => server::serve(config.clone(), config.server.bind).await,
        Command::IssueTriggered(args) => {
            let trigger = IssueTrigger::local(
                args.repo,
                args.issue,
                args.trigger,
                args.actor,
                args.installation_id,
                args.run_id,
            );
            worker::run_issue_triggered(config, trigger).await
        }
        Command::Status(args) => {
            let status = worker::issue_status(&config, &args.repo, args.issue).await?;
            print_issue_status(&status);
            Ok(())
        }
        Command::Cancel(args) => {
            let result = worker::request_cancel(&config, &args.repo, args.issue).await?;
            info!(
                run_id = %result.active_run.run_id,
                marker = %result.marker_path.display(),
                "cancel requested"
            );
            Ok(())
        }
        Command::CheckConfig => {
            info!(
                repos = ?config.github.allowed_repos,
                app_slug = %config.github.app_slug,
                label = %config.github.implementation_label,
                mention = %config.github.command_mention,
                bind = %config.server.bind,
                max_run_minutes = config.workspace.max_run_minutes,
                "configuration loaded"
            );
            Ok(())
        }
    }
}

fn print_issue_status(status: &worker::IssueStatus) {
    let active_run_id = status.active_run.as_ref().map(|run| run.run_id.as_str());
    let active_trigger = status
        .active_run
        .as_ref()
        .map(|run| run.trigger.to_string());
    let latest = status.latest_state.as_ref();

    info!(
        repo = %status.repo,
        issue = status.issue_number,
        issue_dir = %status.issue_dir.display(),
        cancel_requested = status.cancel_requested,
        active_run_id,
        active_trigger,
        latest_run_id = latest.map(|run| run.run_id.as_str()),
        latest_status = latest.map(|run| run.status.to_string()),
        branch = latest.and_then(|run| run.branch_name.as_deref()),
        plan = latest.and_then(|run| run.plan_path.as_ref()).map(|path| path.display().to_string()),
        error = latest.and_then(|run| run.error.as_deref()),
        pr = latest.and_then(|run| run.pr_url.as_deref()),
        "issue run status"
    );
}

fn init_tracing() {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("coding=info"));

    tracing_subscriber::fmt().with_env_filter(env_filter).init();
}
