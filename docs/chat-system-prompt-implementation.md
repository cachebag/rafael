# Rafael Chat System Prompt Implementation

## Goal

Give Rafael a compact, high-leverage default system prompt that improves answer quality without pretending a local model has frontier-model reasoning ability. The prompt should be easy to inspect, easy to override, and short enough that it does not crowd out conversation history, memory, or web evidence.

This implements suggestion 1 from the quality review: use a short Rafael-specific system prompt rather than copying an entire Claude product prompt.

## Current State

The chat service already supports a per-provider `system_prompt`.

- `StoredProvider.system_prompt` exists in `services/chat/src/types.rs`.
- `RAFAEL_CHAT_SYSTEM_PROMPT` is loaded into the default provider in `services/chat/src/config.rs`.
- `model_messages` prepends `provider.system_prompt`, then memory and web system messages, then recent conversation history in `services/chat/src/model.rs`.
- Discovered llama-swap providers inherit the default provider's `system_prompt`.
- Saved providers with `system_prompt: None` currently send no base prompt.

The weakness is that there is no built-in quality prompt. A user must know to configure one, and saved providers can silently run with no behavioral guidance.

## Design Principles

- Keep the default prompt short. Target 300-600 words, not a copied product prompt.
- Keep the prompt generic enough for Qwen, Gemma, DeepSeek, and GPT-OSS style local models.
- Do not make the model roleplay omniscience. The prompt should encourage grounded uncertainty and source use.
- Preserve user override semantics. If a provider has an explicit prompt, that prompt remains authoritative.
- Make the effective prompt visible enough that quality experiments are not mysterious.

## Target Behavior

For every chat completion, Rafael chooses exactly one primary system prompt:

1. If the selected provider has a non-empty `system_prompt`, use it as the full primary prompt.
2. Otherwise, use the built-in Rafael default prompt.
3. If the built-in prompt is disabled, send no primary prompt.

Memory and web prompts remain separate system messages appended after the primary prompt, as they are operational context rather than personality or answer-quality policy.

This avoids merging two full prompts together and accidentally creating contradictory instructions.

## Default Prompt

Use this exact default prompt first. It is intentionally compact and operational.

```text
You are Rafael, a private local assistant for technical discussion, programming help, research, and practical planning.

Be direct, grounded, and useful. Prefer clear technical reasoning over polished filler. State assumptions when they matter. If a question has version, date, source, or recent-change sensitivity, use available web tools before answering. Prefer official documentation, source repositories, release notes, standards, and primary sources over blogs or summaries. When using web information, cite the source URLs.

For technical answers, identify relevant constraints, explain tradeoffs, and give concrete examples when they help. Do not bluff. If evidence is weak or conflicting, say exactly what is uncertain and what would verify it.

Match depth to the user's request. For quick questions, answer briefly. For complex design, debugging, or research questions, structure the answer, compare options, and end with a practical recommendation.
```

## Configuration

Add an explicit prompt configuration layer to `AppConfig`.

```rust
// Add this field to the existing AppConfig struct.
pub prompt: PromptConfig,

#[derive(Debug, Clone)]
pub struct PromptConfig {
    pub default_system_prompt: Option<String>,
}
```

Environment variables:

- `RAFAEL_CHAT_DEFAULT_SYSTEM_PROMPT=rafael`
  - Default value.
  - Uses the built-in prompt above.
- `RAFAEL_CHAT_DEFAULT_SYSTEM_PROMPT=none`
  - Disables the built-in prompt.
- `RAFAEL_CHAT_DEFAULT_SYSTEM_PROMPT=<literal text>`
  - Uses the literal value as the default prompt.

Keep `RAFAEL_CHAT_SYSTEM_PROMPT` for backward compatibility. It continues to populate `default_provider.system_prompt`, which means it overrides the built-in prompt for the default provider and for discovered providers that inherit it.

This keeps existing deployments stable while allowing a good no-config default.

## Backend Implementation

### Add Prompt Module

Create `services/chat/src/prompts.rs`.

Contents:

- `DEFAULT_RAFAEL_SYSTEM_PROMPT: &str`
- `PromptConfig`
- `default_system_prompt_from_env(value: Option<String>) -> Option<String>`
- `effective_system_prompt(provider: &StoredProvider, prompt_config: &PromptConfig) -> Option<String>`

Rules for `effective_system_prompt`:

```rust
if provider.system_prompt.trim() is non-empty:
    return provider.system_prompt.trim().to_owned()
else:
    return prompt_config.default_system_prompt.clone()
```

