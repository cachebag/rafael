import { useState } from "react";
import type { ReactNode, SelectHTMLAttributes } from "react";
import { ChevronDown, Moon, Sun, Trash2, X } from "lucide-react";
import { compactModelName } from "../display";
import {
  composeTheme,
  themeBase,
  themeMode,
  themes,
  toggledTheme,
  type ThemeBaseName
} from "../themes";
import type { PublicProvider, ThemeName } from "../types";

interface SettingsPanelProps {
  providers: PublicProvider[];
  activeProviderId: string;
  conversationCount: number;
  busy: boolean;
  theme: ThemeName;
  onClose: () => void;
  onProviderChange: (id: string) => Promise<void>;
  onThemeChange: (theme: ThemeName) => Promise<void>;
  onPurgeConversations: () => Promise<void>;
}

export function SettingsPanel({
  providers,
  activeProviderId,
  conversationCount,
  busy,
  theme,
  onClose,
  onProviderChange,
  onThemeChange,
  onPurgeConversations
}: SettingsPanelProps) {
  const activeProvider =
    providers.find((provider) => provider.id === activeProviderId) ?? providers[0];
  const [saving, setSaving] = useState(false);
  const [purgeConfirm, setPurgeConfirm] = useState("");
  const [error, setError] = useState<string | null>(null);
  const mode = themeMode(theme);
  const switchToMode = mode === "dark" ? "light" : "dark";
  const purgeReady = purgeConfirm.trim() === "PURGE";
  const controlsDisabled = saving || busy;

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
