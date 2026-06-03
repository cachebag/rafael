import type { ThemeName } from "./types";

export type ThemeBaseName = "charcoal" | "gruvbox";
export type ThemeMode = "dark" | "light";

export interface ThemeOption {
  value: ThemeBaseName;
  label: string;
}

export const themes: ThemeOption[] = [
  { value: "charcoal", label: "Charcoal" },
  { value: "gruvbox", label: "Gruvbox" }
];

const themeDetails: Record<
  ThemeName,
  {
    base: ThemeBaseName;
    mode: ThemeMode;
  }
> = {
  charcoal: { base: "charcoal", mode: "dark" },
  charcoal_light: { base: "charcoal", mode: "light" },
  gruvbox: { base: "gruvbox", mode: "dark" },
  gruvbox_light: { base: "gruvbox", mode: "light" }
};

export function applyTheme(theme: ThemeName): void {
  document.documentElement.dataset.theme = theme;
}

export function themeBase(theme: ThemeName): ThemeBaseName {
  return themeDetails[theme].base;
}

export function themeMode(theme: ThemeName): ThemeMode {
  return themeDetails[theme].mode;
}

export function composeTheme(base: ThemeBaseName, mode: ThemeMode): ThemeName {
  if (base === "charcoal") {
    return mode === "dark" ? "charcoal" : "charcoal_light";
  }

  return mode === "dark" ? "gruvbox" : "gruvbox_light";
}

export function toggledTheme(theme: ThemeName): ThemeName {
  const details = themeDetails[theme];
  return composeTheme(details.base, details.mode === "dark" ? "light" : "dark");
}
