# Crate Rust Agent Guidelines

These instructions apply to Rust code under `crates/`.

The crates are the reusable foundation for `rafael`: shared utilities, configuration,
memory, tool adapters, registries, protocol types, and other library code used by
services and CLIs. Keep them small, explicit, composable, and boring to depend on.

## Scope and style

- Treat crates as library boundaries, not application glue.
- Keep public APIs focused, typed, and hard to misuse.
- Prefer small modules with clear ownership over broad framework-style abstractions.
- Use existing crate patterns and dependencies before adding new ones.
- Do not introduce new crates unless the benefit is clear and local alternatives are worse.
- Avoid service-specific assumptions such as GitHub issue numbers, webhook deliveries, run IDs, or systemd unit names unless the crate is explicitly about that domain.
- Avoid hidden global state. Prefer explicit inputs, outputs, and configuration.
- Keep side effects visible in function names and types. Pure helpers should stay pure.

## Public APIs

- Design APIs around stable capabilities, not one caller's immediate implementation detail.
- Expose concrete types when they make behavior clearer; avoid trait abstractions until there is real variation to model.
- Keep constructors and builders validating invariants early.
- Prefer returning structured data over preformatted strings when callers may need to inspect or compose results.
- Avoid leaking internal filesystem layouts, environment variable names, process lifecycles, or service-specific state into generic crates.
- Keep compatibility in mind when changing public structs, enums, or error types used by multiple services.

## Error handling

- Propagate realistic failures with useful context.
- Avoid `unwrap`, `expect`, and panics in production paths.
- Use precise error types for reusable contracts when callers need to branch on outcomes.
- Use `anyhow` only where the crate is intentionally application-facing or where structured errors would not improve callers.
- Include useful context in errors: operation, path, command, backend, external service, or parsed value.
- Do not expose secrets, tokens, private keys, auth headers, or sensitive prompt/user content in errors or logs.
- Recovery and retry helpers must be bounded and explicit. Avoid hidden loops or repeated side effects.

## State and filesystem safety

- Validate caller-provided paths before using them.
- Keep path components such as IDs and names as single safe components, not arbitrary paths.
- Ensure computed paths remain inside the intended root when a crate manages filesystem state.
- Use atomic creation or replacement where it matters for locks, caches, snapshots, or durable state.
- Handle stale, corrupt, or partially written state deliberately.
- Preserve enough structured state for callers to debug failures after a crash.
- Do not delete caller data, run artifacts, logs, diffs, or state files unless the API explicitly documents that behavior.

## Async and subprocesses

- Prefer async APIs only when the crate genuinely performs async work or is primarily used by async services.
- In async code, do not block the reactor thread. Use async APIs or `spawn_blocking`.
- Use `tokio::process::Command` for subprocesses in async crates.
- Pass subprocess arguments as structured args, not shell strings.
- Disable interactive prompts for subprocesses that may run unattended.
- External calls and subprocesses should have explicit timeouts or accept caller-provided timeout policy.
- Capture outputs needed for diagnostics, but avoid returning or logging huge outputs by default.

## Observability and secrets

- Library crates should avoid initializing tracing/logging subscribers.
- Use `tracing` spans/events only where they add reusable diagnostic value.
- Never log secrets, tokens, private keys, webhook secrets, auth headers, credentials, or sensitive prompt/user content.
- Prefer letting services decide how verbose logs should be.

## Tests

- Add focused tests for crate contracts: parsing, validation, path safety, state transitions, serialization, retry bounds, and error recovery.
- Bug fixes should include a regression test when practical.
- Keep tests deterministic. Avoid live network, GitHub, model, Docker, systemd, or Tailscale calls in unit tests.
- Prefer table-driven tests for parsing, filtering, validation, and policy decisions.
- Test public behavior rather than private implementation details unless the private helper guards a subtle invariant.
- Do not add tests only to inflate coverage or exercise trivial getters.

## Code organization

- Keep modules focused around reusable domains: config, filesystem safety, Git helpers, tool execution, memory/state, protocol types, and adapters.
- Put caller-facing types and functions near the top of a module; keep implementation details below the code that uses them when practical.
- Define type aliases, structs, enums, helper functions, helper methods, and private utilities after the code that uses them when it improves scanability.
- Put constants after production code, but before any `#[cfg(test)]` test modules. If a file has no tests, constants should be at the bottom.
- Split files when a module mixes unrelated responsibilities or becomes hard to scan.
- Avoid turning `common` into a dumping ground. Shared code should have a clear domain and at least one real caller.

## Documentation and comments

- Document public APIs when the behavior, invariants, side effects, or failure modes are not obvious.
- Add comments only when they explain non-obvious intent, constraints, or tradeoffs.
- Do not add comments that merely restate the code.
- Document migrations when changing public APIs used by services.
- If a crate introduces required environment variables, file layouts, or external credentials, document that in the crate or the service that owns deployment.

## Validation

When finished with code changes, never run any verification or git commands that mutate the workspace. This includes, but is not limited to:

- `cargo test`
- `cargo fmt`
- `git push`
- `git commit`
- `git add`
- `git restore`

You may use read-only commands, such as:

- `cargo check`
- `git status`
- `git diff`
- `git log`
