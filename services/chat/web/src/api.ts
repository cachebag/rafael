import type {
  AuthSession,
  AuthUser,
  ChatState,
  Conversation,
  PublicProvider,
  SaveProviderRequest,
  ToolActivity,
  UpdateSettingsRequest
} from "./types";

type JsonValue =
  | string
  | number
  | boolean
  | null
  | JsonValue[]
  | { [key: string]: JsonValue | undefined };

const AUTH_TOKEN_STORAGE_KEY = "rafael.chat.authToken";

let authToken =
  typeof window === "undefined"
    ? null
    : window.localStorage.getItem(AUTH_TOKEN_STORAGE_KEY);

export function getStoredAuthToken(): string | null {
  return authToken;
}

export function setStoredAuthToken(token: string): void {
  authToken = token;
  window.localStorage.setItem(AUTH_TOKEN_STORAGE_KEY, token);
}

export function clearStoredAuthToken(): void {
  authToken = null;
  window.localStorage.removeItem(AUTH_TOKEN_STORAGE_KEY);
}

export async function register(
  username: string,
  firstName: string,
  password: string
): Promise<AuthSession> {
  return request<AuthSession>("/api/auth/register", {
    method: "POST",
    body: { username, firstName, password },
    auth: false
  });
}

export async function login(username: string, password: string): Promise<AuthSession> {
  return request<AuthSession>("/api/auth/login", {
    method: "POST",
    body: { username, password },
    auth: false
  });
}

export async function getCurrentUser(): Promise<AuthUser> {
  return request<AuthUser>("/api/auth/me");
}

export async function getState(): Promise<ChatState> {
  return request<ChatState>("/api/state");
}

export async function createConversation(title?: string): Promise<Conversation> {
  return request<Conversation>("/api/conversations", {
    method: "POST",
    body: { title }
  });
}

export async function getConversation(id: string): Promise<Conversation> {
  return request<Conversation>(`/api/conversations/${encodeURIComponent(id)}`);
}

export async function deleteConversation(id: string): Promise<void> {
  await request<void>(`/api/conversations/${encodeURIComponent(id)}`, {
    method: "DELETE"
  });
}

export async function purgeConversations(): Promise<ChatState> {
  return request<ChatState>("/api/conversations", {
    method: "DELETE"
  });
}

export async function updateConversation(
  id: string,
  updates: { pinned?: boolean }
): Promise<Conversation> {
  return request<Conversation>(`/api/conversations/${encodeURIComponent(id)}`, {
    method: "PATCH",
    body: { pinned: updates.pinned }
  });
}

export async function sendMessage(
  conversationId: string,
  content: string,
  providerId?: string
): Promise<Conversation> {
  return request<Conversation>(
    `/api/conversations/${encodeURIComponent(conversationId)}/messages`,
    {
      method: "POST",
      body: { content, providerId }
    }
  );
}

export interface StreamMessageHandlers {
  onConversation: (conversation: Conversation) => void;
  onDelta: (content: string) => void;
  onToolActivity: (activity: ToolActivity) => void;
}

export async function streamMessage(
  conversationId: string,
  content: string,
  providerId: string | undefined,
  handlers: StreamMessageHandlers
): Promise<void> {
  const response = await fetch(
    `/api/conversations/${encodeURIComponent(conversationId)}/messages/stream`,
    {
      method: "POST",
      headers: {
        ...authHeaders(),
        "Content-Type": "application/json"
      },
      body: JSON.stringify({ content, providerId })
    }
  );

  if (!response.ok) {
    const error = await readError(response);
    throw new Error(error);
  }

  if (response.body === null) {
    throw new Error("streaming response was empty");
  }

  await readSseStream(response.body, handlers);
}

export async function saveProvider(
  provider: SaveProviderRequest
): Promise<PublicProvider> {
  return request<PublicProvider>("/api/providers", {
    method: "POST",
    body: providerToJson(provider)
  });
}