Reject empty strings after trimming.

### Wire Config

In `services/chat/src/config.rs`:

1. Read `RAFAEL_CHAT_DEFAULT_SYSTEM_PROMPT`.
2. Convert it with `default_system_prompt_from_env`.
3. Store the result in `AppConfig.prompt`.
4. Keep current `RAFAEL_CHAT_SYSTEM_PROMPT` behavior unchanged.

Expected parsing:

- Missing env: built-in Rafael prompt.
- Empty env: built-in Rafael prompt. Empty env should not accidentally disable quality guidance.
- `none`, `off`, `disabled`: `None`.
- Any other value: trimmed literal prompt.

### Wire Model Calls

Update these model functions to accept prompt config:

- `complete_chat`
- `stream_chat`
- `openai_compatible_chat`
- `openai_compatible_stream_chat`
- `openai_compatible_stream_chat_with_tools`
- `model_messages`

Preferred function shape:

```rust
fn model_messages(
    provider: &StoredProvider,
    prompt_config: &PromptConfig,
    messages: &[ChatMessageRecord],
    max_context_chars: usize,
    extra_system_prompts: &[&str],
) -> Vec<ChatMessage>
```

Inside `model_messages`, push `effective_system_prompt(provider, prompt_config)` before memory/web prompts.

Update call sites in `services/chat/src/server.rs`:

- Non-streaming `send_message`
- Streaming `stream_message_worker`
- Memory extraction can keep its specialized `MEMORY_EXTRACTION_SYSTEM_PROMPT` and should not use the Rafael chat prompt.

### Context Budget Behavior

The built-in prompt participates in `max_context_chars`, same as the existing provider prompt. That is fine because the prompt is short.

If a user configures a very large provider prompt, current truncation behavior will preserve the beginning and end. Keep this behavior for now, but add a test that an oversized prompt does not remove the latest user message entirely.

## API/UI Implementation

Do not add prompt editing to the main chat surface. It belongs in settings.

Add read-only visibility first:

- In `ModelDetails`, show a `System prompt` detail with one of:
  - `Provider override`
  - `Rafael default`
  - `Disabled`

`PublicProvider` already exposes `system_prompt`. Add one field to report whether the backend default is active:

```rust
pub uses_default_system_prompt: bool,
```

`uses_default_system_prompt` is true when `provider.system_prompt` is empty and `AppConfig.prompt.default_system_prompt` is present.

If adding the field to `PublicProvider::from_stored` needs config access, replace it with:

```rust
PublicProvider::from_stored(provider, &state.config.prompt)
```

Existing provider save APIs can remain unchanged. A fuller editing UI can be added later, but the first implementation should not block on it because the backend behavior is what affects quality.

## Tests

Add backend unit tests in `services/chat/src/model.rs` or `services/chat/src/prompts.rs`.

Required tests:

1. `uses_default_prompt_when_provider_prompt_absent`
   - Provider has `system_prompt: None`.
   - Prompt config has the Rafael default.
   - First request message is a system message containing `You are Rafael`.

2. `provider_prompt_overrides_default_prompt`
   - Provider has `system_prompt: Some("Custom")`.
   - Prompt config has the Rafael default.
   - First system message is exactly `Custom`.

3. `disabled_default_prompt_sends_no_primary_system_prompt`
   - Provider prompt absent.
   - Prompt config default is `None`.
   - No primary system prompt is emitted unless memory/web prompts are passed.

4. `web_prompt_still_follows_primary_prompt`
   - Provider prompt absent.
   - Web tools enabled.
   - Message order is primary prompt, web prompt, then conversation history.

5. `memory_extraction_does_not_use_chat_prompt`
   - Memory extraction still sends only `MEMORY_EXTRACTION_SYSTEM_PROMPT` plus conversation content.

Frontend tests are optional unless a frontend test harness already exists for settings components. Backend tests are sufficient for the first pass.

## Tradeoffs

- Built-in default prompt improves out-of-box behavior, but every token in the prompt is context overhead.
- Provider override semantics are less flexible than layered custom instructions, but they are predictable and preserve existing behavior.
- A short local-model prompt will improve answer shape more than deep reasoning. Stronger models still matter more for hard synthesis.

## Definition of Done

- Existing providers with custom prompts keep their exact behavior.
- Saved providers with no prompt get the default prompt.
- Memory extraction is unaffected.
- Web citation instructions still apply when tools are enabled.
- README documents both default prompt and provider override behavior.
