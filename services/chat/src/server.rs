use std::{convert::Infallible, sync::Arc};

use anyhow::Context;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, patch, post},
};
use chrono::Utc;
use client::ModelInfo;
use common::{SafeComponent, slugify};
use memory::sqlite::{
    ConversationMemoryMode, MemoryListFilter, MemoryRecord, MemoryRecordPatch, MemorySettingsPatch,
    MemoryStatus, NewMemoryRecord, SqliteMemoryError, SqliteMemoryStore,
};
use serde::Serialize;
use tokio::{
    net::TcpListener,
    sync::{Mutex, mpsc},
};
use tokio_stream::{StreamExt, wrappers::UnboundedReceiverStream};
use tower_http::services::{ServeDir, ServeFile};
use tracing::{info, warn};

use crate::{
    auth::{AuthFailure, AuthSession, AuthStore},
    config::AppConfig,
    model,
    store::{ChatStore, clean_optional, new_id},
    tools::ChatToolRuntime,
    types::{
        AuthSessionResponse, ChatConfigFile, ChatMemoryUse, ChatMessageMetadata, ChatMessageRecord,
        ChatRole, ChatStateResponse, Conversation, CreateConversationRequest, CreateMemoryRequest,
        ListMemoriesQuery, LoginRequest, MemoryCounts, MemoryListResponse, MemoryStateResponse,
        ProviderKind, PublicProvider, PublicUser, RegisterRequest, SaveProviderRequest,
        SendMessageRequest, StoredProvider, UpdateConversationRequest, UpdateMemoryRequest,
        UpdateMemorySettingsRequest, UpdateSettingsRequest,
    },
};

#[derive(Clone)]
struct ServerState {
    config: AppConfig,
    auth: AuthStore,
    writes: Arc<Mutex<()>>,
    tools: Option<ChatToolRuntime>,
}

