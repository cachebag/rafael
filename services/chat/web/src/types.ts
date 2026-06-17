export type ThemeName = "charcoal" | "charcoal_light" | "gruvbox" | "gruvbox_light";

export type ProviderKind = "open_ai_compatible" | "anthropic";

export type ChatRole = "user" | "assistant" | "system";
export type MemoryStatus = "pending" | "active" | "archived";
export type ConversationMemoryMode = "normal" | "no_memory";

export interface AuthUser {
  id: string;
  username: string;
  firstName: string;
}

export interface AuthSession {
  token: string;
  user: AuthUser;
}

export interface PublicProvider {
  id: string;
  name: string;
  kind: ProviderKind;
  baseUrl: string;
  model: string;
  hasApiKey: boolean;
  systemPrompt?: string;
  usesDefaultSystemPrompt: boolean;
  chatSupported: boolean;
}

export interface ConversationSummary {
  id: string;
  title: string;
  pinned: boolean;
  createdAt: string;
  updatedAt: string;
  messageCount: number;
}

export interface ChatMessageRecord {
  id: string;
  role: ChatRole;
  content: string;
  createdAt: string;
  providerId?: string;
  metadata?: ChatMessageMetadata;
}

export interface ChatMessageMetadata {
  toolUses?: ChatToolUse[];
  sources?: ChatSource[];
  memories?: ChatMemoryUse[];
}

export interface ChatToolUse {
  name: string;
}

export interface ChatSource {
  title?: string;
  url: string;
}

export interface Conversation {
  id: string;
  title: string;
  pinned: boolean;
  memoryMode?: ConversationMemoryMode;
  createdAt: string;
  updatedAt: string;
  messages: ChatMessageRecord[];
}

export interface ChatState {
  providers: PublicProvider[];
  activeProviderId: string;
  theme: ThemeName;
  memory: MemoryState;
  conversations: ConversationSummary[];
}

export interface SaveProviderRequest {
  id?: string;
  name: string;
  kind: ProviderKind;
  baseUrl: string;
  model: string;
  apiKey?: string;
  systemPrompt?: string;
}

export interface UpdateSettingsRequest {
  activeProviderId?: string;
  theme?: ThemeName;
}

export interface ToolActivity {
  name: string;
}

export interface ChatMemoryUse {
  id: string;
  kind: string;
  content: string;
}

export interface MemorySettings {
  enabled: boolean;
  autoCapture: boolean;
  requireApproval: boolean;
  defaultConversationMode: ConversationMemoryMode;
  memoryBudgetChars: number;
  updatedAt: string;
}

export interface MemoryCounts {
  pending: number;
  active: number;
  archived: number;
}

export interface MemoryState {
  settings: MemorySettings;
  counts: MemoryCounts;
}

export interface MemoryRecord {
  id: string;
  kind: string;
  content: string;
  status: MemoryStatus;
  tags: string[];
  sourceConversationId?: string;
  sourceMessageIds: string[];
  createdAt: string;
  updatedAt: string;
  lastUsedAt?: string;
  confidence?: number;
  userEdited: boolean;
}

export interface UpdateMemorySettingsRequest {
  enabled?: boolean;
  autoCapture?: boolean;
  requireApproval?: boolean;
  defaultConversationMode?: ConversationMemoryMode;
  memoryBudgetChars?: number;
}

export interface CreateMemoryRequest {
  kind: string;
  content: string;
  status?: MemoryStatus;
  tags?: string[];
  sourceConversationId?: string;
  sourceMessageIds?: string[];
  confidence?: number;
}

export interface UpdateMemoryRequest {
  kind?: string;
  content?: string;
  status?: MemoryStatus;
  tags?: string[];
  confidence?: number | null;
}
