import { LiftState } from "./types";

function apiUrl(path: string) {
  const base = import.meta.env.BASE_URL.replace(/\/$/, "");
  return `${base}${path}`;
}

export async function loadState(): Promise<Partial<LiftState>> {
  const response = await fetch(apiUrl("/api/state"));
  if (!response.ok) {
    throw new Error("Failed to load lift state");
  }
  return response.json();
}

export async function saveState(state: LiftState): Promise<LiftState> {
  const response = await fetch(apiUrl("/api/state"), {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(state)
  });
  if (!response.ok) {
    throw new Error("Failed to save lift state");
  }
  return response.json();
}
