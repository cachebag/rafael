# rafael crates

these crates are reusable building blocks for services, clis, desktop tools, and
automation. they are intentionally small: each crate owns one reusable concern,
and services compose them instead of inheriting a framework.

## how to depend on them

from a service crate:

```toml
[dependencies]
client = { path = "../../crates/client" }
filesystem = { path = "../../crates/tools/filesystem" }
shell = { path = "../../crates/tools/shell" }
```

use only the crates a service actually needs.

## core crates

### `common`

small shared primitives that are useful across crate boundaries.

- `SafeComponent`: validates a single filesystem/key component, rejecting empty
  values, `.`/`..`, separators, and nul bytes.
- `SecretString`: stores secret text while redacting `Debug`/`Display` output.
- `truncate_utf8`: byte-limit text without cutting utf8 in the middle.
- `parse_bool`, `split_delimited`, `dedupe_preserving_order`, `slugify`.
- `looks_sensitive_name`: conservative env/key-name sensitivity detection.

use this when a reusable crate needs basic validation or text helpers.

next implementation: add a `SafeRelativePath` type for reusable path validation
that is less filesystem-specific than `tools/filesystem`.

### `config`

typed environment parsing without service-specific env names.

- `EnvConfig::from_current_env()` captures process env.
- `EnvConfig` implements `FromIterator` (e.g. `pairs.into_iter().collect()`) for deterministic tests.
- `EnvReader` supports required/optional strings, parsed values, bools, lists,
  csvs, and paths.
- error messages redact values for sensitive-looking names.

use this to build service-specific config structs while keeping parsing boring.

next implementation: add layered config loading from env plus an optional toml
file, with env values taking precedence.

### `events`

generic event envelopes and jsonl helpers.

- `EventKind`: validated event kind like `model.output` or `tool.result`.
- `EventEnvelope<T>`: id, kind, source, timestamp, and typed data.
- `JsonEvent`: envelope over `serde_json::Value`.
- `EventFilter`: simple kind/source matching.
- `to_json_line` / `from_json_line`: append/read jsonl event streams.

use this for transcripts, audit trails, workflow state, or service event logs.

next implementation: add an async jsonl `EventStore` with append, read, tail,
and filter-by-kind/source operations.

### `client`

openai-compatible local model client for `/v1/chat/completions`.

- `LocalModelClient`: async chat client over `reqwest`.
- `ModelClientConfig`: base url, model name, optional timeout.
- `ChatMessage`, `ChatRequest`, `ChatOptions`, `ChatCompletion`.
- `complete_text`: system/user prompt to text.
- `complete_json<T>`: asks for json object and parses into a typed value.
- `extract_json_output` / `parse_json_output`: handles fenced or embedded json.

use this for llama.cpp/openai-compatible local model calls.

next implementation: add tool-call request/response support using
`tool-registry` tool specs.

### `memory`

simple file-backed json memory store.

- `MemoryAddress`: safe `namespace` + `key`.
- `NewMemoryEntry`: json value plus tags/source.
- `MemoryEntry`: stored value plus created/updated timestamps.
- `JsonMemoryStore`: async `put`, `get`, `list`, and `delete`.
- writes use temp-file replacement and reject symlinks.

use this for durable local facts, summaries, preferences, model outputs, and
small state snapshots. it is not a vector database yet.

next implementation: add tag/source indexes so callers can query memory without
scanning every json file in a namespace.

## tool crates

### `tool-registry`

model-facing tool metadata and invocation validation.

- `ToolName`: validated tool identifier.
- `ToolDefinition`: name, description, json-schema parameters.
- `ToolRegistry`: register tools, reject duplicates, validate invocations.
- `openai_tools`: converts registered definitions into openai-style function
  tool specs.
- `ToolInvocation`: parsed model-selected tool name and arguments.

use this when a service gives a model a set of callable tools.

next implementation: add typed argument validation and async tool dispatch so a
registered tool can be executed from a model invocation.

