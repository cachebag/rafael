# AI Collaborator Design

## Overview

The AI collaborator is a personal coding agent, exposed to GitHub as a GitHub App, that watches accepted GitHub issue events and turns issue specs into one or more pull requests. It should behave like a careful junior-to-mid-level contributor: read the issue, inspect the repository, plan the work, make scoped changes, run verification, explain tradeoffs, and leave the repository in a reviewable state.

The first version should prioritize reliability, auditability, and narrow permissions over autonomy. The agent should only operate on repositories where the GitHub App is installed and explicitly configured, only respond to assignment events, labels, or explicit commands, and never push directly to protected branches.

## Goals

- Trigger implementation work from approved GitHub issue events.
- Read the issue body, labels, comments, repository context, and current default branch.
- Generate an implementation plan before changing code.
- Create a dedicated branch for the issue.
- Modify code using local tools.
- Run project-appropriate checks.
- Commit the work with a clear message.
- Open a pull request linked to the original issue.
- Comment progress and final status back on the issue.
- Leave enough logs and artifacts to debug failures.
- Support local model execution through the existing llama.cpp OpenAI-compatible endpoint.
- Use GitHub App installation tokens instead of a long-lived personal access token.
- Keep GitHub permissions scoped per installation and per repository.

## Non-Goals

- Fully autonomous production deployments.
- Direct pushes to `master`, `main`, or protected release branches.
- Running on arbitrary public repositories without allowlist configuration.
- Building a public multi-tenant GitHub App service.
- Replacing human code review.
- Solving ambiguous or under-specified issues without asking for clarification.
- Guaranteeing one-shot success for large, cross-cutting feature work.

## User Experience

The intended workflow:

1. A human installs the GitHub App on an allowed repository.
2. A human writes a GitHub issue with a clear implementation spec.
3. The human assigns the issue, applies an `ai:implement` label, or comments with an explicit command.
4. The GitHub App receives the event and validates that the repository and trigger are allowed.
5. The collaborator acknowledges the run with an issue comment as the app bot identity.
6. The collaborator creates a branch such as `ai/issue-123-short-title`.
7. The collaborator inspects the repo and posts or stores a brief plan.
8. The collaborator implements the requested change.
9. The collaborator runs checks.
10. The collaborator opens a pull request as the app bot identity.
11. The pull request includes summary, tests run, known limitations, and a link to the issue.
12. If the task is blocked, the collaborator comments on the issue with specific questions.

Example issue command surface:

```text
@collaborator implement
@collaborator retry
@collaborator split into smaller PRs
@collaborator stop
```

For the first version, the cleanest trigger is likely `ai:implement` label or `@collaborator implement` comment, with assignment support enabled if the chosen app or bot identity can be assigned in the repository. The worker should explicitly check whether the configured collaborator login is assignable before relying on assignment as the only trigger.

## High-Level Architecture

```text
GitHub Issue Event
        |
        v
GitHub App
        |
        +--> Installation access token
        +--> Issue / label / comment webhooks
        |
        v
Webhook Receiver or GitHub Actions Trigger
        |
        v
Self-hosted Runner on rafael
        |
        v
services/coding worker
        |
        +--> GitHub API
        +--> Local git checkout
        +--> Tool crates
        +--> OpenAI-compatible model endpoint
        +--> Test/build commands
        |
        v
Branch + Commit + Pull Request
```

The GitHub App should be the primary GitHub identity and permission boundary. The execution path has two viable options:

- `GitHub Actions bridge`: GitHub Actions receives issue events, runs on the self-hosted runner, and the worker exchanges the app private key for an installation token. This avoids exposing a webhook receiver immediately.
- `Direct webhook receiver`: the GitHub App sends webhooks to a small service running on `rafael`, which queues work and launches the coding worker. This gives better control over queueing, retries, and cancellation, but requires a reachable and secured webhook endpoint.

The MVP should probably start with the GitHub Actions bridge unless exposing a webhook endpoint is already comfortable. The design should still treat the GitHub App as the long-term control plane.

## Repository Fit

This repo already has a natural place for the worker:

- `services/coding`: main coding-agent service or CLI.
- `crates/client`: model client for the OpenAI-compatible endpoint.
- `crates/config`: shared configuration loading.
- `crates/events`: event types and run lifecycle records.
- `crates/memory`: durable notes about repositories, conventions, and previous runs.
- `crates/tools/git`: git operations.
- `crates/tools/shell`: command execution.
- `crates/tools/filesystem`: file reading and editing helpers.
- `crates/tool-registry`: tool routing and metadata.

The first version can keep most logic in `services/coding` until the right abstractions become obvious. Shared crates should grow only when duplicated behavior appears.

## Trigger Design

### GitHub App Trigger

The GitHub App should subscribe to these webhook events:

