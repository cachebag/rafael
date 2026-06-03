use std::{convert::Infallible, sync::Arc};

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, patch, post},
};
use chrono::Utc;
use common::{SafeComponent, slugify};
use serde::Serialize;
use tokio::{
    net::TcpListener,
    sync::{Mutex, mpsc},
};
use tokio_stream::{StreamExt, wrappers::UnboundedReceiverStream};
use tower_http::services::{ServeDir, ServeFile};
use tracing::{info, warn};

use crate::{
    config::AppConfig,
    model,
    store::{ChatStore, clean_optional, new_id},
    types::{
        ChatConfigFile, ChatMessageRecord, ChatRole, ChatStateResponse, Conversation,
        CreateConversationRequest, PublicProvider, SaveProviderRequest, SendMessageRequest,
        StoredProvider, UpdateConversationRequest, UpdateSettingsRequest,
    },
};

#[derive(Clone)]
struct ServerState {
    config: AppConfig,
    store: ChatStore,
    writes: Arc<Mutex<()>>,
}

pub async fn serve(config: AppConfig) -> anyhow::Result<()> {
    let store = ChatStore::new(config.data_dir.clone());
    let state = ServerState {
        config: config.clone(),
        store,
        writes: Arc::new(Mutex::new(())),
    };

    state
        .store
        .load_or_initialize_config(&state.config.default_provider)
        .await?;

    let api = Router::new()
        .route("/state", get(get_state))
        .route("/providers", post(save_provider))
        .route("/settings", patch(update_settings))
        .route(
            "/conversations",
            get(list_conversations).post(create_conversation),
        )
        .route(
            "/conversations/{id}",
            get(get_conversation)
                .patch(update_conversation)
                .delete(delete_conversation),
        )
        .route("/conversations/{id}/messages", post(send_message))
        .route("/conversations/{id}/messages/stream", post(stream_message))
        .with_state(state);

    let static_files = ServeDir::new(config.web_dist.clone())
        .not_found_service(ServeFile::new(config.web_dist.join("index.html")));
    let app = Router::new()
        .nest("/api", api)
        .fallback_service(static_files);
    let listener = TcpListener::bind(config.bind).await?;

    info!(
        bind = %config.bind,
        data_dir = %config.data_dir.display(),
        web_dist = %config.web_dist.display(),
        "chat server listening"
    );

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn get_state(State(state): State<ServerState>) -> Result<Json<ChatStateResponse>, ApiError> {
    let config = state.chat_config().await?;
    let conversations = state.store.list_conversations().await?;
    Ok(Json(ChatStateResponse {
        providers: config
            .providers
            .iter()
            .map(PublicProvider::from_stored)
            .collect(),
        active_provider_id: config.settings.active_provider_id,
        theme: config.settings.theme,
        conversations,
    }))
}

async fn list_conversations(
    State(state): State<ServerState>,
) -> Result<Json<Vec<crate::types::ConversationSummary>>, ApiError> {
    Ok(Json(state.store.list_conversations().await?))
}

async fn create_conversation(
    State(state): State<ServerState>,
    Json(request): Json<CreateConversationRequest>,
) -> Result<Json<Conversation>, ApiError> {
    let _guard = state.writes.lock().await;
    Ok(Json(state.store.create_conversation(request.title).await?))
}

async fn get_conversation(
    State(state): State<ServerState>,
    Path(id): Path<String>,
) -> Result<Json<Conversation>, ApiError> {
    let conversation = state
        .store
        .get_conversation(&id)
        .await?
        .ok_or_else(|| ApiError::not_found("conversation not found"))?;
    Ok(Json(conversation))
}

async fn update_conversation(
    State(state): State<ServerState>,
    Path(id): Path<String>,
    Json(request): Json<UpdateConversationRequest>,
) -> Result<Json<Conversation>, ApiError> {
    let _guard = state.writes.lock().await;
    let mut conversation = state
        .store
        .get_conversation(&id)
        .await?
        .ok_or_else(|| ApiError::not_found("conversation not found"))?;

    if let Some(pinned) = request.pinned {
        conversation.pinned = pinned;
        conversation.updated_at = Utc::now();
    }

    state.store.save_conversation(&conversation).await?;
    Ok(Json(conversation))
}

async fn delete_conversation(
    State(state): State<ServerState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let _guard = state.writes.lock().await;
    if state.store.delete_conversation(&id).await? {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Err(ApiError::not_found("conversation not found"))
    }
}

async fn send_message(
    State(state): State<ServerState>,
    Path(id): Path<String>,
    Json(request): Json<SendMessageRequest>,
) -> Result<Json<Conversation>, ApiError> {
    let content = clean_required(request.content, "message content")?;
    let _guard = state.writes.lock().await;
    let config = state.chat_config().await?;
    let provider_id = request
        .provider_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&config.settings.active_provider_id)
        .to_owned();
    let provider = config
        .providers
        .iter()
        .find(|provider| provider.id == provider_id)
        .ok_or_else(|| ApiError::bad_request("selected provider does not exist"))?;
    if !provider.kind.chat_supported() {
        return Err(ApiError::bad_request(
            "selected provider is not chat-enabled yet",
        ));
    }

    let mut conversation = state
        .store
        .get_conversation(&id)
        .await?
        .ok_or_else(|| ApiError::not_found("conversation not found"))?;
    let now = Utc::now();
    conversation.messages.push(ChatMessageRecord {
        id: new_id("msg"),
        role: ChatRole::User,
        content,
        created_at: now,
        provider_id: None,
    });
    conversation.title = conversation_title(&conversation);
    conversation.updated_at = now;
    state.store.save_conversation(&conversation).await?;

    let response =
        match model::complete_chat(provider, &conversation.messages, state.config.model_timeout)
            .await
        {
            Ok(response) => response,
            Err(err) => {
                warn!(provider_id = %provider.id, error = %err, "model request failed");
                return Err(ApiError::bad_gateway("model endpoint returned an error"));
            }
        };

    let now = Utc::now();
    conversation.messages.push(ChatMessageRecord {
        id: new_id("msg"),
        role: ChatRole::Assistant,
        content: response,
        created_at: now,
        provider_id: Some(provider.id.clone()),
    });
    conversation.updated_at = now;
    state.store.save_conversation(&conversation).await?;

    Ok(Json(conversation))
}