### `tools/filesystem`

safe rooted async filesystem operations.

- `RootedFilesystem::open` / `open_or_create`.
- `read_text`, `read_text_range`, `write_text`, `edit_text`.
- `search_text` and `list_dir`.
- `ReadOptions`, `WriteOptions`, `SearchOptions` for explicit limits.
- rejects absolute paths, parent traversal, `.git`, symlinks, and known
  secret-bearing paths like `.env`, pem/key files, token/secret paths.

use this for model-accessible file operations and workspace tools.

next implementation: expose filesystem operations as `tool-registry`
definitions plus typed invocation/result structs.

### `tools/shell`

bounded async subprocess runner.

- `CommandSpec`: program, args, cwd, env, timeout, env policy.
- `CommandRunner`: timeout + stdout/stderr capture limits.
- `CommandOutput`: status, captured stdout/stderr, byte counts, truncation.
- `parse_command_line`: small parser for simple command strings; rejects shell
  control operators and variable expansion.
- child processes run noninteractive by default.

use this for service-owned commands where args are structured and bounded.

next implementation: add a log-to-file runner for long commands, keeping only a
bounded preview in memory.

### `tools/git`

typed git cli wrapper built on `shell`.

- `GitClient`: raw git execution plus common helpers.
- `status_short`, `changed_files`, `diff_stat`, `current_branch`, `rev_parse`.
- `switch_branch` validates branch names.
- `clone_repo` with optional auth header.
- `github_basic_auth_header` for GitHub App installation tokens.
- `slug_branch_component` for branch-name-safe labels.

use this for repository automation without shell strings.

next implementation: add a safe worktree manager for clone/fetch/switch flows
under a caller-provided root.

### `tools/docker`

thin docker cli wrapper built on `shell`.

- `DockerClient::version`.
- `list_containers(all)` parses `docker ps --format json`.
- `inspect(target)` returns json.
- `logs(container, tail)` returns bounded command output.
- `raw(args)` for explicit caller-owned docker calls.

use this for local container introspection and controlled docker operations.

next implementation: add docker compose project inspection for services,
containers, status, and recent logs.

### `tools/tailscale`

thin tailscale cli wrapper built on `shell`.

- `TailscaleClient::status()` parses `tailscale status --json`.
- `ip()` returns local tailscale addresses.
- `ping(target, timeout_seconds)`.
- `raw(args)` for explicit caller-owned tailscale calls.

use this for mesh/device awareness in homelab services.

next implementation: add peer lookup and health summary helpers for finding
nodes by hostname, dns name, tag, online state, or tailscale ip.

### `tools/systemd`

thin `systemctl` wrapper built on `shell`.

- `SystemdClient::user()` and `SystemdClient::system()`.
- `UnitName`: validated unit identifier.
- `show_unit`, `is_active`, `list_units`.
- `start`, `stop`, `restart`.

use this for user/system service inspection and bounded service control.

next implementation: add journal tail support for a unit with bounded output and
structured metadata.

## common service patterns

### model tool loop

combine:

- `client` to ask the model for the next action.
- `tool-registry` to define the action surface.
- `filesystem`, `git`, `shell`, or other tool crates to execute actions.
- `events` to record prompts, tool calls, and results.
- `memory` to persist summaries and reusable context.

### local automation service

combine:

- `config` to parse service env.
- `shell` for bounded commands.
- `docker`, `tailscale`, and `systemd` for typed local adapters.
- `events` for a jsonl operation log.

### workspace-aware cli

combine:

- `filesystem` for safe reads/search/edits under a selected root.
- `git` for diff/status/branch metadata.
- `client` to turn local model output into typed json commands.

## current limits

- tool crates are cli adapters, not daemon clients.
- `memory` is a json-file store, not search/vector memory.
- `tool-registry` validates tool names and registry membership; argument schema
  validation is the next missing piece.
- live docker/tailscale/systemd/git behavior should be integration-tested by the
  service that uses it.
