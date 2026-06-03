import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  createConversation,
  deleteConversation,
  getConversation,
  getState,
  streamMessage,
  updateConversation,
  updateSettings
} from "./api";
import { ChatPanel } from "./components/ChatPanel";
import { SettingsPanel } from "./components/SettingsPanel";
import { Sidebar } from "./components/Sidebar";
import { applyTheme } from "./themes";
import type {
  ChatMessageRecord,
  ChatState,
  Conversation,
  PublicProvider,
  ThemeName
} from "./types";

type LoadState = "idle" | "loading" | "ready" | "failed";

export default function App() {
  const [state, setState] = useState<ChatState | null>(null);
  const [conversation, setConversation] = useState<Conversation | null>(null);
  const [selectedConversationId, setSelectedConversationId] = useState<string | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(() => isMobileViewport());
  const [sidebarMounted, setSidebarMounted] = useState(() => !isMobileViewport());
  const [isMobile, setIsMobile] = useState(() => isMobileViewport());
  const [loading, setLoading] = useState<LoadState>("idle");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const sidebarOpenFrameRef = useRef<number | null>(null);

  const activeProvider = useMemo(() => {
    if (state === null) {
      return null;
    }
    return state.providers.find((provider) => provider.id === state.activeProviderId) ?? null;
  }, [state]);
  const activeTheme = state?.theme ?? "charcoal";

  const refreshState = useCallback(async () => {
    const nextState = await getState();
    setState(nextState);
    applyTheme(nextState.theme);
    return nextState;
  }, []);

  useEffect(() => {
    let active = true;
    setLoading("loading");
    refreshState()
      .then((nextState) => {
        if (!active) {
          return;
        }
        const firstConversation = nextState.conversations[0];
        if (firstConversation !== undefined) {
          setSelectedConversationId(firstConversation.id);
        }
        setLoading("ready");
      })
      .catch((cause: unknown) => {
        if (!active) {
          return;
        }
        setError(cause instanceof Error ? cause.message : "failed to load chat");
        setLoading("failed");
      });

    return () => {
      active = false;
    };
  }, [refreshState]);

  useEffect(() => {
    const media = window.matchMedia("(max-width: 767px)");
    const update = (): void => {
      setIsMobile(media.matches);
      if (media.matches) {
        closeSidebar();
      }
    };

    update();
    media.addEventListener("change", update);
    return () => media.removeEventListener("change", update);
  }, []);

  useEffect(() => {
    if (!sidebarCollapsed) {
      setSidebarMounted(true);
      return;
    }

    const timeout = window.setTimeout(() => {
      setSidebarMounted(false);
    }, SIDEBAR_ANIMATION_MS);

    return () => window.clearTimeout(timeout);
  }, [sidebarCollapsed]);

  useEffect(() => {
    return () => {
      if (sidebarOpenFrameRef.current !== null) {
        window.cancelAnimationFrame(sidebarOpenFrameRef.current);
      }
    };
  }, []);

  useEffect(() => {
    let active = true;
    if (selectedConversationId === null) {
      setConversation(null);
      return () => {
        active = false;
      };
    }

    getConversation(selectedConversationId)
      .then((nextConversation) => {
        if (active) {
          setConversation(nextConversation);
        }
      })
      .catch((cause: unknown) => {
        if (active) {
          setError(cause instanceof Error ? cause.message : "failed to load conversation");
        }
      });

    return () => {
      active = false;
    };
  }, [selectedConversationId]);

  async function handleNewConversation(): Promise<void> {
    setBusy(true);
    setError(null);
    try {
      const nextConversation = await createConversation();
      setConversation(nextConversation);
      setSelectedConversationId(nextConversation.id);
      if (isMobile) {
        closeSidebar();
      }
      await refreshState();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "failed to create conversation");
    } finally {
      setBusy(false);
    }
  }

  async function handleDeleteConversation(id: string): Promise<void> {
    setBusy(true);
    setError(null);
    try {
      await deleteConversation(id);
      const nextState = await refreshState();
      if (selectedConversationId === id) {
        const nextConversation = nextState.conversations[0] ?? null;
        setSelectedConversationId(nextConversation?.id ?? null);
        setConversation(null);
      }
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "failed to delete conversation");
    } finally {
      setBusy(false);
    }
  }

  async function handlePinConversation(id: string, pinned: boolean): Promise<void> {
    setBusy(true);
    setError(null);
    try {
      const updatedConversation = await updateConversation(id, { pinned });
      if (selectedConversationId === id) {
        setConversation(updatedConversation);
      }
      await refreshState();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "failed to update conversation");
    } finally {
      setBusy(false);
    }
  }

  async function handleSend(content: string): Promise<void> {
    setBusy(true);
    setError(null);
    let attemptedConversationId = conversation?.id ?? null;
    try {
      const currentConversation = conversation ?? (await createConversation());
      attemptedConversationId = currentConversation.id;
      if (conversation === null) {
        setSelectedConversationId(currentConversation.id);
      }
      setConversation(optimisticConversation(currentConversation, content, activeProvider?.id));
      await streamMessage(
        currentConversation.id,
        content,
        activeProvider?.id,
        {
          onConversation: (nextConversation) => {
            setConversation(nextConversation);
            setSelectedConversationId(nextConversation.id);
          },
          onDelta: (delta) => {
            setConversation((current) =>
              appendAssistantDelta(current, currentConversation.id, delta, activeProvider?.id)
            );
          }
        }
      );
      await refreshState();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "failed to send message");
      if (attemptedConversationId !== null) {
        const updatedConversation = await getConversation(attemptedConversationId).catch(() => null);
        if (updatedConversation !== null) {
          setConversation(updatedConversation);
          setSelectedConversationId(updatedConversation.id);
          await refreshState();
        }
      }
    } finally {
      setBusy(false);
    }
  }

  function handleSelectConversation(id: string): void {
    setSelectedConversationId(id);
    if (isMobile) {
      closeSidebar();
    }
  }

  function handleOpenSettings(): void {
    setSettingsOpen(true);
    if (isMobile) {
      closeSidebar();
    }
  }

  function openSidebar(): void {
    if (sidebarOpenFrameRef.current !== null) {
      window.cancelAnimationFrame(sidebarOpenFrameRef.current);
    }

    setSidebarMounted(true);
    sidebarOpenFrameRef.current = window.requestAnimationFrame(() => {
      setSidebarCollapsed(false);
      sidebarOpenFrameRef.current = null;
    });
  }

  function closeSidebar(): void {
    if (sidebarOpenFrameRef.current !== null) {
      window.cancelAnimationFrame(sidebarOpenFrameRef.current);
      sidebarOpenFrameRef.current = null;
    }

    setSidebarCollapsed(true);
  }

  function toggleSidebar(): void {
    if (sidebarCollapsed) {
      openSidebar();
    } else {
      closeSidebar();
    }
  }

  async function handleProviderChange(providerId: string): Promise<void> {
    setError(null);
    try {
      const nextState = await updateSettings({ activeProviderId: providerId });
      setState(nextState);
      applyTheme(nextState.theme);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "failed to update provider");
      throw cause;
    }
  }

  async function handleThemeChange(theme: ThemeName): Promise<void> {
    setError(null);
    applyTheme(theme);
    try {
      const nextState = await updateSettings({ theme });
      setState(nextState);
      applyTheme(nextState.theme);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "failed to update theme");
      throw cause;
    }
  }

  async function handleProvidersChanged(provider?: PublicProvider): Promise<void> {
    const nextState = await refreshState();
    if (provider !== undefined && provider.chatSupported) {
      const updatedState = await updateSettings({ activeProviderId: provider.id });
      setState(updatedState);
      applyTheme(updatedState.theme);
      return;
    }
    setState(nextState);
  }

  return (
    <main className="app-shell min-h-dvh text-[var(--text)]">
      <div
        className={[
          "chat-layout",
          sidebarCollapsed ? "chat-layout-collapsed" : ""
        ].join(" ")}
      >
        {sidebarMounted ? (
          <button
            type="button"
            className={[
              "mobile-sidebar-backdrop md:hidden",
              sidebarCollapsed ? "mobile-sidebar-backdrop-closed" : ""
            ].join(" ")}
            aria-label="Close sidebar"
            onClick={closeSidebar}
          />
        ) : null}
        <div
          className={[
            "sidebar-frame",
            sidebarCollapsed ? "sidebar-frame-collapsed" : ""
          ].join(" ")}
        >
          {sidebarMounted ? (
            <Sidebar
              state={state}
              selectedConversationId={selectedConversationId}
              activeProvider={activeProvider}
              busy={busy}
              collapsed={sidebarCollapsed}
              theme={activeTheme}
              onNewConversation={handleNewConversation}
              onSelectConversation={handleSelectConversation}
              onDeleteConversation={handleDeleteConversation}
              onPinConversation={handlePinConversation}
              onOpenSettings={handleOpenSettings}
              onThemeChange={handleThemeChange}
              onCollapse={closeSidebar}
            />
          ) : null}
        </div>
        <ChatPanel
          conversation={conversation}
          activeProvider={activeProvider}
          busy={busy}
          error={error}
          loading={loading}
          sidebarCollapsed={sidebarCollapsed}
          onToggleSidebar={toggleSidebar}
          onSend={handleSend}
        />
      </div>
      {settingsOpen && state !== null ? (
        <SettingsPanel
          providers={state.providers}
          activeProviderId={state.activeProviderId}
          theme={state.theme}
          onClose={() => setSettingsOpen(false)}
          onSaved={handleProvidersChanged}
          onProviderChange={handleProviderChange}
          onThemeChange={handleThemeChange}
        />
      ) : null}
    </main>
  );
}

