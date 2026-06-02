mod change_execution;
mod config;
mod github;
mod model;
mod publish;
mod repo_context;
mod server;
mod types;
mod verification;
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
        Command::CheckConfig => {
            info!(
                repos = ?config.github.allowed_repos,
                app_slug = %config.github.app_slug,
                label = %config.github.implementation_label,
                mention = %config.github.command_mention,
                bind = %config.server.bind,
                max_run_minutes = config.workspace.max_run_minutes,
                verify_commands_count = config.workspace.verify_commands.len(),
                max_tool_iterations = config.workspace.max_tool_iterations,
                max_tool_runtime_seconds = config.workspace.max_tool_runtime_seconds,
                max_file_read_bytes = config.workspace.max_file_read_bytes,
                max_write_bytes = config.workspace.max_write_bytes,
                max_changed_files = config.workspace.max_changed_files,
                verification_command_timeout_seconds = config.workspace.verification_command_timeout_seconds,
                verification_total_timeout_seconds = config.workspace.verification_total_timeout_seconds,
                allow_unverified_publish = config.workspace.allow_unverified_publish,
                quiet_comments = config.github.quiet_comments,
                "configuration loaded"
            );
            Ok(())
        }
    }
}

fn init_tracing() {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("coding=info"));

    tracing_subscriber::fmt().with_env_filter(env_filter).init();
}