async fn stream_message(
    State(state): State<ServerState>,
    Path(id): Path<String>,
    Json(request): Json<SendMessageRequest>,
) -> Result<Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let content = clean_required(request.content, "message content")?;
    let provider_id = request.provider_id;
    let (tx, rx) = mpsc::unbounded_channel();

    tokio::spawn(async move {
        if let Err(err) = stream_message_worker(state, id, content, provider_id, tx.clone()).await {
            warn!(error = %err, "streaming message failed");
            let _ = tx.send(ChatStreamEvent::Error {
                error: "model endpoint returned an error".to_owned(),
            });
        }
    });

    let stream = UnboundedReceiverStream::new(rx).map(|message| Ok(stream_event(message)));
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

async fn stream_message_worker(
    state: ServerState,
    id: String,
    content: String,
    requested_provider_id: Option<String>,
    tx: mpsc::UnboundedSender<ChatStreamEvent>,
) -> anyhow::Result<()> {
    let _guard = state.writes.lock().await;
    let config = state
        .store
        .load_or_initialize_config(&state.config.default_provider)
        .await?;
    let provider_id = requested_provider_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&config.settings.active_provider_id)
        .to_owned();
    let provider = config
        .providers
        .iter()
        .find(|provider| provider.id == provider_id)
        .ok_or_else(|| anyhow::anyhow!("selected provider does not exist"))?
        .clone();
    if !provider.kind.chat_supported() {
        anyhow::bail!("selected provider is not chat-enabled yet");
    }

    let mut conversation = state
        .store
        .get_conversation(&id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("conversation not found"))?;
    let now = Utc::now();
    conversation.messages.push(ChatMessageRecord {
        id: new_id("msg"),
        role: ChatRole::User,
        content,
        created_at: now,
        provider_id: None,
    });
    conversation.title = conversation_title(&conversation);
    conversation.updated_at = now;
    state.store.save_conversation(&conversation).await?;
    let _ = tx.send(ChatStreamEvent::Conversation {
        conversation: conversation.clone(),
    });

    let assistant_content = model::stream_chat(
        &provider,
        &conversation.messages,
        state.config.model_timeout,
        |delta| {
            let _ = tx.send(ChatStreamEvent::Delta {
                content: delta.to_owned(),
            });
        },
    )
    .await?;

    let now = Utc::now();
    conversation.messages.push(ChatMessageRecord {
        id: new_id("msg"),
        role: ChatRole::Assistant,
        content: assistant_content,
        created_at: now,
        provider_id: Some(provider.id.clone()),
    });
    conversation.updated_at = now;
    state.store.save_conversation(&conversation).await?;
    let _ = tx.send(ChatStreamEvent::Conversation { conversation });
    let _ = tx.send(ChatStreamEvent::Done);

    Ok(())
}

async fn save_provider(
    State(state): State<ServerState>,
    Json(request): Json<SaveProviderRequest>,
) -> Result<Json<PublicProvider>, ApiError> {
    let _guard = state.writes.lock().await;
    let mut config = state.chat_config().await?;
    let provider = normalize_provider(request, &config)?;
    let public = PublicProvider::from_stored(&provider);

    match config
        .providers
        .iter()
        .position(|existing| existing.id == provider.id)
    {
        Some(index) => config.providers[index] = provider,
        None => config.providers.push(provider),
    }

    if !config
        .providers
        .iter()
        .any(|provider| provider.id == config.settings.active_provider_id)
    {
        config.settings.active_provider_id = public.id.clone();
    }

    state.store.save_config(&config).await?;
    Ok(Json(public))
}

