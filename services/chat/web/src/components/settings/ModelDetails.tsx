import type { PublicProvider } from "../../types";
import { Detail } from "./SettingsControls";

interface ModelDetailsProps {
  activeProvider: PublicProvider | undefined;
}

export function ModelDetails({ activeProvider }: ModelDetailsProps) {
  return (
    <section className="settings-section">
      <h3 className="settings-section-title">Model details</h3>
      {activeProvider !== undefined ? (
        <div className="settings-grid settings-grid-two">
          <Detail label="Name" value={activeProvider.name} />
          <Detail label="Type" value={providerKindLabel(activeProvider)} />
          <Detail label="Endpoint" value={activeProvider.baseUrl} />
          <Detail label="Model ID" value={activeProvider.model} />
          <Detail label="System prompt" value={systemPromptLabel(activeProvider)} />
        </div>
      ) : (
        <p className="text-sm text-[var(--muted)]">No model selected.</p>
      )}
    </section>
  );
}

function systemPromptLabel(provider: PublicProvider): string {
  if (provider.systemPrompt !== undefined && provider.systemPrompt.trim() !== "") {
    return "Provider override";
  }
  if (provider.usesDefaultSystemPrompt) {
    return "Rafael default";
  }
  return "Disabled";
}

function providerKindLabel(provider: PublicProvider): string {
  if (provider.kind === "open_ai_compatible") {
    return "OpenAI compatible";
  }
  return "Anthropic";
}
