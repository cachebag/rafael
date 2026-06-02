# Service Rust Agent Guidelines

These instructions apply to Rust code under `services/`. This directory contains runnable services, CLIs, webhook handlers, workers, and other user-facing processes for `rafael`.

When changing service code, favor reliable behavior, clear state transitions, bounded work, and debuggable failures over clever abstractions.

## Scope and style

- Keep changes focused on the requested service behavior.
- Prefer small, explicit functions over broad framework-style abstractions.
- Use existing patterns and dependencies before adding new ones.
- Do not introduce new crates unless the benefit is clear and local alternatives are worse.
- Use `tracing` for service logs. Avoid `println!`/`eprintln!` in production paths unless the command is intentionally producing CLI output.
- Never log secrets, tokens, private keys, webhook secrets, auth headers, or sensitive prompt/user content.

## Error handling

- Propagate realistic failures with `anyhow::Context` or equivalent context.
- Avoid `unwrap`, `expect`, and panics in production paths.
- Include useful context in errors: operation, path, repo, issue number, run ID, external service, or command where relevant.
- Keep terminal states precise. Distinguish outcomes like `failed`, `blocked`, and `cancelled` when behavior differs.
- Failed work should leave useful artifacts inspectable. Clean up partial locks/temp dirs, but do not delete run logs, state files, diffs, or other debugging evidence without a clear reason.
- Recovery and retry logic must be bounded and understandable. Avoid hidden loops or repeated side effects.
- External calls and subprocesses should have explicit timeouts or be covered by a service-level run timeout.
- In async code, do not perform blocking filesystem, network, or process work on the reactor thread. Use async APIs or `spawn_blocking`.
- User-visible errors should be actionable and should not expose internals or secrets.

## State, locks, and filesystem safety

- Validate any user-, webhook-, or CLI-derived value before using it in filesystem paths.
- Path components such as run IDs must be single safe components, not arbitrary paths.
- Ensure computed paths stay inside the configured workspace/run directories.
- Use atomic creation for locks when preventing duplicate work.
- Handle stale, corrupt, or orphaned lock files deliberately.
- Clear one-shot markers, such as cancellation files, when their run is finished.
- Persist enough state to understand what happened after a crash or timeout.

## Async and subprocesses

- Prefer `tokio` async APIs for filesystem and network work.
- Use `tokio::process::Command` for subprocesses.
- Pass command arguments as structured args, not shell strings.
- Disable interactive prompts for subprocesses that run unattended.
- Capture command output when it is needed for diagnostics, but avoid logging large outputs by default.
- Git network/branch operations may use the `git` CLI; keep auth headers and tokens out of logs.

## Tests

- Add focused tests for behavior that can break service correctness: state transitions, locking, cancellation, path validation, webhook decisions, and error recovery.
- Bug fixes should include a regression test when practical.
- Keep tests deterministic. Avoid live GitHub/model calls in unit tests.
- Prefer table-driven tests for parsing, filtering, and decision logic.
- Do not add tests only to inflate coverage or exercise trivial getters.

## Code organization

- Organize files top-down: the main service flow should appear first, and implementation details should appear after they are used.
- Define type aliases, structs, enums, helper functions, helper methods, and private utilities after the code that uses them whenever practical.
- Put constants after production code, but before any `#[cfg(test)]` test modules. If a file has no tests, constants should be at the bottom.
- Keep modules focused around service responsibilities: config, server, webhook, worker, GitHub/model clients, and state helpers.
- Split files when they become hard to scan or mix unrelated responsibilities.
- Avoid premature shared abstractions across services. Extract to crates only after duplication or a stable boundary appears.

## Documentation and comments

- Add comments only when they explain non-obvious intent, constraints, or tradeoffs.
- Do not add comments that merely restate the code.
- Document service behavior in docs or plans when it affects operation, secrets, deployment, or user workflow.
- If changing required environment variables, update the relevant docs or mention the migration clearly.

## Validation

When finished with code changes, never run any verification or git commands that mutate the workspace. This is including, but not limited to:
- `cargo test`
- `cargo clippy`
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
