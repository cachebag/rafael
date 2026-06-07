import { useEffect, useState } from "react";
import { Check, Database, Plus, Save, Trash2 } from "lucide-react";
import {
  createMemory,
  deleteMemory,
  listMemories,
  updateMemory
} from "../../api";
import type {
  ConversationMemoryMode,
  MemoryRecord,
  MemoryState,
  MemoryStatus,
  UpdateMemoryRequest,
  UpdateMemorySettingsRequest
} from "../../types";
import { Field, SelectControl, ToggleField } from "./SettingsControls";

interface MemorySettingsProps {
  memory: MemoryState;
  busy: boolean;
  onMemorySettingsChange: (settings: UpdateMemorySettingsRequest) => Promise<MemoryState>;
  onMemoryChanged: () => Promise<void>;
}

const memoryModeOptions = [
  { value: "normal", label: "Memory" },
  { value: "no_memory", label: "No memory" }
];

const memoryFilterOptions = [
  { value: "active", label: "Active" },
  { value: "pending", label: "Pending" },
  { value: "archived", label: "Archived" },
  { value: "all", label: "All" }
];

const memoryStatusOptions = [
  { value: "active", label: "Active" },
  { value: "pending", label: "Pending" },
  { value: "archived", label: "Archived" }
];

export function MemorySettings({
  memory,
  busy,
  onMemorySettingsChange,
  onMemoryChanged
}: MemorySettingsProps) {
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [memoryState, setMemoryState] = useState(memory);
  const [memories, setMemories] = useState<MemoryRecord[]>([]);
  const [memoryQuery, setMemoryQuery] = useState("");
  const [memoryStatus, setMemoryStatus] = useState<MemoryStatus | "all">("active");
  const [memoryLoading, setMemoryLoading] = useState(false);
  const [newMemoryKind, setNewMemoryKind] = useState("preference");
  const [newMemoryContent, setNewMemoryContent] = useState("");
  const [newMemoryTags, setNewMemoryTags] = useState("");
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

  return (
    <>
      <section className="settings-section">
        <div className="settings-section-bar">
          <h3 className="settings-section-title">Memory</h3>
          <span className="settings-memory-count">
            {memoryState.counts.active} active · {memoryState.counts.pending} pending
          </span>
        </div>
        <div className="memory-toggle-grid">
          <ToggleField
            label="Enabled"
            description="Allows this user account to retrieve saved memories during normal chats."
            checked={memoryState.settings.enabled}
            disabled={controlsDisabled}
            onChange={(checked) => void updateMemorySettings({ enabled: checked })}
          />
          <ToggleField
            label="Auto capture"
            description="Lets Rafael create memories from conversations when something looks useful long term."
            checked={memoryState.settings.autoCapture}
            disabled={controlsDisabled || !memoryState.settings.enabled}
            onChange={(checked) => void updateMemorySettings({ autoCapture: checked })}
          />
          <ToggleField
            label="Require approval"
            description="Keeps captured memories pending until you approve them."
            checked={memoryState.settings.requireApproval}
            disabled={controlsDisabled || !memoryState.settings.enabled}
            onChange={(checked) =>
              void updateMemorySettings({ requireApproval: checked })
            }
          />
        </div>
        <div className="memory-policy-fields">
          <Field
            label="New chats"
            description="Default memory behavior for new conversations."
          >
            <SelectControl
              value={memoryState.settings.defaultConversationMode}
              options={memoryModeOptions}
              ariaLabel="Default memory mode for new chats"
              disabled={controlsDisabled || !memoryState.settings.enabled}
              onChange={(value) =>
                void updateMemorySettings({
                  defaultConversationMode: value as ConversationMemoryMode
                })
              }
            />
          </Field>
          <Field
            label="Context budget"
            description="Maximum characters of memory context Rafael can add to one model request."
          >
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
      </section>

      <section className="settings-section">
        <div className="settings-section-bar">
          <h3 className="settings-section-title">Manage memories</h3>
          <span className="settings-memory-count">
            {memories.length} shown
          </span>
        </div>
        <div className="memory-manager">
          <div className="memory-toolbar">
            <Field
              label="Search"
              description="Find saved memories by their text, kind, or tags."
            >
              <input
                className="control"
                value={memoryQuery}
                placeholder="Search memories"
                disabled={controlsDisabled}
                onChange={(event) => setMemoryQuery(event.target.value)}
              />
            </Field>
            <Field
              label="Status"
              description="Filter memories by whether they are active, pending review, or archived."
            >
              <SelectControl
                value={memoryStatus}
                options={memoryFilterOptions}
                ariaLabel="Memory status filter"
                disabled={controlsDisabled}
                onChange={(value) => setMemoryStatus(value as MemoryStatus | "all")}
              />
            </Field>
          </div>

          <section className="memory-subsection">
            <h4 className="memory-subsection-title">Add memory</h4>
            <div className="memory-create-row">
              <div className="memory-create-meta">
                <Field
                  label="Kind"
                  description="A short category for the memory, like preference, project, fact, or habit."
                >
                  <input
                    className="control"
                    value={newMemoryKind}
                    disabled={controlsDisabled}
                    onChange={(event) => setNewMemoryKind(event.target.value)}
                  />
                </Field>
                <Field
                  label="Tags"
                  description="Optional labels for grouping or finding related memories later."
                >
                  <input
                    className="control"
                    value={newMemoryTags}
                    placeholder="tags"
                    disabled={controlsDisabled}
                    onChange={(event) => setNewMemoryTags(event.target.value)}
                  />
                </Field>
              </div>
              <Field
                label="Memory"
                className="memory-content-field"
                description="The actual saved note Rafael may retrieve and inject when relevant."
              >
                <textarea
                  className="control memory-content-input"
                  value={newMemoryContent}
                  placeholder="New memory"
                  rows={2}
                  disabled={controlsDisabled}
                  onChange={(event) => setNewMemoryContent(event.target.value)}
                />
              </Field>
              <div className="memory-create-actions">
                <button
                  type="button"
                  className="button-secondary memory-add-button"
                  disabled={controlsDisabled || newMemoryContent.trim() === ""}
                  onClick={() => void addMemory()}
                >
                  <Plus aria-hidden="true" size={15} strokeWidth={2.1} />
                  Add memory
                </button>
              </div>
            </div>
          </section>

          <section className="memory-subsection">
            <h4 className="memory-subsection-title">Saved memories</h4>
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
          </section>
        </div>
      </section>

      {error !== null ? (
        <div className="settings-error" role="alert">
          {error}
        </div>
      ) : null}
    </>
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
          options={memoryStatusOptions}
          ariaLabel="Memory status"
          disabled={disabled}
          onChange={(value) => setStatus(value as MemoryStatus)}
        />
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

function splitTags(value: string): string[] {
  return value
    .split(/[,\s]+/)
    .map((tag) => tag.trim())
    .filter((tag) => tag !== "");
}
