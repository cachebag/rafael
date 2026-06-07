import { useEffect, useState } from "react";
import type { ReactNode, SelectHTMLAttributes } from "react";
import { Check, ChevronDown, Database, LogOut, Moon, Plus, Save, Sun, Trash2, X } from "lucide-react";
import {
  createMemory,
  deleteMemory,
  listMemories,
  updateMemory
} from "../api";
import { compactModelName } from "../display";
import {
  composeTheme,
  themeBase,
  themeMode,
  themes,
  toggledTheme,
  type ThemeBaseName
} from "../themes";
import type {
  AuthUser,
  ConversationMemoryMode,
  MemoryRecord,
  MemoryState,
  MemoryStatus,
  PublicProvider,
  ThemeName,
  UpdateMemoryRequest,
  UpdateMemorySettingsRequest
} from "../types";

interface SettingsPanelProps {
  providers: PublicProvider[];
  activeProviderId: string;
  user: AuthUser;
  conversationCount: number;
  memory: MemoryState;
  busy: boolean;
  theme: ThemeName;
  onClose: () => void;
  onProviderChange: (id: string) => Promise<void>;
  onThemeChange: (theme: ThemeName) => Promise<void>;
  onMemorySettingsChange: (settings: UpdateMemorySettingsRequest) => Promise<MemoryState>;
  onMemoryChanged: () => Promise<void>;
  onPurgeConversations: () => Promise<void>;
  onLogout: () => void;
}

