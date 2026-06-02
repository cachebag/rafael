use std::net::SocketAddr;

use anyhow::Context;
use axum::{
    Router,
    body::Bytes,
    extract::{OriginalUri, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use serde::Serialize;
use tokio::net::TcpListener;
use tracing::{error, info, warn};

use crate::{
    config::AppConfig,
    webhook::{self, WebhookDecision},
    worker,
};

#[derive(Clone)]
struct ServerState {
    config: AppConfig,
}

pub async fn serve(config: AppConfig, bind: SocketAddr) -> anyhow::Result<()> {
    if config.github.webhook_secret.is_none() {
        anyhow::bail!("RAFAEL_GITHUB_WEBHOOK_SECRET is required for webhook server");
    }

    let app = Router::new()
        .route("/webhooks/github", post(github_webhook))
        .with_state(ServerState { config });

    let listener = TcpListener::bind(bind)
        .await
        .with_context(|| format!("failed to bind {bind}"))?;

    info!(%bind, "coding webhook server listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("webhook server failed")
}

async fn github_webhook(
    State(state): State<ServerState>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if uri.path() != "/webhooks/github" {
        return response(StatusCode::NOT_FOUND, "not found");
    }

    let signature = match required_header(&headers, "X-Hub-Signature-256") {
        Ok(value) => value,
        Err(err) => return response(StatusCode::BAD_REQUEST, err),
    };
    let event_name = match required_header(&headers, "X-GitHub-Event") {
        Ok(value) => value,
        Err(err) => return response(StatusCode::BAD_REQUEST, err),
    };
    let delivery_id = match required_header(&headers, "X-GitHub-Delivery") {
        Ok(value) => value,
        Err(err) => return response(StatusCode::BAD_REQUEST, err),
    };
    let Some(secret) = state.config.github.webhook_secret.as_deref() else {
        return response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "webhook secret not configured",
        );
    };

    if let Err(err) = webhook::verify_signature(secret, &signature, &body) {
        warn!(%event_name, %delivery_id, error = %err, "rejected webhook signature");
        return response(StatusCode::UNAUTHORIZED, "invalid signature");
    }

    match webhook::evaluate_event(&state.config, &event_name, &delivery_id, &body) {
        Ok(WebhookDecision::Accepted(trigger)) => {
            info!(
                repo = %trigger.repo,
                issue = trigger.issue_number,
                trigger = %trigger.trigger,
                run_id = %trigger.run_id,
                "accepted webhook trigger"
            );

            let config = state.config.clone();
            tokio::spawn(async move {
                if let Err(err) = worker::run_issue_triggered(config, trigger).await {
                    error!(error = %err, "coding run failed");
                }
            });

            json_response(
                StatusCode::ACCEPTED,
                WebhookResponse {
                    status: "accepted",
                    reason: None,
                },
            )
        }
        Ok(WebhookDecision::Ignored { reason }) => {
            info!(%event_name, %delivery_id, %reason, "ignored webhook trigger");
            json_response(
                StatusCode::OK,
                WebhookResponse {
                    status: "ignored",
                    reason: Some(reason),
                },
            )
        }
        Err(err) => {
            warn!(%event_name, %delivery_id, error = %err, "failed to evaluate webhook");
            response(StatusCode::BAD_REQUEST, "invalid webhook payload")
        }
    }
}

fn required_header(headers: &HeaderMap, name: &'static str) -> Result<String, &'static str> {
    headers
        .get(name)
        .ok_or("missing required GitHub webhook header")?
        .to_str()
        .map(str::to_owned)
        .map_err(|_| "invalid GitHub webhook header")
}

fn response(status: StatusCode, message: &'static str) -> Response {
    (status, message).into_response()
}

fn json_response<T: Serialize>(status: StatusCode, body: T) -> Response {
    (status, axum::Json(body)).into_response()
}

#[derive(Debug, Serialize)]
struct WebhookResponse {
    status: &'static str,
    reason: Option<String>,
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl-C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
