use std::path::PathBuf;

use axum::Router;
use lift::{config::Config, create_app, db};
use tower_http::services::{ServeDir, ServeFile};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let config = Config::from_env();
    let pool = db::create_pool(&config.database_url)
        .await
        .expect("failed to create database pool");

    db::run_migrations(&pool)
        .await
        .expect("failed to run migrations");

    let static_dir = PathBuf::from(config.static_dir.clone());
    let root_static =
        ServeDir::new(&static_dir).not_found_service(ServeFile::new(static_dir.join("index.html")));
    let lift_static =
        ServeDir::new(&static_dir).not_found_service(ServeFile::new(static_dir.join("index.html")));
    let app = Router::new()
        .route_service("/lift", ServeFile::new(static_dir.join("index.html")))
        .route_service("/lift/", ServeFile::new(static_dir.join("index.html")))
        .merge(create_app(pool.clone()))
        .nest("/lift", create_app(pool).fallback_service(lift_static))
        .fallback_service(root_static);

    tracing::info!(bind = %config.bind, "lift server listening");

    let listener = tokio::net::TcpListener::bind(&config.bind)
        .await
        .expect("failed to bind lift server");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("lift server failed");
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install ctrl-c handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install sigterm handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("received ctrl-c, shutting down"),
        _ = terminate => tracing::info!("received sigterm, shutting down"),
    }
}
