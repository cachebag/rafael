mod config;
mod model;
mod server;
mod store;
mod types;

use clap::{Parser, Subcommand};
use tracing::info;

use crate::config::AppConfig;

#[derive(Debug, Parser)]
#[command(version, about = "Minimal homelab chat interface")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Serve,
    CheckConfig,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let cli = Cli::parse();
    let config = AppConfig::from_env()?;

    match cli.command {
        Command::Serve => server::serve(config).await,
        Command::CheckConfig => {
            info!(
                bind = %config.bind,
                data_dir = %config.data_dir.display(),
                web_dist = %config.web_dist.display(),
                default_provider = %config.default_provider.id,
                default_model = %config.default_provider.model,
                model_list_timeout_seconds = config.model_list_timeout.as_secs(),
                "configuration loaded"
            );
            Ok(())
        }
    }
}

fn init_tracing() {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("chat=info"));

    tracing_subscriber::fmt().with_env_filter(env_filter).init();
}
