---
applyTo: "**/*.rs"
---
# rafael - rust code review guide

Guide for reviewing Rust pull requests in the `rafael` repository.

## Review bar

Raise issues only for material concerns: correctness, safety, security, reliability, maintainability, or behavior that can break users or future work.

Do not raise issues for pedantic, minor, stylistic, or non-breaking changes. If something is merely a preference, skip it. When in doubt, approve and move on.

## Error handling

Review error handling for correctness, debuggability, and safe failure behavior.

- Flag panics, unwraps, expects, or silent fallbacks in production paths when the failure is realistic and should be handled.
- Check that propagated errors include useful context: operation, path, repo, run ID, issue number, or external service when relevant.
- Flag any logging of secrets, tokens, webhook secrets, private keys, auth headers, or sensitive model/user content.
- Check that failed operations leave local state inspectable and consistent. Cleanup is good for partial locks or temp dirs; useful run artifacts and diffs should not be deleted without a clear reason.
- Check that terminal states are precise. Do not collapse distinct outcomes such as `failed`, `blocked`, and `cancelled` unless the behavior intentionally treats them the same.
- Flag blocking filesystem, network, or process work inside async contexts unless it uses async APIs or `spawn_blocking`.
- Check that external calls have bounded timeouts or are covered by an enclosing run timeout.
- Flag retry or recovery loops that are unbounded, unclear, or can repeat side effects unexpectedly.
- Prefer structured logging with `tracing`; flag `println!`/`eprintln!` in production paths unless it is intentional CLI output.
- User-visible errors should be actionable without exposing internals or secrets.

## Tests

Review tests for meaningful behavior coverage, not coverage volume.

- Request tests when a change affects core logic, edge cases, error handling, state transitions, security boundaries, or external-service behavior.
- Bug fixes should include a regression test that fails before the fix and passes after it, unless the scenario is impractical to test.
- Tests should be deterministic and avoid external state, timing assumptions, or live services unless explicitly marked as integration tests.
- Prefer focused tests with clear names. If a test fails, it should be obvious what behavior broke.
- Do not request tests for trivial code paths or purely mechanical changes unless they create real risk.

## Code organization

Review organization for readability and long-term maintenance.

- Flag functions or modules that mix too many responsibilities or make the main control flow hard to follow.
- Suggest splitting files over 1,000 lines when the split would make ownership and flow clearer.
- Prefer code that tells the story first and leaves implementation details later. Helpers, structs, and constants should not interrupt the main flow unless they are central to understanding it.
- Be careful with purely stylistic organization feedback. If the current organization is clear and maintainable, do not raise an issue.

## Bug fixes

For bug-fix PRs, check that the fix addresses the root cause instead of only masking the symptom. Prefer a regression test that captures the original failure mode.

Do not block a bug fix on broad refactors, unrelated cleanup, or ideal architecture changes. Keep review feedback scoped to the bug and any direct safety risks introduced by the fix.

## Documentation

Review documentation and comments for usefulness, not volume.

- Public APIs and non-obvious internal behavior should be understandable from names, types, and concise comments where needed.
- Flag comments that are misleading, stale, or merely restate obvious code.
- Prefer comments that explain why a tradeoff, state transition, recovery path, or external-service behavior exists.
- Do not request documentation for self-explanatory private helpers or minor changes unless missing context would create maintenance risk.