pub async fn serve(config: AppConfig) -> anyhow::Result<()> {
    let auth = AuthStore::new(config.data_dir.clone(), config.auth_token_ttl).await?;
    let state = ServerState {
        config: config.clone(),
        auth,
        writes: Arc::new(Mutex::new(())),
        tools: if config.tools.enabled() {
            Some(ChatToolRuntime::new(config.tools.clone())?)
        } else {
            None
        },
    };

    let api = Router::new()
        .route("/auth/register", post(register_user))
        .route("/auth/login", post(login_user))
        .route("/auth/me", get(get_current_user))
        .route("/state", get(get_state))
        .route("/providers", post(save_provider))
        .route("/settings", patch(update_settings))
        .route("/memory", get(list_memories).post(create_memory))
        .route("/memory/settings", patch(update_memory_settings))
        .route("/memory/{id}", patch(update_memory).delete(delete_memory))
        .route(
            "/conversations",
            get(list_conversations)
                .post(create_conversation)
                .delete(delete_conversations),
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

async fn register_user(
    State(state): State<ServerState>,
    Json(request): Json<RegisterRequest>,
) -> Result<Json<AuthSessionResponse>, ApiError> {
    let _guard = state.writes.lock().await;
    let session = state
        .auth
        .register(&request.username, &request.first_name, &request.password)
        .await?;
    Ok(Json(auth_session_response(session)))
}

async fn login_user(
    State(state): State<ServerState>,
    Json(request): Json<LoginRequest>,
) -> Result<Json<AuthSessionResponse>, ApiError> {
    let session = state
        .auth
        .login(&request.username, &request.password)
        .await?;
    Ok(Json(auth_session_response(session)))
}

async fn get_current_user(
    State(state): State<ServerState>,
    headers: HeaderMap,
) -> Result<Json<PublicUser>, ApiError> {
    Ok(Json(authenticate(&state, &headers)?))
}

async fn get_state(
    State(state): State<ServerState>,
    headers: HeaderMap,
) -> Result<Json<ChatStateResponse>, ApiError> {
    let user = authenticate(&state, &headers)?;
    let store = state.auth.user_chat_store(&user);
    let memory_store = state.auth.user_memory_store(&user).await?;
    let config = state.chat_config(&store).await?;
    let providers = state.runtime_providers(&config).await;
    let active_provider_id =
        active_provider_id(&config, &providers, &state.config.default_provider);
    let conversations = store.list_conversations().await?;
    Ok(Json(ChatStateResponse {
        providers: providers.iter().map(PublicProvider::from_stored).collect(),
        active_provider_id,
        theme: config.settings.theme,
        memory: memory_state(&memory_store).await?,
        conversations,
    }))
}

async fn list_conversations(
    State(state): State<ServerState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::types::ConversationSummary>>, ApiError> {
    let user = authenticate(&state, &headers)?;
    let store = state.auth.user_chat_store(&user);
    Ok(Json(store.list_conversations().await?))
}

async fn create_conversation(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Json(request): Json<CreateConversationRequest>,
) -> Result<Json<Conversation>, ApiError> {
    let user = authenticate(&state, &headers)?;
    let store = state.auth.user_chat_store(&user);
    let memory_store = state.auth.user_memory_store(&user).await?;
    let _guard = state.writes.lock().await;
    let mut conversation = store.create_conversation(request.title).await?;
    if let Some(memory_mode) = request.memory_mode {
        memory_store
            .set_conversation_mode(&conversation.id, memory_mode)
            .await?;
    }
    attach_memory_mode(&memory_store, &mut conversation).await?;
    Ok(Json(conversation))
}

async fn get_conversation(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Conversation>, ApiError> {
    let user = authenticate(&state, &headers)?;
    let store = state.auth.user_chat_store(&user);
    let conversation = store
        .get_conversation(&id)
        .await?
        .ok_or_else(|| ApiError::not_found("conversation not found"))?;
    let memory_store = state.auth.user_memory_store(&user).await?;
    let mut conversation = conversation;
    attach_memory_mode(&memory_store, &mut conversation).await?;
    Ok(Json(conversation))
}

async fn update_conversation(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(request): Json<UpdateConversationRequest>,
) -> Result<Json<Conversation>, ApiError> {
    let user = authenticate(&state, &headers)?;
    let store = state.auth.user_chat_store(&user);
    let memory_store = state.auth.user_memory_store(&user).await?;
    let _guard = state.writes.lock().await;
    let mut conversation = store
        .get_conversation(&id)
        .await?
        .ok_or_else(|| ApiError::not_found("conversation not found"))?;

    if let Some(pinned) = request.pinned {
        conversation.pinned = pinned;
        conversation.updated_at = Utc::now();
    }
    if let Some(memory_mode) = request.memory_mode {
        memory_store
            .set_conversation_mode(&conversation.id, memory_mode)
            .await?;
        conversation.updated_at = Utc::now();
    }

    conversation.memory_mode = None;
    store.save_conversation(&conversation).await?;
    attach_memory_mode(&memory_store, &mut conversation).await?;
    Ok(Json(conversation))
}

async fn delete_conversation(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let user = authenticate(&state, &headers)?;
    let store = state.auth.user_chat_store(&user);
    let _guard = state.writes.lock().await;
    if store.delete_conversation(&id).await? {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Err(ApiError::not_found("conversation not found"))
    }
}

async fn delete_conversations(
    State(state): State<ServerState>,
    headers: HeaderMap,
) -> Result<Json<ChatStateResponse>, ApiError> {
    let user = authenticate(&state, &headers)?;
    let store = state.auth.user_chat_store(&user);
    let memory_store = state.auth.user_memory_store(&user).await?;
    let _guard = state.writes.lock().await;
    let deleted = store.delete_all_conversations().await?;
    info!(user = %user.id, deleted, "purged chat conversations");

    let config = state.chat_config(&store).await?;
    let providers = state.runtime_providers(&config).await;
    let active_provider_id =
        active_provider_id(&config, &providers, &state.config.default_provider);

    Ok(Json(ChatStateResponse {
        providers: providers.iter().map(PublicProvider::from_stored).collect(),
        active_provider_id,
        theme: config.settings.theme,
        memory: memory_state(&memory_store).await?,
        conversations: Vec::new(),
    }))
}

async fn list_memories(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Query(query): Query<ListMemoriesQuery>,
) -> Result<Json<MemoryListResponse>, ApiError> {
    let user = authenticate(&state, &headers)?;
    let memory_store = state.auth.user_memory_store(&user).await?;
    let memories = memory_store
        .list_memories(MemoryListFilter {
            query: query.query,
            status: query.status,
            limit: None,
        })
        .await?;
    Ok(Json(MemoryListResponse { memories }))
}

async fn create_memory(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Json(request): Json<CreateMemoryRequest>,
) -> Result<Json<MemoryRecord>, ApiError> {
    let user = authenticate(&state, &headers)?;
    let memory_store = state.auth.user_memory_store(&user).await?;
    let memory = memory_store
        .create_memory(NewMemoryRecord {
            kind: request.kind,
            content: request.content,
            status: request.status.unwrap_or(MemoryStatus::Active),
            tags: request.tags,
            source_conversation_id: request.source_conversation_id,
            source_message_ids: request.source_message_ids,
            confidence: request.confidence,
        })
        .await?;
    Ok(Json(memory))
}

async fn update_memory(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(request): Json<UpdateMemoryRequest>,
) -> Result<Json<MemoryRecord>, ApiError> {
    let user = authenticate(&state, &headers)?;
    let memory_store = state.auth.user_memory_store(&user).await?;
    let memory = memory_store
        .update_memory(
            &id,
            MemoryRecordPatch {
                kind: request.kind,
                content: request.content,
                status: request.status,
                tags: request.tags,
                confidence: request.confidence,
                source_conversation_id: None,
                source_message_ids: None,
            },
        )
        .await?
        .ok_or_else(|| ApiError::not_found("memory not found"))?;
    Ok(Json(memory))
}

async fn delete_memory(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let user = authenticate(&state, &headers)?;
    let memory_store = state.auth.user_memory_store(&user).await?;
    if memory_store.delete_memory(&id).await? {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Err(ApiError::not_found("memory not found"))
    }
}

async fn update_memory_settings(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Json(request): Json<UpdateMemorySettingsRequest>,
) -> Result<Json<MemoryStateResponse>, ApiError> {
    let user = authenticate(&state, &headers)?;
    let memory_store = state.auth.user_memory_store(&user).await?;
    memory_store
        .update_settings(MemorySettingsPatch {
            enabled: request.enabled,
            auto_capture: request.auto_capture,
            require_approval: request.require_approval,
            default_conversation_mode: request.default_conversation_mode,
            memory_budget_chars: request.memory_budget_chars,
        })
        .await?;
    Ok(Json(memory_state(&memory_store).await?))
}

async fn send_message(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(request): Json<SendMessageRequest>,
) -> Result<Json<Conversation>, ApiError> {
    let content = clean_required(request.content, "message content")?;
    let user = authenticate(&state, &headers)?;
    let store = state.auth.user_chat_store(&user);
    let memory_store = state.auth.user_memory_store(&user).await?;
    let _guard = state.writes.lock().await;
    let config = state.chat_config(&store).await?;
    let providers = state.runtime_providers(&config).await;
    let default_provider_id =
        active_provider_id(&config, &providers, &state.config.default_provider);
    let provider_id = request
        .provider_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&default_provider_id)
        .to_owned();
    let provider = providers
        .iter()
        .find(|provider| provider.id == provider_id)
        .ok_or_else(|| ApiError::bad_request("selected provider does not exist"))?;
    if !provider.kind.chat_supported() {
        return Err(ApiError::bad_request(
            "selected provider is not chat-enabled yet",
        ));
    }

    let mut conversation = store
        .get_conversation(&id)
        .await?
        .ok_or_else(|| ApiError::not_found("conversation not found"))?;
    let now = Utc::now();
    let memory_query = content.clone();
    conversation.messages.push(ChatMessageRecord {
        id: new_id("msg"),
        role: ChatRole::User,
        content,
        created_at: now,
        provider_id: None,
        metadata: None,
    });
    conversation.title = conversation_title(&conversation);
    conversation.updated_at = now;
    store.save_conversation(&conversation).await?;
    let memory_selection =
        select_memory_context(&memory_store, &conversation.id, &memory_query).await?;

    let response = match model::complete_chat(
        provider,
        &conversation.messages,
        memory_selection.prompt.as_deref(),
        state.config.model_timeout,
        state.config.model_context_max_chars,
    )
    .await
    {
        Ok(response) => response,
        Err(err) => {
            warn!(provider_id = %provider.id, error = %err, "model request failed");
            return Err(ApiError::bad_gateway("model endpoint returned an error"));
        }
    };

    let now = Utc::now();
    let assistant_message_id = new_id("msg");
    let metadata = memory_metadata(&memory_selection.memories).into_option();
    conversation.messages.push(ChatMessageRecord {
        id: assistant_message_id.clone(),
        role: ChatRole::Assistant,
        content: response,
        created_at: now,
        provider_id: Some(provider.id.clone()),
        metadata,
    });
    conversation.updated_at = now;
    store.save_conversation(&conversation).await?;
    record_memory_usage(
        &memory_store,
        &memory_selection.memories,
        &conversation.id,
        &assistant_message_id,
    )
    .await?;
    spawn_memory_capture(
        memory_store,
        provider.clone(),
        conversation.clone(),
        state.config.model_timeout,
    );

    Ok(Json(conversation))
}

async fn stream_message(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(request): Json<SendMessageRequest>,
) -> Result<Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let content = clean_required(request.content, "message content")?;
    let user = authenticate(&state, &headers)?;
    let provider_id = request.provider_id;
    let (tx, rx) = mpsc::unbounded_channel();

    tokio::spawn(async move {
        if let Err(err) =
            stream_message_worker(state, user, id, content, provider_id, tx.clone()).await
        {
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
    user: PublicUser,
    id: String,
    content: String,
    requested_provider_id: Option<String>,
    tx: mpsc::UnboundedSender<ChatStreamEvent>,
) -> anyhow::Result<()> {
    let store = state.auth.user_chat_store(&user);
    let memory_store = state.auth.user_memory_store(&user).await?;
    let _guard = state.writes.lock().await;
    let config = store
        .load_or_initialize_config(&state.config.default_provider)
        .await?;
    let providers = state.runtime_providers(&config).await;
    let default_provider_id =
        active_provider_id(&config, &providers, &state.config.default_provider);
    let provider_id = requested_provider_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&default_provider_id)
        .to_owned();
    let provider = providers
        .iter()
        .find(|provider| provider.id == provider_id)
        .ok_or_else(|| anyhow::anyhow!("selected provider does not exist"))?
        .clone();
    if !provider.kind.chat_supported() {
        anyhow::bail!("selected provider is not chat-enabled yet");
    }

    let mut conversation = store
        .get_conversation(&id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("conversation not found"))?;
    let now = Utc::now();
    let memory_query = content.clone();
    conversation.messages.push(ChatMessageRecord {
        id: new_id("msg"),
        role: ChatRole::User,
        content,
        created_at: now,
        provider_id: None,
        metadata: None,
    });
    conversation.title = conversation_title(&conversation);
    conversation.updated_at = now;
    store.save_conversation(&conversation).await?;
    let _ = tx.send(ChatStreamEvent::Conversation {
        conversation: conversation.clone(),
    });
    let memory_selection =
        select_memory_context(&memory_store, &conversation.id, &memory_query).await?;

    let assistant_response = model::stream_chat(
        &provider,
        &conversation.messages,
        memory_selection.prompt.as_deref(),
        state.config.model_timeout,
        state.config.model_context_max_chars,
        state.tools.as_ref(),
        |delta| {
            let _ = tx.send(ChatStreamEvent::Delta {
                content: delta.to_owned(),
            });
        },
        |tool_name| {
            let _ = tx.send(ChatStreamEvent::Tool {
                name: tool_name.to_owned(),
            });
        },
    )
    .await?;

    let now = Utc::now();
    let assistant_message_id = new_id("msg");
    let mut metadata = assistant_response.metadata.unwrap_or_default();
    metadata.merge(memory_metadata(&memory_selection.memories));
    conversation.messages.push(ChatMessageRecord {
        id: assistant_message_id.clone(),
        role: ChatRole::Assistant,
        content: assistant_response.content,
        created_at: now,
        provider_id: Some(provider.id.clone()),
        metadata: metadata.into_option(),
    });
    conversation.updated_at = now;
    store.save_conversation(&conversation).await?;
    record_memory_usage(
        &memory_store,
        &memory_selection.memories,
        &conversation.id,
        &assistant_message_id,
    )
    .await?;
    spawn_memory_capture(
        memory_store,
        provider.clone(),
        conversation.clone(),
        state.config.model_timeout,
    );
    let _ = tx.send(ChatStreamEvent::Conversation { conversation });
    let _ = tx.send(ChatStreamEvent::Done);

    Ok(())
}

async fn save_provider(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Json(request): Json<SaveProviderRequest>,
) -> Result<Json<PublicProvider>, ApiError> {
    let user = authenticate(&state, &headers)?;
    let store = state.auth.user_chat_store(&user);
    let _guard = state.writes.lock().await;
    let mut config = state.chat_config(&store).await?;
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

    let providers = state.runtime_providers(&config).await;
    if !providers
        .iter()
        .any(|provider| provider.id == config.settings.active_provider_id)
    {
        config.settings.active_provider_id = public.id.clone();
    }

    store.save_config(&config).await?;
    Ok(Json(public))
}

async fn update_settings(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Json(request): Json<UpdateSettingsRequest>,
) -> Result<Json<ChatStateResponse>, ApiError> {
    let user = authenticate(&state, &headers)?;
    let store = state.auth.user_chat_store(&user);
    let memory_store = state.auth.user_memory_store(&user).await?;
    let _guard = state.writes.lock().await;
    let mut config = state.chat_config(&store).await?;
    let providers = state.runtime_providers(&config).await;

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
            .chain(providers.iter())
            .any(|provider| provider.id == active_provider_id)
        {
            return Err(ApiError::bad_request("active provider does not exist"));
        }
        config.settings.active_provider_id = active_provider_id.to_owned();
    }

    if let Some(theme) = request.theme {
        config.settings.theme = theme;
    }

    store.save_config(&config).await?;
    let providers = state.runtime_providers(&config).await;
    let active_provider_id =
        active_provider_id(&config, &providers, &state.config.default_provider);
    let conversations = store.list_conversations().await?;
    Ok(Json(ChatStateResponse {
        providers: providers.iter().map(PublicProvider::from_stored).collect(),
        active_provider_id,
        theme: config.settings.theme,
        memory: memory_state(&memory_store).await?,
        conversations,
    }))
}

#[derive(Debug)]
struct MemorySelection {
    prompt: Option<String>,
    memories: Vec<MemoryRecord>,
}

async fn memory_state(store: &SqliteMemoryStore) -> Result<MemoryStateResponse, SqliteMemoryError> {
    let settings = store.settings().await?;
    let memories = store
        .list_memories(MemoryListFilter {
            query: None,
            status: None,
            limit: Some(1_000),
        })
        .await?;
    let mut counts = MemoryCounts::default();
    for memory in memories {
        match memory.status {
            MemoryStatus::Pending => counts.pending += 1,
            MemoryStatus::Active => counts.active += 1,
            MemoryStatus::Archived => counts.archived += 1,
        }
    }

    Ok(MemoryStateResponse { settings, counts })
}

async fn attach_memory_mode(
    store: &SqliteMemoryStore,
    conversation: &mut Conversation,
) -> Result<(), SqliteMemoryError> {
    conversation.memory_mode = Some(store.conversation_mode(&conversation.id).await?);
    Ok(())
}

async fn select_memory_context(
    store: &SqliteMemoryStore,
    conversation_id: &str,
    query: &str,
) -> Result<MemorySelection, SqliteMemoryError> {
    let settings = store.settings().await?;
    let mode = store.conversation_mode(conversation_id).await?;
    if !settings.enabled || mode == ConversationMemoryMode::NoMemory {
        return Ok(MemorySelection {
            prompt: None,
            memories: Vec::new(),
        });
    }

    let memories = store
        .retrieve_memories(query, settings.memory_budget_chars, None)
        .await?;
    let prompt = if memories.is_empty() {
        None
    } else {
        Some(memory_prompt(&memories))
    };
    Ok(MemorySelection { prompt, memories })
}

fn memory_prompt(memories: &[MemoryRecord]) -> String {
    let mut prompt = String::from(
        "User-approved long-term memory is available below. Treat it as editable user context, not guaranteed truth. If the current chat conflicts with memory, prefer the current chat and say so briefly when relevant.\n\n",
    );
    for memory in memories {
        let tags = if memory.tags.is_empty() {
            String::new()
        } else {
            format!(" tags: {}", memory.tags.join(", "))
        };
        prompt.push_str(&format!(
            "- [{}] {}{}: {}\n",
            memory.id, memory.kind, tags, memory.content
        ));
    }
    prompt
}

fn memory_metadata(memories: &[MemoryRecord]) -> ChatMessageMetadata {
    ChatMessageMetadata {
        memories: memories
            .iter()
            .map(|memory| ChatMemoryUse {
                id: memory.id.clone(),
                kind: memory.kind.clone(),
                content: memory.content.clone(),
            })
            .collect(),
        ..ChatMessageMetadata::default()
    }
}

async fn record_memory_usage(
    store: &SqliteMemoryStore,
    memories: &[MemoryRecord],
    conversation_id: &str,
    message_id: &str,
) -> Result<(), SqliteMemoryError> {
    let memory_ids = memories
        .iter()
        .map(|memory| memory.id.clone())
        .collect::<Vec<_>>();
    store
        .record_usage(&memory_ids, conversation_id, Some(message_id))
        .await
}

fn spawn_memory_capture(
    store: SqliteMemoryStore,
    provider: StoredProvider,
    conversation: Conversation,
    timeout: std::time::Duration,
) {
    tokio::spawn(async move {
        if let Err(err) = capture_memories(store, provider, conversation, timeout).await {
            warn!(error = %err, "memory capture failed");
        }
    });
}

async fn capture_memories(
    store: SqliteMemoryStore,
    provider: StoredProvider,
    conversation: Conversation,
    timeout: std::time::Duration,
) -> anyhow::Result<()> {
    let settings = store.settings().await?;
    let mode = store.conversation_mode(&conversation.id).await?;
    if !settings.enabled || !settings.auto_capture || mode == ConversationMemoryMode::NoMemory {
        return Ok(());
    }

    let candidates = model::extract_memory_candidates(&provider, &conversation.messages, timeout)
        .await
        .context("failed to extract memory candidates")?;
    let status = if settings.require_approval {
        MemoryStatus::Pending
    } else {
        MemoryStatus::Active
    };
    let source_message_ids = conversation
        .messages
        .iter()
        .rev()
        .take(2)
        .map(|message| message.id.clone())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>();

    for candidate in candidates {
        let existing = store
            .list_memories(MemoryListFilter {
                query: Some(candidate.content.clone()),
                status: None,
                limit: Some(12),
            })
            .await?;
        if existing.iter().any(|memory| {
            memory
                .content
                .trim()
                .eq_ignore_ascii_case(candidate.content.trim())
        }) {
            continue;
        }
        store
            .create_memory(NewMemoryRecord {
                kind: candidate.kind,
                content: candidate.content,
                status,
                tags: candidate.tags,
                source_conversation_id: Some(conversation.id.clone()),
                source_message_ids: source_message_ids.clone(),
                confidence: candidate.confidence,
            })
            .await?;
    }

    Ok(())
}

impl ServerState {
    async fn chat_config(&self, store: &ChatStore) -> Result<ChatConfigFile, ApiError> {
        Ok(store
            .load_or_initialize_config(&self.config.default_provider)
            .await?)
    }

    async fn runtime_providers(&self, config: &ChatConfigFile) -> Vec<StoredProvider> {
        match model::list_models(
            &self.config.default_provider,
            self.config.model_list_timeout,
        )
        .await
        {
            Ok(models) if !models.is_empty() => {
                let mut providers = config
                    .providers
                    .iter()
                    .filter(|provider| {
                        !same_openai_endpoint(provider, &self.config.default_provider)
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                providers.extend(discovered_providers(&self.config.default_provider, models));
                providers
            }
            Ok(_) => config.providers.clone(),
            Err(err) => {
                warn!(
                    base_url = %self.config.default_provider.base_url,
                    error = %err,
                    "failed to discover model list; using saved providers"
                );
                config.providers.clone()
            }
        }
    }
}

fn discovered_providers(source: &StoredProvider, models: Vec<ModelInfo>) -> Vec<StoredProvider> {
    let mut providers = Vec::new();

    for model in models {
        let model_id = model.id.trim();
        if model_id.is_empty() {
            continue;
        }

        let id = provider_id_for_model(model_id, &providers);
        let name = model
            .name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty() && *value != model_id)
            .unwrap_or(model_id)
            .to_owned();

        providers.push(StoredProvider {
            id,
            name,
            kind: ProviderKind::OpenAiCompatible,
            base_url: source.base_url.clone(),
            model: model_id.to_owned(),
            api_key: source.api_key.clone(),
            system_prompt: source.system_prompt.clone(),
        });
    }

    providers
}

fn provider_id_for_model(model_id: &str, existing: &[StoredProvider]) -> String {
    let base = if SafeComponent::parse(model_id.to_owned()).is_ok() {
        model_id.to_owned()
    } else {
        slugify(model_id, 64, "model")
    };

    if !existing.iter().any(|provider| provider.id == base) {
        return base;
    }

    let mut index = 2;
    loop {
        let candidate = format!("{base}-{index}");
        if !existing.iter().any(|provider| provider.id == candidate) {
            return candidate;
        }
        index += 1;
    }
}

fn same_openai_endpoint(left: &StoredProvider, right: &StoredProvider) -> bool {
    left.kind == ProviderKind::OpenAiCompatible
        && right.kind == ProviderKind::OpenAiCompatible
        && normalize_base_url(&left.base_url) == normalize_base_url(&right.base_url)
}

fn normalize_base_url(value: &str) -> String {
    value.trim().trim_end_matches('/').to_owned()
}

fn active_provider_id(
    config: &ChatConfigFile,
    providers: &[StoredProvider],
    default_provider: &StoredProvider,
) -> String {
    if providers
        .iter()
        .any(|provider| provider.id == config.settings.active_provider_id)
    {
        return config.settings.active_provider_id.clone();
    }

    if providers
        .iter()
        .any(|provider| provider.id == default_provider.model)
    {
        return default_provider.model.clone();
    }

    providers
        .first()
        .map(|provider| provider.id.clone())
        .unwrap_or_else(|| config.settings.active_provider_id.clone())
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

fn auth_session_response(session: AuthSession) -> AuthSessionResponse {
    AuthSessionResponse {
        token: session.token,
        user: session.user,
    }
}

fn authenticate(state: &ServerState, headers: &HeaderMap) -> Result<PublicUser, ApiError> {
    let token = bearer_token(headers).ok_or_else(|| ApiError::unauthorized("login required"))?;
    state.auth.verify_token(token).map_err(ApiError::from)
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?.trim();
    value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))
        .map(str::trim)
        .filter(|token| !token.is_empty())
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

    fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: message.into(),
        }
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
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

impl From<AuthFailure> for ApiError {
    fn from(error: AuthFailure) -> Self {
        match error {
            AuthFailure::NotAllowed | AuthFailure::WeakPassword | AuthFailure::InvalidUsername => {
                Self::bad_request(error.user_message())
            }
            AuthFailure::AlreadyRegistered => Self::conflict(error.user_message()),
            AuthFailure::InvalidCredentials | AuthFailure::InvalidToken => {
                Self::unauthorized(error.user_message())
            }
            AuthFailure::Internal(source) => {
                warn!(error = %source, "authentication failed");
                Self {
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    message: "authentication failed".to_owned(),
                }
            }
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

impl From<SqliteMemoryError> for ApiError {
    fn from(error: SqliteMemoryError) -> Self {
        ApiError::from(anyhow::Error::new(error))
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
    Tool { name: String },
    Done,
    Error { error: String },
}

fn stream_event(message: ChatStreamEvent) -> Event {
    match message {
        ChatStreamEvent::Conversation { conversation } => {
            json_stream_event("conversation", &conversation)
        }
        ChatStreamEvent::Delta { content } => json_stream_event("delta", &DeltaEvent { content }),
        ChatStreamEvent::Tool { name } => json_stream_event("tool", &ToolEvent { name }),
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
struct ToolEvent {
    name: String,
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