export function SettingsPanel({
  providers,
  activeProviderId,
  user,
  conversationCount,
  memory,
  busy,
  theme,
  onClose,
  onProviderChange,
  onThemeChange,
  onMemorySettingsChange,
  onMemoryChanged,
  onPurgeConversations,
  onLogout
}: SettingsPanelProps) {
  const activeProvider =
    providers.find((provider) => provider.id === activeProviderId) ?? providers[0];
  const [saving, setSaving] = useState(false);
  const [purgeConfirm, setPurgeConfirm] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [memoryState, setMemoryState] = useState(memory);
  const [memories, setMemories] = useState<MemoryRecord[]>([]);
  const [memoryQuery, setMemoryQuery] = useState("");
  const [memoryStatus, setMemoryStatus] = useState<MemoryStatus | "all">("active");
  const [memoryLoading, setMemoryLoading] = useState(false);
  const [newMemoryKind, setNewMemoryKind] = useState("preference");
  const [newMemoryContent, setNewMemoryContent] = useState("");
  const [newMemoryTags, setNewMemoryTags] = useState("");
  const mode = themeMode(theme);
  const switchToMode = mode === "dark" ? "light" : "dark";
  const purgeReady = purgeConfirm.trim() === "PURGE";
  const controlsDisabled = saving || busy;

  useEffect(() => {
    setMemoryState(memory);
  }, [memory]);

  useEffect(() => {
    let active = true;
    setMemoryLoading(true);
    listMemories({
      query: memoryQuery,
      status: memoryStatus === "all" ? undefined : memoryStatus
    })
      .then((nextMemories) => {
        if (active) {
          setMemories(nextMemories);
        }
      })
      .catch((cause: unknown) => {
        if (active) {
          setError(cause instanceof Error ? cause.message : "failed to load memories");
        }
      })
      .finally(() => {
        if (active) {
          setMemoryLoading(false);
        }
      });

    return () => {
      active = false;
    };
  }, [memoryQuery, memoryStatus]);

  async function updateActiveProvider(providerId: string): Promise<void> {
    setSaving(true);
    setError(null);
    try {
      await onProviderChange(providerId);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "failed to update model");
    } finally {
      setSaving(false);
    }
  }

  async function updateTheme(themeName: ThemeName): Promise<void> {
    setSaving(true);
    setError(null);
    try {
      await onThemeChange(themeName);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "failed to update theme");
    } finally {
      setSaving(false);
    }
  }

  async function updateMemorySettings(
    patch: UpdateMemorySettingsRequest
  ): Promise<void> {
    setSaving(true);
    setError(null);
    try {
      const nextMemory = await onMemorySettingsChange(patch);
      setMemoryState(nextMemory);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "failed to update memory");
    } finally {
      setSaving(false);
    }
  }

  async function refreshMemories(): Promise<void> {
    const nextMemories = await listMemories({
      query: memoryQuery,
      status: memoryStatus === "all" ? undefined : memoryStatus
    });
    setMemories(nextMemories);
    await onMemoryChanged();
  }

  async function addMemory(): Promise<void> {
    const content = newMemoryContent.trim();
    if (content === "") {
      return;
    }
    setSaving(true);
    setError(null);
    try {
      await createMemory({
        kind: newMemoryKind,
        content,
        status: "active",
        tags: splitTags(newMemoryTags)
      });
      setNewMemoryContent("");
      setNewMemoryTags("");
      await refreshMemories();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "failed to save memory");
    } finally {
      setSaving(false);
    }
  }

  async function saveMemory(id: string, updates: UpdateMemoryRequest): Promise<void> {
    setSaving(true);
    setError(null);
    try {
      await updateMemory(id, updates);
      await refreshMemories();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "failed to update memory");
    } finally {
      setSaving(false);
    }
  }

  async function removeMemory(id: string): Promise<void> {
    setSaving(true);
    setError(null);
    try {
      await deleteMemory(id);
      await refreshMemories();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "failed to delete memory");
    } finally {
      setSaving(false);
    }
  }

  async function purgeChats(): Promise<void> {
    if (!purgeReady || conversationCount === 0) {
      return;
    }

    setSaving(true);
    setError(null);
    try {
      await onPurgeConversations();
      setPurgeConfirm("");
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "failed to purge chats");
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="settings-overlay">
      <section className="settings-modal" role="dialog" aria-modal="true" aria-labelledby="settings-title">
        <header className="settings-header">
          <div className="min-w-0">
            <h2 id="settings-title" className="text-base font-semibold">
              Settings
            </h2>
            {activeProvider !== undefined ? (
              <p className="mt-1 truncate text-xs text-[var(--muted)]" title={activeProvider.model}>
                {activeProvider.name} · {compactModelName(activeProvider.model)}
              </p>
            ) : null}
          </div>
          <button
            type="button"
            className="icon-button icon-button-subtle"
            aria-label="Close settings"
            title="Close settings"
            onClick={onClose}
          >
            <X aria-hidden="true" size={17} strokeWidth={2.1} />
          </button>
        </header>

        <div className="settings-body">
          <section className="settings-section">
            <h3 className="settings-section-title">Chat</h3>
            <div className="settings-grid settings-grid-two">
              <Field label="Active model">
                <SelectControl
                  value={activeProviderId}
                  disabled={controlsDisabled || providers.length === 0}
                  onChange={(event) => void updateActiveProvider(event.target.value)}
                >
                  {providers.length === 0 ? <option value="">No providers</option> : null}
                  {providers.map((provider) => (
                    <option key={provider.id} value={provider.id} disabled={!provider.chatSupported}>
                      {provider.name}
                    </option>
                  ))}
                </SelectControl>
              </Field>
              <Field label="Theme">
                <div className="grid grid-cols-[minmax(0,1fr)_auto] gap-2">
                  <SelectControl
                    value={themeBase(theme)}
                    disabled={controlsDisabled}
                    onChange={(event) =>
                      void updateTheme(
                        composeTheme(event.target.value as ThemeBaseName, mode)
                      )
                    }
                  >
                    {themes.map((themeOption) => (
                      <option key={themeOption.value} value={themeOption.value}>
                        {themeOption.label}
                      </option>
                    ))}
                  </SelectControl>
                  <button
                    type="button"
                    className="theme-mode-button"
                    disabled={controlsDisabled}
                    title={`Switch to ${switchToMode} mode`}
                    onClick={() => void updateTheme(toggledTheme(theme))}
                  >
                    {switchToMode === "light" ? (
                      <Sun aria-hidden="true" size={15} strokeWidth={2.1} />
                    ) : (
                      <Moon aria-hidden="true" size={15} strokeWidth={2.1} />
                    )}
                    {switchToMode === "light" ? "Light" : "Dark"}
                  </button>
                </div>
              </Field>
            </div>
          </section>

          <section className="settings-section">
            <div className="flex items-center justify-between gap-3">
              <h3 className="settings-section-title">Memory</h3>
              <span className="settings-memory-count">
                {memoryState.counts.active} active · {memoryState.counts.pending} pending
              </span>
            </div>
            <div className="settings-grid settings-grid-two">
              <ToggleField
                label="Enabled"
                checked={memoryState.settings.enabled}
                disabled={controlsDisabled}
                onChange={(checked) => void updateMemorySettings({ enabled: checked })}
              />
              <ToggleField
                label="Auto capture"
                checked={memoryState.settings.autoCapture}
                disabled={controlsDisabled || !memoryState.settings.enabled}
                onChange={(checked) => void updateMemorySettings({ autoCapture: checked })}
              />
              <ToggleField
                label="Require approval"
                checked={memoryState.settings.requireApproval}
                disabled={controlsDisabled || !memoryState.settings.enabled}
                onChange={(checked) =>
                  void updateMemorySettings({ requireApproval: checked })
                }
              />
              <Field label="New chats">
                <SelectControl
                  value={memoryState.settings.defaultConversationMode}
                  disabled={controlsDisabled || !memoryState.settings.enabled}
                  onChange={(event) =>
                    void updateMemorySettings({
                      defaultConversationMode: event.target.value as ConversationMemoryMode
                    })
                  }
                >
                  <option value="normal">Memory</option>
                  <option value="no_memory">No memory</option>
                </SelectControl>
              </Field>
              <Field label="Context budget">
                <input
                  className="control"
                  type="number"
                  min={512}
                  max={32768}
                  step={512}
                  value={memoryState.settings.memoryBudgetChars}
                  disabled={controlsDisabled || !memoryState.settings.enabled}
                  onChange={(event) =>
                    void updateMemorySettings({
                      memoryBudgetChars: Number(event.target.value)
                    })
                  }
                />
              </Field>
            </div>

            <div className="memory-manager">
              <div className="memory-toolbar">
                <input
                  className="control"
                  value={memoryQuery}
                  placeholder="Search memories"
                  disabled={controlsDisabled}
                  onChange={(event) => setMemoryQuery(event.target.value)}
                />
                <SelectControl
                  value={memoryStatus}
                  disabled={controlsDisabled}
                  onChange={(event) => setMemoryStatus(event.target.value as MemoryStatus | "all")}
                >
                  <option value="active">Active</option>
                  <option value="pending">Pending</option>
                  <option value="archived">Archived</option>
                  <option value="all">All</option>
                </SelectControl>
              </div>

              <div className="memory-create-row">
                <input
                  className="control"
                  value={newMemoryKind}
                  disabled={controlsDisabled}
                  onChange={(event) => setNewMemoryKind(event.target.value)}
                />
                <input
                  className="control"
                  value={newMemoryTags}
                  placeholder="tags"
                  disabled={controlsDisabled}
                  onChange={(event) => setNewMemoryTags(event.target.value)}
                />
                <textarea
                  className="control memory-content-input"
                  value={newMemoryContent}
                  placeholder="New memory"
                  rows={2}
                  disabled={controlsDisabled}
                  onChange={(event) => setNewMemoryContent(event.target.value)}
                />
                <button
                  type="button"
                  className="button-secondary memory-add-button"
                  disabled={controlsDisabled || newMemoryContent.trim() === ""}
                  onClick={() => void addMemory()}
                >
                  <Plus aria-hidden="true" size={15} strokeWidth={2.1} />
                  Add
                </button>
              </div>

              <div className="memory-list">
                {memoryLoading ? (
                  <p className="settings-account-copy">loading</p>
                ) : memories.length === 0 ? (
                  <p className="settings-account-copy">No memories.</p>
                ) : (
                  memories.map((memoryRecord) => (
                    <MemoryRow
                      key={memoryRecord.id}
                      memory={memoryRecord}
                      disabled={controlsDisabled}
                      onSave={saveMemory}
                      onDelete={removeMemory}
                    />
                  ))
                )}
              </div>
            </div>
          </section>

          <section className="settings-section">
            <h3 className="settings-section-title">Model details</h3>
            {activeProvider !== undefined ? (
              <div className="settings-grid settings-grid-two">
                <Detail label="Name" value={activeProvider.name} />
                <Detail label="Type" value={providerKindLabel(activeProvider)} />
                <Detail label="Endpoint" value={activeProvider.baseUrl} />
                <Detail label="Model ID" value={activeProvider.model} />
              </div>
            ) : (
              <p className="text-sm text-[var(--muted)]">No model selected.</p>
            )}
          </section>

          <section className="settings-section">
            <h3 className="settings-section-title">Account</h3>
            <div className="settings-account-row">
              <div className="min-w-0">
                <p className="settings-account-name">{user.firstName}</p>
                <p className="settings-account-copy">@{user.username} · signed in on this browser.</p>
              </div>
              <button
                type="button"
                className="button-secondary settings-logout-button"
                disabled={controlsDisabled}
                onClick={onLogout}
              >
                <LogOut aria-hidden="true" size={15} strokeWidth={2.1} />
                Sign out
              </button>
            </div>
          </section>

          <section className="settings-section settings-danger-section">
            <h3 className="settings-section-title">Danger zone</h3>
            <div className="settings-danger-layout">
              <div className="min-w-0">
                <p className="settings-danger-title">Purge all chats</p>
                <p className="settings-danger-copy">
                  {conversationCountLabel(conversationCount)}. Type PURGE to confirm.
                </p>
              </div>
              <div className="settings-danger-controls">
                <input
                  className="control settings-danger-input"
                  value={purgeConfirm}
                  placeholder="PURGE"
                  disabled={controlsDisabled || conversationCount === 0}
                  spellCheck={false}
                  onChange={(event) => setPurgeConfirm(event.target.value)}
                />
                <button
                  type="button"
                  className="button-danger"
                  disabled={controlsDisabled || conversationCount === 0 || !purgeReady}
                  onClick={() => void purgeChats()}
                >
                  <Trash2 aria-hidden="true" size={15} strokeWidth={2.1} />
                  Purge chats
                </button>
              </div>
            </div>
          </section>

          {error !== null ? (
            <div className="rounded-md border border-[var(--danger)] bg-[var(--danger-bg)] px-3 py-2 text-sm text-[var(--danger-text)]">
              {error}
            </div>
          ) : null}
        </div>
      </section>
    </div>
  );
}