async function readSseStream(
  body: ReadableStream<Uint8Array>,
  handlers: StreamMessageHandlers
): Promise<void> {
  const reader = body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";

  for (;;) {
    const { value, done } = await reader.read();
    if (done) {
      break;
    }

    buffer += decoder.decode(value, { stream: true });
    let eventBoundary = findSseBoundary(buffer);
    while (eventBoundary !== null) {
      const block = buffer.slice(0, eventBoundary.index);
      buffer = buffer.slice(eventBoundary.index + eventBoundary.length);
      dispatchSseBlock(block, handlers);
      eventBoundary = findSseBoundary(buffer);
    }
  }

  const rest = buffer.trim();
  if (rest !== "") {
    dispatchSseBlock(rest, handlers);
  }
}

function findSseBoundary(buffer: string): { index: number; length: number } | null {
  const lf = buffer.indexOf("\n\n");
  const crlf = buffer.indexOf("\r\n\r\n");

  if (lf === -1 && crlf === -1) {
    return null;
  }
  if (lf === -1) {
    return { index: crlf, length: 4 };
  }
  if (crlf === -1 || lf < crlf) {
    return { index: lf, length: 2 };
  }
  return { index: crlf, length: 4 };
}

function dispatchSseBlock(block: string, handlers: StreamMessageHandlers): void {
  let eventName = "message";
  const dataLines: string[] = [];

  for (const line of block.split(/\r?\n/)) {
    if (line.startsWith("event:")) {
      eventName = line.slice("event:".length).trim();
    } else if (line.startsWith("data:")) {
      dataLines.push(line.slice("data:".length).trimStart());
    }
  }

  const data = dataLines.join("\n");
  if (data === "") {
    return;
  }

  if (eventName === "conversation") {
    handlers.onConversation(JSON.parse(data) as Conversation);
    return;
  }

  if (eventName === "delta") {
    const payload = JSON.parse(data) as { content?: unknown };
    if (typeof payload.content === "string") {
      handlers.onDelta(payload.content);
    }
    return;
  }

  if (eventName === "tool") {
    const payload = JSON.parse(data) as { name?: unknown };
    if (typeof payload.name === "string" && payload.name.trim() !== "") {
      handlers.onToolActivity({ name: payload.name });
    }
    return;
  }

  if (eventName === "error") {
    const payload = JSON.parse(data) as { error?: unknown };
    throw new Error(typeof payload.error === "string" ? payload.error : "stream failed");
  }
}

export async function updateSettings(
  settings: UpdateSettingsRequest
): Promise<ChatState> {
  return request<ChatState>("/api/settings", {
    method: "PATCH",
    body: settingsToJson(settings)
  });
}

async function request<T>(
  path: string,
  options: { method?: string; body?: JsonValue; auth?: boolean } = {}
): Promise<T> {
  const headers: Record<string, string> = {};
  if (options.auth !== false) {
    Object.assign(headers, authHeaders());
  }
  if (options.body !== undefined) {
    headers["Content-Type"] = "application/json";
  }

  const response = await fetch(path, {
    method: options.method ?? "GET",
    headers,
    body: options.body === undefined ? undefined : JSON.stringify(options.body)
  });

  if (!response.ok) {
    const error = await readError(response);
    throw new Error(error);
  }

  if (response.status === 204) {
    return undefined as T;
  }

  return (await response.json()) as T;
}

function authHeaders(): Record<string, string> {
  return authToken === null ? {} : { Authorization: `Bearer ${authToken}` };
}

async function readError(response: Response): Promise<string> {
  try {
    const body = (await response.json()) as { error?: unknown };
    if (typeof body.error === "string" && body.error.trim() !== "") {
      return body.error;
    }
  } catch {
    return response.statusText;
  }
  return response.statusText;
}

function providerToJson(provider: SaveProviderRequest): JsonValue {
  return {
    id: provider.id,
    name: provider.name,
    kind: provider.kind,
    baseUrl: provider.baseUrl,
    model: provider.model,
    apiKey: provider.apiKey,
    systemPrompt: provider.systemPrompt
  };
}

function settingsToJson(settings: UpdateSettingsRequest): JsonValue {
  return {
    activeProviderId: settings.activeProviderId,
    theme: settings.theme
  };
}
