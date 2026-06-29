pub mod config;
pub mod db;

use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Json, Router};
use serde_json::{json, Value};
use sqlx::{Row, SqlitePool};
use tower_http::cors::{Any, CorsLayer};

#[derive(Clone)]
pub struct AppState {
    pool: SqlitePool,
}

pub fn create_app(pool: SqlitePool) -> Router {
    let state = AppState { pool };
    let api = Router::new()
        .route("/health", get(health))
        .route("/api/state", get(get_state).put(save_state))
        .with_state(state);

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any)
        .allow_credentials(false);

    api.layer(cors)
}

async fn health() -> Json<Value> {
    Json(json!({ "ok": true }))
}

async fn get_state(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let row = sqlx::query("SELECT data FROM app_state WHERE id = 1")
        .fetch_optional(&state.pool)
        .await?;

    let value = match row {
        Some(row) => serde_json::from_str(row.get::<String, _>("data").as_str())
            .unwrap_or_else(|_| json!({ "version": 1 })),
        None => json!({ "version": 1 }),
    };

    Ok(Json(value))
}

async fn save_state(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let data = serde_json::to_string(&payload).map_err(ApiError::Json)?;

    sqlx::query(
        r#"
        INSERT INTO app_state (id, data, updated_at)
        VALUES (1, ?, datetime('now'))
        ON CONFLICT(id) DO UPDATE SET
            data = excluded.data,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(data)
    .execute(&state.pool)
    .await?;

    Ok(Json(payload))
}

#[derive(Debug)]
enum ApiError {
    Database(sqlx::Error),
    Json(serde_json::Error),
}

impl From<sqlx::Error> for ApiError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        match &self {
            Self::Database(error) => tracing::error!(error = ?error, "lift database error"),
            Self::Json(error) => tracing::error!(error = ?error, "lift json error"),
        };
        let status = StatusCode::INTERNAL_SERVER_ERROR;
        (status, Json(json!({ "error": "internal server error" }))).into_response()
    }
}