- `issues`: assignment, labels, issue edits, issue lifecycle changes.
- `issue_comment`: explicit commands such as `@collaborator retry`.
- `pull_request`: optional follow-up tracking for PR state.
- `check_suite` or `check_run`: optional tracking for CI completion.

The app should start work only when one of these conditions is true:

- An issue receives the configured implementation label, such as `ai:implement`.
- An issue is assigned to the configured collaborator login and that login is assignable in the repository.
- A trusted user comments with an explicit command such as `@collaborator implement`.

The app should ignore:

- Pull requests masquerading as issues.
- Events from repositories outside the app installation allowlist.
- Events from users who are not allowed to command the agent.
- Issues with blocking labels such as `no-ai`, `needs-human`, or `blocked`.
- Duplicate events for an already active issue run.

### GitHub Actions Bridge

Use the `issues` and `issue_comment` events.

```yaml
on:
  issues:
    types: [assigned, labeled]
  issue_comment:
    types: [created]
```

The workflow should run only when:

- The issue is not a pull request.
- The trigger is assignment to the configured collaborator identity, the configured implementation label, or a trusted command comment.
- The repository is allowlisted.
- The issue does not have a blocking label such as `no-ai`, `needs-human`, or `blocked`.

The workflow should pass these values into the worker:

- Repository owner.
- Repository name.
- Issue number.
- Event action.
- Assignee login.
- GitHub run ID.
- Default branch.

The Actions bridge should use the GitHub App private key to generate an installation access token before calling the worker. The worker should use that installation token for GitHub API calls, branch pushes, issue comments, and PR creation.

### Direct Webhook Receiver

A future daemon can accept GitHub App webhooks directly. That would allow:

- Better queueing.
- Local run cancellation.
- Richer event handling.
- Better integration with homelab services.

The webhook version should validate GitHub signatures using `X-Hub-Signature-256` and a shared secret. It should return quickly, persist the event, and hand work to a queue rather than running the entire coding job inside the webhook request.

## Worker CLI

Initial command shape:

```sh
coding issue-triggered \
  --repo owner/name \
  --issue 123 \
  --trigger label \
  --actor octocat \
  --installation-id 123456 \
  --run-id "$GITHUB_RUN_ID"
```

Useful subcommands:

```sh
coding issue-triggered
coding issue-comment
coding run-local
coding resume
coding status
```

For the MVP, `issue-triggered` is enough.

## Run Lifecycle

Each run should have a stable lifecycle:

1. `received`: validate event and configuration.
2. `authenticated`: exchange the GitHub App JWT for an installation access token.
3. `claimed`: comment on issue and mark run active.
4. `prepared`: checkout default branch and create work branch.
5. `planned`: inspect repo and generate implementation plan.
6. `implemented`: apply code changes.
7. `verified`: run checks.
8. `published`: push branch and open PR.
9. `completed`: final issue/PR comment.
10. `blocked` or `failed`: explain what happened and preserve logs.

State should be persisted locally so a failed run can be inspected or resumed.

Suggested state location:

```text
.rafael/runs/<repo>/<issue>/<run-id>.json
```

This state should not be committed to the target repo unless explicitly desired.

## Configuration

Configuration should come from environment variables first, then an optional config file.

Environment variables:

```text
RAFAEL_MODEL_BASE_URL=http://rafael:8080/v1
RAFAEL_MODEL_NAME=Qwen/Qwen2.5-Coder-14B-Instruct-GGUF:Q4_K_M
RAFAEL_GITHUB_APP_ID=...
RAFAEL_GITHUB_APP_INSTALLATION_ID=...
RAFAEL_GITHUB_APP_PRIVATE_KEY_PATH=/run/secrets/rafael-github-app.pem
RAFAEL_GITHUB_WEBHOOK_SECRET=...
RAFAEL_GITHUB_APP_SLUG=rafael-collaborator
RAFAEL_COLLABORATOR_LOGIN=rafael-collaborator[bot]
RAFAEL_ALLOWED_REPOS=owner/repo,owner/other-repo
RAFAEL_WORKDIR=/var/lib/rafael/worktrees
RAFAEL_MAX_RUN_MINUTES=45
```

Optional config file:

```toml
[model]
base_url = "http://rafael:8080/v1"
name = "Qwen/Qwen2.5-Coder-14B-Instruct-GGUF:Q4_K_M"

[github]
app_id = 123456
installation_id = 987654
private_key_path = "/run/secrets/rafael-github-app.pem"
webhook_secret_env = "RAFAEL_GITHUB_WEBHOOK_SECRET"
app_slug = "rafael-collaborator"
collaborator_login = "rafael-collaborator[bot]"
allowed_repos = ["owner/repo"]

[workspace]
workdir = "/var/lib/rafael/worktrees"
branch_prefix = "ai"

[limits]
max_run_minutes = 45
max_changed_files = 50
max_diff_bytes = 200000
```

