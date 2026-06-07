import { useEffect, useMemo, useRef, useState } from "react";
import { Database, PanelLeftOpen, SendHorizontal } from "lucide-react";
import {
  compactModelName,
  providerConnectionTitle
} from "../display";
import type {
  Conversation,
  ConversationMemoryMode,
  PublicProvider,
  ToolActivity
} from "../types";
import { MessageThread } from "./MessageThread";

interface ChatPanelProps {
  conversation: Conversation | null;
  activeProvider: PublicProvider | null;
  memoryEnabled: boolean;
  memoryMode: ConversationMemoryMode;
  busy: boolean;
  toolActivity: ToolActivity | null;
  error: string | null;
  loading: "idle" | "loading" | "ready" | "failed";
  sidebarCollapsed: boolean;
  onToggleSidebar: () => void;
  onMemoryModeChange: (mode: ConversationMemoryMode) => Promise<void>;
  onSend: (content: string) => Promise<void>;
}

export function ChatPanel({
  conversation,
  activeProvider,
  memoryEnabled,
  memoryMode,
  busy,
  toolActivity,
  error,
  loading,
  sidebarCollapsed,
  onToggleSidebar,
  onMemoryModeChange,
  onSend
}: ChatPanelProps) {
  const [draft, setDraft] = useState("");
  const [followStream, setFollowStream] = useState(true);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const composerRef = useRef<HTMLTextAreaElement | null>(null);
  const wasBusyRef = useRef(false);
  const restoreComposerFocusRef = useRef(false);
  const canSend = draft.trim().length > 0 && !busy && activeProvider?.chatSupported === true;
  const modelLabel =
    activeProvider === null ? "No model selected" : compactModelName(activeProvider.model);
  const nextMemoryMode = memoryMode === "no_memory" ? "normal" : "no_memory";
  const streamPositionKey = useMemo(() => {
    const lastMessage = conversation?.messages.at(-1);
    return `${conversation?.id ?? "none"}:${lastMessage?.id ?? "none"}:${lastMessage?.content.length ?? 0}`;
  }, [conversation?.id, conversation?.messages]);

  useEffect(() => {
    const wasBusy = wasBusyRef.current;
    if (busy && !wasBusy) {
      setFollowStream(true);
    }
    if (!busy && wasBusy && restoreComposerFocusRef.current) {
      restoreComposerFocusRef.current = false;
      requestAnimationFrame(() => composerRef.current?.focus());
    }
    wasBusyRef.current = busy;
  }, [busy]);

  useEffect(() => {
    if (!followStream && busy) {
      return;
    }
    scrollToBottom(busy ? "auto" : "smooth");
  }, [busy, followStream, streamPositionKey]);

  async function submit(): Promise<void> {
    const content = draft.trim();
    if (content === "" || busy) {
      return;
    }
    restoreComposerFocusRef.current = true;
    setDraft("");
    await onSend(content);
  }

  function scrollToBottom(behavior: ScrollBehavior): void {
    requestAnimationFrame(() => {
      const scrollElement = scrollRef.current;
      if (scrollElement === null) {
        return;
      }
      if (behavior === "auto") {
        scrollElement.scrollTop = scrollElement.scrollHeight;
      } else {
        scrollElement.scrollTo({
          top: scrollElement.scrollHeight,
          behavior
        });
      }
    });
  }

  function interruptFollow(): void {
    if (busy) {
      setFollowStream(false);
    }
  }

  function updateFollowFromScrollPosition(): void {
    if (!busy) {
      return;
    }
    const scrollElement = scrollRef.current;
    if (scrollElement === null) {
      return;
    }
    if (isAtBottom(scrollElement)) {
      setFollowStream(true);
    }
  }

  function useStarterPrompt(prompt: string): void {
    if (busy || activeProvider?.chatSupported !== true) {
      setDraft(prompt);
      return;
    }

    restoreComposerFocusRef.current = true;
    void onSend(prompt);
  }

  return (
    <section className="flex h-dvh min-h-0 min-w-0 flex-col overflow-hidden">
      <header className="header-shell border-b border-[var(--line)] px-3 py-3 sm:px-5 sm:py-4">
        <div className="flex items-center justify-between gap-3 sm:gap-4">
          <div className="flex min-w-0 items-center gap-3">
            {sidebarCollapsed ? (
              <button
                type="button"
                className="icon-button icon-button-subtle"
                aria-label="Open sidebar"
                title="Open sidebar"
                onClick={onToggleSidebar}
              >
                <PanelLeftOpen aria-hidden="true" size={17} strokeWidth={2.1} />
              </button>
            ) : null}
            <div className="min-w-0">
              <h2 className="truncate text-base font-semibold">
                {conversation?.title ?? "New conversation"}
              </h2>
              <p
                className="mt-0.5 truncate text-xs text-[var(--muted)]"
                title={providerConnectionTitle(activeProvider)}
              >
                {activeProvider === null ? modelLabel : `${activeProvider.name} · ${modelLabel}`}
              </p>
            </div>
          </div>
          <button
            type="button"
            className="theme-mode-button shrink-0"
            disabled={busy || !memoryEnabled}
            title={
              !memoryEnabled
                ? "Memory is disabled"
                : memoryMode === "no_memory"
                  ? "Use memory"
                  : "Disable memory for this chat"
            }
            onClick={() => void onMemoryModeChange(nextMemoryMode)}
          >
            <Database aria-hidden="true" size={15} strokeWidth={2.1} />
            {!memoryEnabled ? "Memory off" : memoryMode === "no_memory" ? "No memory" : "Memory"}
          </button>
        </div>
      </header>

      <div className="relative min-h-0 flex-1">
        <div
          ref={scrollRef}
          className="h-full min-w-0 overflow-x-hidden overflow-y-auto px-3 py-4 sm:px-5 sm:py-7"
          onScroll={updateFollowFromScrollPosition}
          onWheel={interruptFollow}
          onPointerDown={interruptFollow}
          onTouchMove={interruptFollow}
        >
          {loading === "loading" ? (
            <p className="text-sm text-[var(--muted)]">loading</p>
          ) : conversation === null || conversation.messages.length === 0 ? (
            <StartPanel
              providerName={activeProvider?.name ?? "No provider configured"}
              disabled={busy}
              onSelectPrompt={useStarterPrompt}
            />
          ) : (
            <MessageThread
              messages={conversation.messages}
              busy={busy}
              toolActivity={toolActivity}
            />
          )}
        </div>
      </div>

      <footer className="composer-shell border-t border-[var(--line)] p-3 sm:p-5">
        <div className="grid gap-3">
          {error !== null ? (
            <div className="rounded-md border border-[var(--danger)] bg-[var(--danger-bg)] px-3 py-2 text-sm text-[var(--danger-text)]">
              {error}
            </div>
          ) : null}
          <div className="rounded-md border border-[var(--line)] bg-[var(--panel)] p-2 shadow-[var(--shadow-soft)]">
            <textarea
              ref={composerRef}
              className="composer-input max-h-28 min-h-12 w-full resize-none rounded border-0 bg-transparent px-2 py-2 text-base leading-6 text-[var(--text)] outline-none sm:text-sm"
              rows={2}
              placeholder={`Message ${activeProvider?.name ?? "rafael"}...`}
              value={draft}
              disabled={busy}
              onChange={(event) => setDraft(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter" && !event.shiftKey) {
                  event.preventDefault();
                  void submit();
                }
              }}
            />
            <div className="flex items-center justify-between gap-3 border-t border-[var(--line)] px-2 pt-2">
              <span className="truncate text-xs text-[var(--muted)]" title={activeProvider?.model}>
                {activeProvider?.chatSupported === false
                  ? "Adapter pending."
                  : modelLabel}
              </span>
              <button
                type="button"
                className="button-primary inline-flex shrink-0 items-center gap-1.5"
                disabled={!canSend}
                onClick={() => void submit()}
              >
                <SendHorizontal aria-hidden="true" size={15} strokeWidth={2.2} />
                Send
              </button>
            </div>
          </div>
        </div>
      </footer>
    </section>
  );
}