async fn update_settings(
    State(state): State<ServerState>,
    Json(request): Json<UpdateSettingsRequest>,
) -> Result<Json<ChatStateResponse>, ApiError> {
    let _guard = state.writes.lock().await;
    let mut config = state.chat_config().await?;

    if let Some(active_provider_id) = request
        .active_provider_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        SafeComponent::parse(active_provider_id.to_owned())
            .map_err(|_| ApiError::bad_request("active provider id is invalid"))?;
        if !config
            .providers
            .iter()
            .any(|provider| provider.id == active_provider_id)
        {
            return Err(ApiError::bad_request("active provider does not exist"));
        }
        config.settings.active_provider_id = active_provider_id.to_owned();
    }

    if let Some(theme) = request.theme {
        config.settings.theme = theme;
    }

    state.store.save_config(&config).await?;
    let conversations = state.store.list_conversations().await?;
    Ok(Json(ChatStateResponse {
        providers: config
            .providers
            .iter()
            .map(PublicProvider::from_stored)
            .collect(),
        active_provider_id: config.settings.active_provider_id,
        theme: config.settings.theme,
        conversations,
    }))
}

impl ServerState {
    async fn chat_config(&self) -> Result<ChatConfigFile, ApiError> {
        Ok(self
            .store
            .load_or_initialize_config(&self.config.default_provider)
            .await?)
    }
}

fn normalize_provider(
    request: SaveProviderRequest,
    config: &ChatConfigFile,
) -> Result<StoredProvider, ApiError> {
    let name = clean_required(request.name, "provider name")?;
    let base_url = clean_required(request.base_url, "provider base URL")?;
    let model = clean_required(request.model, "provider model")?;
    let id = match clean_optional(request.id) {
        Some(id) => {
            SafeComponent::parse(id.clone())
                .map_err(|_| ApiError::bad_request("provider id is invalid"))?;
            id
        }
        None => unique_provider_id(&name, config),
    };
    let previous = config.providers.iter().find(|provider| provider.id == id);
    let api_key = match request.api_key {
        Some(value) => clean_optional(Some(value)),
        None => previous.and_then(|provider| provider.api_key.clone()),
    };

    Ok(StoredProvider {
        id,
        name,
        kind: request.kind,
        base_url,
        model,
        api_key,
        system_prompt: clean_optional(request.system_prompt),
    })
}

fn unique_provider_id(name: &str, config: &ChatConfigFile) -> String {
    let base = slugify(name, 40, "provider");
    if !config.providers.iter().any(|provider| provider.id == base) {
        return base;
    }

    loop {
        let candidate = format!("{base}-{}", new_id("provider"));
        if !config
            .providers
            .iter()
            .any(|provider| provider.id == candidate)
        {
            return candidate;
        }
    }
}

fn conversation_title(conversation: &Conversation) -> String {
    if conversation.title != "New conversation" || conversation.messages.is_empty() {
        return conversation.title.clone();
    }

    let first_user = conversation
        .messages
        .iter()
        .find(|message| message.role == ChatRole::User)
        .map(|message| message.content.trim())
        .filter(|content| !content.is_empty())
        .unwrap_or("New conversation");
    truncate_title(first_user)
}

fn truncate_title(value: &str) -> String {
    let mut title = value.chars().take(64).collect::<String>();
    if value.chars().count() > 64 {
        title.push_str("...");
    }
    title
}

fn clean_required(value: String, label: &'static str) -> Result<String, ApiError> {
    clean_optional(Some(value)).ok_or_else(|| ApiError::bad_request(format!("{label} is required")))
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    fn bad_gateway(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: message.into(),
        }
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        warn!(error = %error, "request failed");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: "internal server error".to_owned(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorResponse {
                error: self.message,
            }),
        )
            .into_response()
    }
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

enum ChatStreamEvent {
    Conversation { conversation: Conversation },
    Delta { content: String },
    Done,
    Error { error: String },
}

fn stream_event(message: ChatStreamEvent) -> Event {
    match message {
        ChatStreamEvent::Conversation { conversation } => {
            json_stream_event("conversation", &conversation)
        }
        ChatStreamEvent::Delta { content } => json_stream_event("delta", &DeltaEvent { content }),
        ChatStreamEvent::Done => json_stream_event("done", &DoneEvent { done: true }),
        ChatStreamEvent::Error { error } => json_stream_event("error", &ErrorResponse { error }),
    }
}

fn json_stream_event<T>(event: &'static str, value: &T) -> Event
where
    T: Serialize,
{
    match serde_json::to_string(value) {
        Ok(data) => Event::default().event(event).data(data),
        Err(err) => Event::default()
            .event("error")
            .data(serde_json::json!({ "error": err.to_string() }).to_string()),
    }
}

#[derive(Debug, Serialize)]
struct DeltaEvent {
    content: String,
}

#[derive(Debug, Serialize)]
struct DoneEvent {
    done: bool,
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