## GitHub App Authentication Flow

For each run, the worker should:

1. Load the GitHub App ID and private key.
2. Create a short-lived JWT signed with the app private key.
3. Use the JWT to request an installation access token for the repository installation.
4. Optionally scope that installation token to the target repository and required permissions.
5. Use the installation token for GitHub API calls and authenticated git pushes.
6. Discard the installation token after the run.

The installation ID can come from the webhook payload, workflow event payload, or static repository config. Static config is acceptable for the MVP if this starts with only one or two repositories.

## GitHub App Permissions

The GitHub App should request the minimum repository permissions required:

- `Contents`: read and write, for reading code and pushing branches.
- `Issues`: read and write, for reading issues and posting status comments.
- `Pull requests`: read and write, for opening PRs and posting PR comments.
- `Metadata`: read, required by GitHub Apps.
- `Checks`: read, optional for observing CI results.
- `Actions`: read, optional if the worker needs to inspect workflow runs.

The app should subscribe to these events:

- `Issues`.
- `Issue comment`.
- `Pull request`.
- `Check run` or `Check suite`, optional.

The worker should generate a short-lived installation access token for each run. Where possible, the token request should be scoped down to the single target repository and only the permissions needed for that run.

Private key handling:

- Store the GitHub App private key outside the repository.
- Mount it as a secret on the self-hosted runner or local daemon.
- Rotate it on a schedule.
- Do not include the private key, generated JWTs, or installation tokens in model context or logs.

If using the GitHub Actions bridge, workflow permissions for the default `GITHUB_TOKEN` should still be explicit and minimal:

```yaml
permissions:
  contents: read
  issues: write
  pull-requests: read
```

The default `GITHUB_TOKEN` should only be used to start the workflow and read the repository enough to run the worker. GitHub write operations should use the custom GitHub App installation token so comments, pushes, and PRs are attributed to the collaborator app.

A personal access token should be treated as a temporary fallback only. It is less clean because it is tied to a user, has different audit semantics, and is easier to over-scope.

## Branch and PR Strategy

Branch format:

```text
type/short-title
```

Commit style:

```text
type(#<issue-number>): short one liner

body

Closes #<issue-number>
```

PR title:

```text
type: short one liner
```

The PR should be created by the GitHub App installation token so the author is the collaborator app bot identity. PR body should include:

- Link to the issue.
- Short summary of changes.
- Verification commands run.
- Known limitations.
- Follow-up work, if any.

If the issue is too large, the agent should either:

- Ask for clarification before starting.
- Create a planning comment proposing multiple PRs.
- Implement the smallest independently reviewable first PR.

## Model Interaction

The model should receive structured context rather than a raw dump of the repository.

Recommended phases:

1. Summarize issue requirements.
2. Identify relevant files and commands.
3. Produce an implementation plan.
4. Execute edits through tools.
5. Review the diff.
6. Decide whether checks are sufficient.
7. Draft PR text.

The agent should make tool calls between model steps instead of asking the model to infer repository state from memory.

System guidance should emphasize:

- Preserve user changes.
- Keep changes scoped.
- Prefer existing patterns.
- Run verification before publishing.
- Ask questions when blocked.
- Do not invent APIs or dependencies without checking the repo.

## Tooling

Minimum tools:

- Read files.
- Search files.
- Edit files.
- Run shell commands.
- Run git commands.
- Generate GitHub App JWTs and installation access tokens.
- Call the GitHub API.
- Call the model endpoint.

Tool safety:

- Deny destructive commands by default.
- Require explicit allowlist for commands such as `rm`, force push, database migration, package publishing, and deployment.
- Limit command runtime.
- Capture stdout, stderr, exit code, and elapsed time.
- Redact secrets from logs.

## Verification

The worker should infer verification commands from the repository, then fall back to conservative defaults.

For this repo:

```sh
cargo fmt --check
cargo test
cargo clippy --workspace --all-targets
```

The issue spec can override or add commands:

```text
Verification:
- cargo test -p coding
- cargo fmt --check
```

If checks cannot run, the PR must say why.

## Failure Handling

The agent should comment on the issue when it cannot proceed.

Common blocked states:

- Issue spec is ambiguous.
- Required credentials are unavailable.
- Tests fail for unrelated reasons.
- Merge base is stale or branch conflicts.
- The requested change requires external services.
- The model produces repeated invalid edits.

Failure comments should include:

- What was attempted.
- The precise blocker.
- Relevant command output summary.
- Specific question or next action needed.

The agent should avoid vague comments such as "I could not complete this."

## Concurrency

Only one active run per issue should be allowed.

Options:

- GitHub Actions concurrency group:

