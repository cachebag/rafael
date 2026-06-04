export type ThemeName = "charcoal" | "charcoal_light" | "gruvbox" | "gruvbox_light";

export type ProviderKind = "open_ai_compatible" | "anthropic";

export type ChatRole = "user" | "assistant" | "system";

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
  createdAt: string;
  updatedAt: string;
  messages: ChatMessageRecord[];
}

export interface ChatState {
  providers: PublicProvider[];
  activeProviderId: string;
  theme: ThemeName;
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
