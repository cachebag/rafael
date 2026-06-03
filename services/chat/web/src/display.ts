import type { PublicProvider } from "./types";

export function compactModelName(model: string): string {
  const lastSegment = model.split("/").at(-1) ?? model;
  const [rawName, quantization] = lastSegment.split(":");
  const name = (rawName ?? lastSegment)
    .replace(/-?Instruct/gi, "")
    .replace(/-?GGUF/gi, "")
    .replace(/--+/g, "-")
    .replace(/^-|-$/g, "");

  return quantization === undefined
    ? compactText(name, 34)
    : `${compactText(name, 26)} · ${quantization}`;
}

export function providerConnectionLabel(provider: PublicProvider | null): string {
  if (provider === null) {
    return "not connected";
  }

  const host = endpointHost(provider.baseUrl);
  return host === "" ? provider.name : `${provider.name} @ ${host}`;
}

export function providerConnectionTitle(provider: PublicProvider | null): string | undefined {
  if (provider === null) {
    return undefined;
  }

  return `${provider.name}\n${provider.model}\n${provider.baseUrl}`;
}

function endpointHost(baseUrl: string): string {
  try {
    return new URL(baseUrl).host;
  } catch {
    return baseUrl.replace(/^https?:\/\//, "").split("/")[0] ?? baseUrl;
  }
}

function compactText(value: string, maxLength: number): string {
  if (value.length <= maxLength) {
    return value;
  }

  const headLength = Math.max(8, maxLength - 9);
  return `${value.slice(0, headLength)}...${value.slice(-6)}`;
}