```yaml
concurrency:
  group: ai-collaborator-${{ github.repository }}-${{ github.event.issue.number }}
  cancel-in-progress: false
```

- Local lock file per issue.
- Run state table if a database is added later.

The worker should avoid running two agents against the same worktree.

## Security

Main risks:

- Prompt injection from issue text, comments, or repository files.
- Exfiltration of secrets through generated code, comments, or logs.
- Exposure of the GitHub App private key or installation tokens.
- Destructive shell commands.
- Pull requests that modify workflow files or security-sensitive config.
- Running untrusted code on the self-hosted runner.

Mitigations:

- Treat issue text and repo content as untrusted input.
- Keep secrets out of model context.
- Keep the GitHub App private key outside the repository and rotate it.
- Generate installation tokens per run instead of caching long-lived credentials.
- Redact environment variables from command output.
- Use a locked-down self-hosted runner user.
- Restrict writable directories.
- Allowlist repositories.
- Avoid giving the agent deployment credentials.
- Flag changes to `.github/workflows`, auth code, secrets handling, or infrastructure files for human review.
- Never auto-merge agent PRs in the MVP.

## Observability

Each run should produce:

- Structured run state.
- Plain text log.
- Commands executed.
- Model prompts and responses, with secrets redacted.
- Diff summary.
- Final status.

Possible local layout:

```text
/var/lib/rafael/runs/
  owner_repo/
    issue-123/
      run-456/
        state.json
        log.txt
        plan.md
        diff.patch
```

GitHub comments should be concise. Detailed logs can stay local unless explicitly attached.

## MVP Implementation Plan

### Phase 0: GitHub App Setup

Create a private GitHub App for the collaborator.

It should:

- Be installed only on selected repositories.
- Request minimal repository permissions.
- Subscribe to issue, issue comment, and pull request events.
- Generate a private key stored outside the repository.
- Record app ID, installation ID, app slug, and bot login in config.
- Verify whether the app bot login can be assigned to issues in each target repository.

### Phase 1: Manual Worker

Build a local command that can run against a specific issue:

```sh
cargo run -p coding -- issue-triggered --repo owner/name --issue 123 --trigger label --installation-id 987654
```

It should:

- Generate a GitHub App installation access token.
- Fetch issue metadata.
- Clone or update the repo worktree.
- Create a branch.
- Generate a plan.
- Stop before editing.

### Phase 2: PR Creation

Add:

- File editing loop.
- Verification command execution.
- Commit creation.
- Branch push.
- PR creation.

### Phase 3: GitHub Actions Trigger

Add workflow:

- Trigger on assigned issues, implementation labels, and trusted command comments.
- Gate on collaborator login, implementation label, or trusted command sender.
- Run on self-hosted runner.
- Generate or pass through a GitHub App installation token.
- Invoke `services/coding`.

### Phase 4: Reliability

Add:

- Run state persistence.
- Retry/resume.
- Better failure comments.
- Command allowlist.
- Repository-specific config.

### Phase 5: Multi-PR Planning

Add:

- Large issue detection.
- Plan comments.
- PR dependency tracking.
- Follow-up issue or PR creation.

## Open Questions

- Should assignment alone start work, or should assignment plus a label such as `ai:implement` be required?
  - Assignment + label. Make the label `jonah:implement`.
- Should the MVP use a direct GitHub App webhook receiver, or use GitHub Actions as the bridge first?
  - GitHub App webhook receiver
- Can the selected GitHub App bot login be assigned to issues in the target repositories, or should label/comment command be the primary trigger?
  - hmmm I don't know. Let's do label/comment.
- Should the agent post its plan publicly as an issue comment before implementation?
  - No.
- Should changes to workflow, auth, infra, or dependency files require explicit human approval?
  - Yes. All changes should require human approval. But work done in the respective branch is freeform. PRs will likely undergo strict iteration by human
    requiring the AI to address any requested changes.
- How much local run history should be retained?
  - An issue assigned to an agent should be self contained. Should be a running knowledge base of the issue at hand. Issues should not 
    span more than one PR. If a change is deemed to be too big, the agent should tag @cachebag in the issue and ask how they should proceed.
- Should the agent support multiple repositories immediately, or only this monorepo at first?
  - Just this monorepo. They are a collaborator specifically on this repo and only this repo.
- Should failed runs automatically retry after model or network failures?
  - Yes.

## Initial Acceptance Criteria

The first usable version is done when:

- The GitHub App is installed on a configured repository.
- Applying the configured trigger to an issue starts a run.
- The worker authenticates using a GitHub App installation token.
- The run creates a dedicated branch.
- The run implements a small code change from the issue.
- The run executes configured verification commands.
- The run opens a pull request.
- The PR links back to the issue.
- The issue receives status comments for start, blocked/failure, and completion.
- The agent never pushes to the default branch.
