import { Moon, Sun } from "lucide-react";
import {
  composeTheme,
  themeBase,
  themeMode,
  themes,
  toggledTheme,
  type ThemeBaseName
} from "../../themes";
import type { PublicProvider, ThemeName } from "../../types";
import { Field, SelectControl } from "./SettingsControls";

interface GeneralSettingsProps {
  providers: PublicProvider[];
  activeProviderId: string;
  controlsDisabled: boolean;
  theme: ThemeName;
  onProviderChange: (id: string) => void;
  onThemeChange: (theme: ThemeName) => void;
}

export function GeneralSettings({
  providers,
  activeProviderId,
  controlsDisabled,
  theme,
  onProviderChange,
  onThemeChange
}: GeneralSettingsProps) {
  const mode = themeMode(theme);
  const switchToMode = mode === "dark" ? "light" : "dark";
  const providerOptions =
    providers.length === 0
      ? [{ value: "", label: "No providers" }]
      : providers.map((provider) => ({
          value: provider.id,
          label: provider.name,
          detail: provider.model,
          disabled: !provider.chatSupported
        }));
  const themeOptions = themes.map((themeOption) => ({
    value: themeOption.value,
    label: themeOption.label
  }));

  return (
    <section className="settings-section">
      <h3 className="settings-section-title">General</h3>
      <div className="settings-grid settings-grid-two">
        <Field label="Active model">
          <SelectControl
            value={activeProviderId}
            options={providerOptions}
            ariaLabel="Active model"
            disabled={controlsDisabled || providers.length === 0}
            onChange={onProviderChange}
          />
        </Field>
        <Field label="Theme">
          <div className="grid grid-cols-[minmax(0,1fr)_auto] gap-2">
            <SelectControl
              value={themeBase(theme)}
              options={themeOptions}
              ariaLabel="Theme"
              disabled={controlsDisabled}
              onChange={(value) =>
                onThemeChange(composeTheme(value as ThemeBaseName, mode))
              }
            />
            <button
              type="button"
              className="theme-mode-button"
              disabled={controlsDisabled}
              title={`Switch to ${switchToMode} mode`}
              onClick={() => onThemeChange(toggledTheme(theme))}
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
  );
}