function SelectControl({
  children,
  className,
  ...props
}: SelectHTMLAttributes<HTMLSelectElement>) {
  return (
    <span className={["select-shell", className ?? ""].join(" ")}>
      <select className="control select-control" {...props}>
        {children}
      </select>
      <ChevronDown
        aria-hidden="true"
        className="select-chevron"
        size={16}
        strokeWidth={2.1}
      />
    </span>
  );
}

function ToggleField({
  label,
  checked,
  disabled,
  onChange
}: {
  label: string;
  checked: boolean;
  disabled: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <label className="settings-toggle-row">
      <input
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(event) => onChange(event.target.checked)}
      />
      <span>{label}</span>
    </label>
  );
}

function MemoryRow({
  memory,
  disabled,
  onSave,
  onDelete
}: {
  memory: MemoryRecord;
  disabled: boolean;
  onSave: (id: string, updates: UpdateMemoryRequest) => Promise<void>;
  onDelete: (id: string) => Promise<void>;
}) {
  const [kind, setKind] = useState(memory.kind);
  const [content, setContent] = useState(memory.content);
  const [status, setStatus] = useState<MemoryStatus>(memory.status);
  const [tags, setTags] = useState(memory.tags.join(", "));

  useEffect(() => {
    setKind(memory.kind);
    setContent(memory.content);
    setStatus(memory.status);
    setTags(memory.tags.join(", "));
  }, [memory]);

  const dirty =
    kind !== memory.kind ||
    content !== memory.content ||
    status !== memory.status ||
    tags !== memory.tags.join(", ");

  return (
    <article className="memory-row">
      <div className="memory-row-header">
        <div className="min-w-0">
          <p className="memory-row-title">
            <Database aria-hidden="true" size={14} strokeWidth={2} />
            {memory.id}
          </p>
          <p className="settings-account-copy">{memory.updatedAt}</p>
        </div>
        <SelectControl
          value={status}
          disabled={disabled}
          onChange={(event) => setStatus(event.target.value as MemoryStatus)}
        >
          <option value="active">Active</option>
          <option value="pending">Pending</option>
          <option value="archived">Archived</option>
        </SelectControl>
      </div>
      <div className="memory-row-grid">
        <input
          className="control"
          value={kind}
          disabled={disabled}
          onChange={(event) => setKind(event.target.value)}
        />
        <input
          className="control"
          value={tags}
          placeholder="tags"
          disabled={disabled}
          onChange={(event) => setTags(event.target.value)}
        />
      </div>
      <textarea
        className="control memory-content-input"
        rows={3}
        value={content}
        disabled={disabled}
        onChange={(event) => setContent(event.target.value)}
      />
      <div className="memory-row-actions">
        {memory.status === "pending" ? (
          <button
            type="button"
            className="button-secondary"
            disabled={disabled}
            onClick={() => void onSave(memory.id, { status: "active" })}
          >
            <Check aria-hidden="true" size={15} strokeWidth={2.1} />
            Approve
          </button>
        ) : null}
        <button
          type="button"
          className="button-secondary"
          disabled={disabled || !dirty || content.trim() === ""}
          onClick={() =>
            void onSave(memory.id, {
              kind,
              content,
              status,
              tags: splitTags(tags)
            })
          }
        >
          <Save aria-hidden="true" size={15} strokeWidth={2.1} />
          Save
        </button>
        <button
          type="button"
          className="button-danger"
          disabled={disabled}
          onClick={() => void onDelete(memory.id)}
        >
          <Trash2 aria-hidden="true" size={15} strokeWidth={2.1} />
          Delete
        </button>
      </div>
    </article>
  );
}

function Field({
  label,
  children,
  className
}: {
  label: string;
  children: ReactNode;
  className?: string;
}) {
  return (
    <label className={["grid gap-2", className ?? ""].join(" ")}>
      <span className="control-label">{label}</span>
      {children}
    </label>
  );
}

function Detail({ label, value }: { label: string; value: string }) {
  return (
    <div className="settings-detail">
      <span className="control-label">{label}</span>
      <span className="settings-detail-value" title={value}>
        {value}
      </span>
    </div>
  );
}

function providerKindLabel(provider: PublicProvider): string {
  if (provider.kind === "open_ai_compatible") {
    return "OpenAI compatible";
  }
  return "Anthropic";
}

function conversationCountLabel(count: number): string {
  return count === 1 ? "1 saved conversation" : `${count} saved conversations`;
}

function splitTags(value: string): string[] {
  return value
    .split(/[,\s]+/)
    .map((tag) => tag.trim())
    .filter((tag) => tag !== "");
}
