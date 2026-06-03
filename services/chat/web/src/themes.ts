import type { ThemeName } from "./types";

export interface ThemeOption {
  value: ThemeName;
  label: string;
}

export const themes: ThemeOption[] = [
  { value: "charcoal", label: "Charcoal" },
  { value: "gruvbox", label: "Gruvbox" }
];

export function applyTheme(theme: ThemeName): void {
  document.documentElement.dataset.theme = theme;
}