interface StartPanelProps {
  providerName: string;
  disabled: boolean;
  onSelectPrompt: (prompt: string) => void;
}

function StartPanel({ providerName, disabled, onSelectPrompt }: StartPanelProps) {
  return (
    <div className="flex min-h-full items-center justify-center">
      <div className="start-panel w-full max-w-3xl rounded-md border border-[var(--line)] bg-[var(--panel)] p-5 shadow-[var(--shadow-soft)]">
        <div className="flex flex-col gap-1 border-b border-[var(--line)] pb-4">
          <p className="text-base font-semibold text-[var(--text)]">Start a thread</p>
          <p className="text-sm text-[var(--muted)]">{providerName}</p>
        </div>
        <div className="grid gap-2 pt-4 sm:grid-cols-2">
          {starterPrompts.map((prompt) => (
            <button
              key={prompt}
              type="button"
              className="starter-prompt"
              disabled={disabled}
              onClick={() => onSelectPrompt(prompt)}
            >
              {prompt}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}

const starterPrompts = [
  "Help me think through a homelab task",
  "Draft a concise note",
  "Explain a command or error",
  "Sketch a small implementation plan"
];

function isAtBottom(element: HTMLElement): boolean {
  return Math.ceil(element.scrollTop + element.clientHeight) >= element.scrollHeight;
}
