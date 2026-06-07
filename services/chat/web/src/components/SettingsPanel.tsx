import { useState } from "react";
import type { LucideIcon } from "lucide-react";
import { Cpu, Database, Settings, Trash2, User, X } from "lucide-react";
import { compactModelName } from "../display";
import type {
  AuthUser,
  MemoryState,
  PublicProvider,
  ThemeName,
  UpdateMemorySettingsRequest
} from "../types";
import { AccountSettings } from "./settings/AccountSettings";
import { DataSettings } from "./settings/DataSettings";
import { GeneralSettings } from "./settings/GeneralSettings";
import { MemorySettings } from "./settings/MemorySettings";
import { ModelDetails } from "./settings/ModelDetails";

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

type SettingsTab = "general" | "memory" | "models" | "account" | "data";

const settingsTabs: Array<{
  id: SettingsTab;
  label: string;
  Icon: LucideIcon;
}> = [
  { id: "general", label: "General", Icon: Settings },
  { id: "memory", label: "Memory", Icon: Database },
  { id: "models", label: "Models", Icon: Cpu },
  { id: "account", label: "Account", Icon: User },
  { id: "data", label: "Data", Icon: Trash2 }
];

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
  const [activeTab, setActiveTab] = useState<SettingsTab>("general");
  const [saving, setSaving] = useState(false);
  const [purgeConfirm, setPurgeConfirm] = useState("");
  const [error, setError] = useState<string | null>(null);
  const controlsDisabled = saving || busy;

  async function saveActiveProvider(providerId: string): Promise<void> {
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

  async function saveTheme(themeName: ThemeName): Promise<void> {
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

  async function purgeChats(): Promise<void> {
    if (purgeConfirm.trim() !== "PURGE" || conversationCount === 0) {
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
          <nav className="settings-nav" aria-label="Settings sections" role="tablist">
            {settingsTabs.map(({ id, label, Icon }) => (
              <button
                key={id}
                id={`settings-tab-${id}`}
                type="button"
                role="tab"
                aria-selected={activeTab === id}
                aria-controls={`settings-panel-${id}`}
                className={[
                  "settings-nav-button",
                  activeTab === id ? "settings-nav-button-active" : ""
                ].join(" ")}
                onClick={() => setActiveTab(id)}
              >
                <Icon aria-hidden="true" size={15} strokeWidth={2.1} />
                <span>{label}</span>
              </button>
            ))}
          </nav>

          <div
            id={`settings-panel-${activeTab}`}
            className="settings-content"
            role="tabpanel"
            tabIndex={0}
            aria-labelledby={`settings-tab-${activeTab}`}
          >
            {activeTab === "general" ? (
              <GeneralSettings
                providers={providers}
                activeProviderId={activeProviderId}
                controlsDisabled={controlsDisabled}
                theme={theme}
                onProviderChange={(providerId) => void saveActiveProvider(providerId)}
                onThemeChange={(themeName) => void saveTheme(themeName)}
              />
            ) : null}

            {activeTab === "memory" ? (
              <MemorySettings
                memory={memory}
                busy={busy}
                onMemorySettingsChange={onMemorySettingsChange}
                onMemoryChanged={onMemoryChanged}
              />
            ) : null}

            {activeTab === "models" ? (
              <ModelDetails activeProvider={activeProvider} />
            ) : null}

            {activeTab === "account" ? (
              <AccountSettings
                user={user}
                controlsDisabled={controlsDisabled}
                onLogout={onLogout}
              />
            ) : null}

            {activeTab === "data" ? (
              <DataSettings
                conversationCount={conversationCount}
                controlsDisabled={controlsDisabled}
                purgeConfirm={purgeConfirm}
                onPurgeConfirmChange={setPurgeConfirm}
                onPurgeConversations={() => void purgeChats()}
              />
            ) : null}

            {error !== null ? (
              <div className="settings-error" role="alert">
                {error}
              </div>
            ) : null}
          </div>
        </div>
      </section>
    </div>
  );
}
