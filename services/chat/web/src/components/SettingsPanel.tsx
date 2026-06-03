import { useMemo, useState } from "react";
import type { ReactNode } from "react";
import { saveProvider } from "../api";
import { compactModelName } from "../display";
import { themes } from "../themes";
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
    <div className="fixed inset-0 z-50 bg-black/40 p-0 sm:p-4">
      <div className="mx-auto grid h-dvh max-h-dvh w-full max-w-xl gap-4 overflow-y-auto border border-[var(--line)] bg-[var(--panel)] p-4 shadow-xl sm:h-auto sm:max-h-[calc(100dvh-2rem)] sm:rounded-md">
        <div className="flex items-center justify-between gap-4">
          <h2 className="text-base font-semibold">Settings</h2>
          <button type="button" className="button-secondary" onClick={onClose}>
            Close
          </button>
        </div>

        <div className="grid gap-3 rounded-md border border-[var(--line)] bg-[var(--panel-soft)] p-3">
          <Field label="Active model">
            <select
              className="control"
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
            </select>
          </Field>
          {activeProvider !== undefined ? (
            <p className="truncate text-xs text-[var(--muted)]" title={activeProvider.model}>
              {compactModelName(activeProvider.model)}
            </p>
          ) : null}
          <Field label="Theme">
            <select
              className="control"
              value={theme}
              disabled={saving}
              onChange={(event) => void updateTheme(event.target.value as ThemeName)}
            >
              {themes.map((themeOption) => (
                <option key={themeOption.value} value={themeOption.value}>
                  {themeOption.label}
                </option>
              ))}
            </select>
          </Field>
        </div>

        <div className="grid gap-2">
          <label className="control-label" htmlFor="provider-edit">
            Provider
          </label>
          <select
            id="provider-edit"
            className="control"
            value={editingId}
            onChange={(event) => chooseProvider(event.target.value)}
          >
            {providers.map((provider) => (
              <option key={provider.id} value={provider.id}>
                {provider.name}
              </option>
            ))}
            <option value="new">New provider</option>
          </select>
        </div>

        <div className="grid gap-3">
          <Field label="Name">
            <input
              className="control"
              value={form.name}
              onChange={(event) => setForm({ ...form, name: event.target.value })}
            />
          </Field>
          <Field label="Type">
            <select
              className="control"
              value={form.kind}
              onChange={(event) =>
                setForm({ ...form, kind: event.target.value as ProviderKind })
              }
            >
              <option value="open_ai_compatible">OpenAI compatible</option>
              <option value="anthropic">Anthropic</option>
            </select>
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
          <Field label="System">
            <textarea
              className="control min-h-20 resize-y"
              value={form.systemPrompt}
              onChange={(event) => setForm({ ...form, systemPrompt: event.target.value })}
            />
          </Field>
        </div>

        {error !== null ? (
          <div className="rounded-md border border-[var(--danger)] px-3 py-2 text-sm text-[var(--danger-text)]">
            {error}
          </div>
        ) : null}

        <div className="flex justify-end">
          <button
            type="button"
            className="button-primary"
            disabled={saving}
            onClick={() => void submit()}
          >
            Save provider
          </button>
        </div>
      </div>
    </div>
  );
}

function Field({
  label,
  children
}: {
  label: string;
  children: ReactNode;
}) {
  return (
    <label className="grid gap-2">
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
