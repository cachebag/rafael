import {
  MoreVertical,
  Moon,
  PanelLeftClose,
  Pin,
  PinOff,
  Plus,
  Settings,
  Sun,
  Trash2
} from "lucide-react";
import { useEffect, useState } from "react";
import {
  providerConnectionLabel,
  providerConnectionTitle
} from "../display";
import { themeMode, toggledTheme } from "../themes";
import type { ChatState, ConversationSummary, PublicProvider, ThemeName } from "../types";

interface SidebarProps {
  state: ChatState | null;
  selectedConversationId: string | null;
  activeProvider: PublicProvider | null;
  busy: boolean;
  collapsed: boolean;
  theme: ThemeName;
  onNewConversation: () => void;
  onSelectConversation: (id: string) => void;
  onDeleteConversation: (id: string) => void;
  onPinConversation: (id: string, pinned: boolean) => void;
  onOpenSettings: () => void;
  onThemeChange: (theme: ThemeName) => Promise<void>;
  onCollapse: () => void;
}

export function Sidebar({
  state,
  selectedConversationId,
  activeProvider,
  busy,
  collapsed,
  theme,
  onNewConversation,
  onSelectConversation,
  onDeleteConversation,
  onPinConversation,
  onOpenSettings,
  onThemeChange,
  onCollapse
}: SidebarProps) {
  const conversations = state?.conversations ?? [];
  const pinnedConversations = conversations.filter((conversation) => conversation.pinned);
  const recentConversations = conversations.filter((conversation) => !conversation.pinned);
  const [openMenuId, setOpenMenuId] = useState<string | null>(null);
  const switchToMode = themeMode(theme) === "dark" ? "light" : "dark";

  return (
    <aside
      className={[
        "sidebar-shell sidebar-drawer fixed inset-y-0 left-0 z-40 w-[min(88vw,320px)] border-r border-[var(--line)] md:sticky md:top-0 md:z-auto md:h-dvh md:w-[320px]",
        collapsed ? "sidebar-drawer-collapsed" : ""
      ].join(" ")}
      aria-hidden={collapsed}
    >
      <div className="flex h-full min-h-0 flex-col gap-4 p-4 sm:gap-5 sm:p-5">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <h1 className="text-base font-semibold">rafael</h1>
            <p
              className="mt-1 truncate text-xs text-[var(--muted)]"
              title={providerConnectionTitle(activeProvider)}
            >
              {providerConnectionLabel(activeProvider)}
            </p>
          </div>
          <div className="flex items-center gap-2">
            <button
              type="button"
              className="icon-button icon-button-subtle"
              aria-label="Collapse sidebar"
              title="Collapse sidebar"
              onClick={onCollapse}
            >
              <PanelLeftClose aria-hidden="true" size={17} strokeWidth={2.1} />
            </button>
            <button
              type="button"
              className="icon-button icon-button-subtle"
              aria-label={`Switch to ${switchToMode} mode`}
              title={`Switch to ${switchToMode} mode`}
              onClick={() => void onThemeChange(toggledTheme(theme)).catch(() => undefined)}
            >
              {switchToMode === "light" ? (
                <Sun aria-hidden="true" size={17} strokeWidth={2.1} />
              ) : (
                <Moon aria-hidden="true" size={17} strokeWidth={2.1} />
              )}
            </button>
            <button
              type="button"
              className="icon-button icon-button-subtle"
              aria-label="New conversation"
              title="New conversation"
              disabled={busy}
              onClick={onNewConversation}
            >
              <Plus aria-hidden="true" size={17} strokeWidth={2.1} />
            </button>
          </div>
        </div>

        <button
          type="button"
          className="sidebar-settings-button inline-flex w-full items-center gap-2"
          onClick={onOpenSettings}
        >
          <Settings aria-hidden="true" size={15} strokeWidth={2.1} />
          Settings
        </button>

        <div className="min-h-0 flex-1 overflow-y-auto pb-4">
          {pinnedConversations.length > 0 ? (
            <ConversationSection
              label="Pinned"
              conversations={pinnedConversations}
              selectedConversationId={selectedConversationId}
              busy={busy}
              openMenuId={openMenuId}
              onSelectConversation={onSelectConversation}
              onDeleteConversation={onDeleteConversation}
              onPinConversation={onPinConversation}
              onMenuToggle={(id) => setOpenMenuId(openMenuId === id ? null : id)}
              onMenuClose={() => setOpenMenuId(null)}
            />
          ) : null}
          <ConversationSection
            label={pinnedConversations.length > 0 ? "Recent" : "Conversations"}
            conversations={recentConversations}
            selectedConversationId={selectedConversationId}
            busy={busy}
            openMenuId={openMenuId}
            onSelectConversation={onSelectConversation}
            onDeleteConversation={onDeleteConversation}
            onPinConversation={onPinConversation}
            onMenuToggle={(id) => setOpenMenuId(openMenuId === id ? null : id)}
            onMenuClose={() => setOpenMenuId(null)}
          />
          {conversations.length === 0 ? (
            <p className="rounded-md border border-dashed border-[var(--line)] px-3 py-4 text-sm text-[var(--muted)]">
              Nothing saved yet.
            </p>
          ) : null}
        </div>
      </div>
    </aside>
  );
}