function optimisticConversation(
  conversation: Conversation,
  content: string,
  providerId: string | undefined
): Conversation {
  const now = new Date().toISOString();
  return {
    ...conversation,
    title: conversation.title === "New conversation" ? titleFromContent(content) : conversation.title,
    updatedAt: now,
    messages: [
      ...conversation.messages,
      {
        id: `pending-user-${Date.now()}`,
        role: "user",
        content,
        createdAt: now,
        providerId
      }
    ]
  };
}

function appendAssistantDelta(
  conversation: Conversation | null,
  conversationId: string,
  delta: string,
  providerId: string | undefined
): Conversation | null {
  if (conversation === null || conversation.id !== conversationId) {
    return conversation;
  }

  const messages = [...conversation.messages];
  const lastMessage = messages.at(-1);
  if (lastMessage?.id === STREAMING_MESSAGE_ID) {
    messages[messages.length - 1] = {
      ...lastMessage,
      content: lastMessage.content + delta
    };
  } else {
    messages.push(streamingAssistantMessage(delta, providerId));
  }

  return {
    ...conversation,
    updatedAt: new Date().toISOString(),
    messages
  };
}

function streamingAssistantMessage(
  content: string,
  providerId: string | undefined
): ChatMessageRecord {
  return {
    id: STREAMING_MESSAGE_ID,
    role: "assistant",
    content,
    createdAt: new Date().toISOString(),
    providerId
  };
}

function titleFromContent(content: string): string {
  const trimmed = content.trim();
  return trimmed.length > 64 ? `${trimmed.slice(0, 64)}...` : trimmed || "New conversation";
}

const STREAMING_MESSAGE_ID = "streaming-assistant";
const SIDEBAR_ANIMATION_MS = 170;

function isMobileViewport(): boolean {
  return typeof window !== "undefined" && window.matchMedia("(max-width: 767px)").matches;
}
