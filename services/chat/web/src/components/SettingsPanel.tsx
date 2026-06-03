import { useMemo, useState } from "react";
import type { ReactNode, SelectHTMLAttributes } from "react";
import { ChevronDown, Moon, Sun, X } from "lucide-react";
import { saveProvider } from "../api";
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
  ProviderKind,
  PublicProvider,
  SaveProviderRequest,
  ThemeName
} from "../types";

interface SettingsPanelProps {
  providers: PublicProvider[];
  activeProviderId: string;
  theme: ThemeName;
  onClose: () => void;
  onSaved: (provider?: PublicProvider) => Promise<void>;
  onProviderChange: (id: string) => Promise<void>;
  onThemeChange: (theme: ThemeName) => Promise<void>;
}

interface ProviderFormState {
  id?: string;
  name: string;
  kind: ProviderKind;
  baseUrl: string;
  model: string;
  apiKey: string;
  systemPrompt: string;
}

const emptyProvider: ProviderFormState = {
  name: "",
  kind: "open_ai_compatible",
  baseUrl: "",
  model: "",
  apiKey: "",
  systemPrompt: ""
};

export function SettingsPanel({
  providers,
  activeProviderId,
  theme,
  onClose,
  onSaved,
  onProviderChange,
  onThemeChange
}: SettingsPanelProps) {
  const activeProvider =
    providers.find((provider) => provider.id === activeProviderId) ?? providers[0];
  const [editingId, setEditingId] = useState<string>(activeProvider?.id ?? "new");
  const [form, setForm] = useState<ProviderFormState>(() =>
    activeProvider === undefined ? emptyProvider : providerToForm(activeProvider)
  );
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const selectedProvider = useMemo(
    () => providers.find((provider) => provider.id === editingId),
    [editingId, providers]
  );
  const mode = themeMode(theme);
  const switchToMode = mode === "dark" ? "light" : "dark";

  function chooseProvider(id: string): void {
    setEditingId(id);
    const provider = providers.find((item) => item.id === id);
    setForm(provider === undefined ? emptyProvider : providerToForm(provider));
    setError(null);
  }

  async function submit(): Promise<void> {
    setSaving(true);
    setError(null);
    try {
      const request = formToRequest(form, selectedProvider);
      const provider = await saveProvider(request);
      await onSaved(provider);
      setEditingId(provider.id);
      setForm(providerToForm(provider));
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "failed to save provider");
    } finally {
      setSaving(false);
    }
  }

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
                  disabled={saving || providers.length === 0}
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
                    disabled={saving}
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
                    disabled={saving}
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
            <div className="settings-section-heading">
              <h3 className="settings-section-title">Provider</h3>
              <SelectControl
                id="provider-edit"
                className="settings-provider-select"
                value={editingId}
                onChange={(event) => chooseProvider(event.target.value)}
              >
                {providers.map((provider) => (
                  <option key={provider.id} value={provider.id}>
                    {provider.name}
                  </option>
                ))}
                <option value="new">New provider</option>
              </SelectControl>
            </div>

            <div className="settings-grid settings-grid-two">
              <Field label="Name">
                <input
                  className="control"
                  value={form.name}
                  onChange={(event) => setForm({ ...form, name: event.target.value })}
                />
              </Field>
              <Field label="Type">
                <SelectControl
                  value={form.kind}
                  onChange={(event) =>
                    setForm({ ...form, kind: event.target.value as ProviderKind })
                  }
                >
                  <option value="open_ai_compatible">OpenAI compatible</option>
                  <option value="anthropic">Anthropic</option>
                </SelectControl>
              </Field>
              <Field label="Base URL">
                <input
                  className="control"
                  value={form.baseUrl}
                  onChange={(event) => setForm({ ...form, baseUrl: event.target.value })}
                />
              </Field>
              <Field label="Model">
                <input
                  className="control"
                  value={form.model}
                  onChange={(event) => setForm({ ...form, model: event.target.value })}
                />
              </Field>
              <Field label="API key">
                <input
                  className="control"
                  type="password"
                  value={form.apiKey}
                  placeholder={selectedProvider?.hasApiKey ? "stored" : ""}
                  onChange={(event) => setForm({ ...form, apiKey: event.target.value })}
                />
              </Field>
              <Field label="System" className="settings-field-wide">
                <textarea
                  className="control min-h-24 resize-y"
                  value={form.systemPrompt}
                  onChange={(event) => setForm({ ...form, systemPrompt: event.target.value })}
                />
              </Field>
            </div>
          </section>

          {error !== null ? (
            <div className="rounded-md border border-[var(--danger)] bg-[var(--danger-bg)] px-3 py-2 text-sm text-[var(--danger-text)]">
              {error}
            </div>
          ) : null}
        </div>

        <footer className="settings-footer">
          <button
            type="button"
            className="button-primary"
            disabled={saving}
            onClick={() => void submit()}
          >
            Save provider
          </button>
        </footer>
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

function providerToForm(provider: PublicProvider): ProviderFormState {
  return {
    id: provider.id,
    name: provider.name,
    kind: provider.kind,
    baseUrl: provider.baseUrl,
    model: provider.model,
    apiKey: "",
    systemPrompt: provider.systemPrompt ?? ""
  };
}

function formToRequest(
  form: ProviderFormState,
  selectedProvider: PublicProvider | undefined
): SaveProviderRequest {
  const apiKey = form.apiKey.trim();
  return {
    id: selectedProvider === undefined ? undefined : form.id,
    name: form.name,
    kind: form.kind,
    baseUrl: form.baseUrl,
    model: form.model,
    apiKey: apiKey === "" ? undefined : apiKey,
    systemPrompt: form.systemPrompt.trim() === "" ? undefined : form.systemPrompt
  };
}