interface ConversationSectionProps {
  label: string;
  conversations: ConversationSummary[];
  selectedConversationId: string | null;
  busy: boolean;
  openMenuId: string | null;
  onSelectConversation: (id: string) => void;
  onDeleteConversation: (id: string) => void;
  onPinConversation: (id: string, pinned: boolean) => void;
  onMenuToggle: (id: string) => void;
  onMenuClose: () => void;
}

function ConversationSection({
  label,
  conversations,
  selectedConversationId,
  busy,
  openMenuId,
  onSelectConversation,
  onDeleteConversation,
  onPinConversation,
  onMenuToggle,
  onMenuClose
}: ConversationSectionProps) {
  if (conversations.length === 0) {
    return null;
  }

  return (
    <section className="mb-5 grid gap-2">
      <span className="control-label">{label}</span>
      {conversations.map((conversation) => (
        <ConversationButton
          key={conversation.id}
          conversation={conversation}
          selected={conversation.id === selectedConversationId}
          disabled={busy}
          onSelect={onSelectConversation}
          onDelete={onDeleteConversation}
          onPin={onPinConversation}
          menuOpen={openMenuId === conversation.id}
          onMenuToggle={onMenuToggle}
          onMenuClose={onMenuClose}
        />
      ))}
    </section>
  );
}

interface ConversationButtonProps {
  conversation: ConversationSummary;
  selected: boolean;
  disabled: boolean;
  menuOpen: boolean;
  onSelect: (id: string) => void;
  onDelete: (id: string) => void;
  onPin: (id: string, pinned: boolean) => void;
  onMenuToggle: (id: string) => void;
  onMenuClose: () => void;
}

function ConversationButton({
  conversation,
  selected,
  disabled,
  menuOpen,
  onSelect,
  onDelete,
  onPin,
  onMenuToggle,
  onMenuClose
}: ConversationButtonProps) {
  const [confirmingDelete, setConfirmingDelete] = useState(false);

  useEffect(() => {
    if (!menuOpen) {
      setConfirmingDelete(false);
    }
  }, [menuOpen]);

  return (
    <div
      className={[
        "conversation-row group relative grid grid-cols-[minmax(0,1fr)_32px] items-center gap-1 rounded-md transition-colors",
        selected ? "conversation-row-selected" : ""
      ].join(" ")}
    >
      <button
        type="button"
        className="min-w-0 px-2.5 py-2 text-left"
        disabled={disabled}
        onClick={() => onSelect(conversation.id)}
      >
        <span className="flex min-w-0 items-center gap-1.5">
          {conversation.pinned ? (
            <Pin
              aria-hidden="true"
              size={13}
              strokeWidth={2}
              className="shrink-0 text-[var(--accent)]"
            />
          ) : null}
          <span className="block truncate text-sm font-medium">{conversation.title}</span>
        </span>
        <span className="block text-xs text-[var(--muted)]">
          {conversation.messageCount} messages
        </span>
      </button>
      <button
        type="button"
        className="conversation-action-button mr-1 inline-flex h-7 w-7 items-center justify-center rounded"
        disabled={disabled}
        aria-label={`Open ${conversation.title} actions`}
        aria-expanded={menuOpen}
        onClick={() => onMenuToggle(conversation.id)}
      >
        <MoreVertical aria-hidden="true" size={15} strokeWidth={2} />
      </button>
      {menuOpen ? (
        <div className="conversation-menu absolute right-1 top-9 z-10 min-w-32 rounded-md border border-[var(--line)] bg-[var(--panel)] p-1 shadow-[var(--shadow-soft)]">
          {confirmingDelete ? (
            <div className="conversation-delete-confirm">
              <p>Delete this chat?</p>
              <div className="conversation-confirm-actions">
                <button
                  type="button"
                  className="conversation-confirm-button"
                  disabled={disabled}
                  onClick={() => setConfirmingDelete(false)}
                >
                  Cancel
                </button>
                <button
                  type="button"
                  className="conversation-confirm-button conversation-confirm-button-danger"
                  disabled={disabled}
                  onClick={() => {
                    onDelete(conversation.id);
                    setConfirmingDelete(false);
                    onMenuClose();
                  }}
                >
                  Delete
                </button>
              </div>
            </div>
          ) : (
            <>
              <button
                type="button"
                className="conversation-menu-item"
                disabled={disabled}
                onClick={() => {
                  onPin(conversation.id, !conversation.pinned);
                  onMenuClose();
                }}
              >
                {conversation.pinned ? (
                  <PinOff aria-hidden="true" size={14} strokeWidth={2} />
                ) : (
                  <Pin aria-hidden="true" size={14} strokeWidth={2} />
                )}
                {conversation.pinned ? "Unpin" : "Pin"}
              </button>
              <button
                type="button"
                className="conversation-menu-item conversation-menu-item-danger"
                disabled={disabled}
                onClick={() => setConfirmingDelete(true)}
              >
                <Trash2 aria-hidden="true" size={14} strokeWidth={2} />
                Delete
              </button>
            </>
          )}
        </div>
      ) : null}
    </div>
  );
}
