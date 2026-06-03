# services/coding

`coding` is Rafael's GitHub App worker. It listens for selected GitHub webhook
events, decides whether the event should start a coding run, asks the local
model for a plan and bounded code actions, verifies the result, then publishes a
branch and pull request through the GitHub App installation.

This is not a general-purpose bot. It is wired for the local Rafael setup:

- model API: `http://rafael:8080/v1`
- model name: `qwen3-coder`
- GitHub App slug: `netshared`
- implementation label: `netshared:implement`
- command mention: `@netshared`
- default bind address: `127.0.0.1:3030`

The model endpoint is served by `infra/systemd/llama-swap.service`.

## Commands

Run commands from the repository root:

```bash
cargo run -p coding -- check-config
cargo run -p coding -- serve
```

`check-config` loads the environment and logs the resolved high-level settings.
Use it before starting the webhook server.

`serve` starts the GitHub webhook receiver on `RAFAEL_BIND` and requires
`RAFAEL_GITHUB_WEBHOOK_SECRET`. The only route is:

```txt
POST /webhooks/github
```

For a one-off issue run without a webhook:

```bash
cargo run -p coding -- issue-triggered \
  --repo cachebag/rafael \
  --issue 123 \
  --trigger manual \
  --actor cachebag \
  --installation-id "$RAFAEL_GITHUB_APP_INSTALLATION_ID"
```

If `--installation-id` is omitted, `RAFAEL_GITHUB_APP_INSTALLATION_ID` must be
set. `--run-id` is optional; without it the service generates a timestamped run
id.

## Webhook Triggers

The server verifies `X-Hub-Signature-256` with
`RAFAEL_GITHUB_WEBHOOK_SECRET`. It also requires `X-GitHub-Event` and
`X-GitHub-Delivery`; the delivery id becomes the run id.

Accepted issue triggers:

- `issues:labeled` when the label matches `RAFAEL_IMPLEMENT_LABEL`
- `issue_comment:created` from a trusted user with `@netshared implement` or
  `@netshared retry`
- `issues:assigned` to `RAFAEL_COLLABORATOR_LOGIN`, only when
  `RAFAEL_ENABLE_ASSIGNMENT_TRIGGER=true`

Accepted pull request revision triggers:

- `issue_comment:created` on a pull request from a trusted user with
  `@netshared revise` or `@netshared retry`
- `pull_request_review:submitted` from a trusted user when the review state is
  `changes_requested` or `commented`
- `pull_request_review_comment:created` from a trusted user with
  `@netshared revise` or `@netshared retry`

The worker ignores repositories outside `RAFAEL_ALLOWED_REPOS`, events from
untrusted users where trust is required, issues with blocking labels, approved
reviews, closed pull requests, and pull requests whose head branch is in a
different repository.

## Run Flow

For issue runs, the worker:

1. Creates a run directory and writes `state.json`.
2. Creates a GitHub App installation token.
3. Fetches repository, issue, and comment context.
4. Creates an isolated checkout under `RAFAEL_WORKDIR`.
5. Creates a branch named from the issue labels and title, such as
   `feat/issue-12-add-webhook-receiver`.
6. Writes `context.json` and asks the model for `plan.md`.
7. Runs the constrained change loop and records `transcript.jsonl`.
8. Runs verification, with one bounded repair attempt after failures.
9. Commits, pushes, and creates or updates a pull request when verification
   allows publishing.

For pull request revision runs, the worker checks out the pull request head
branch directly, records `pr-state.json` and `review-feedback.json`, applies a
revision plan, then pushes back to the existing pull request branch.

## Filesystem Layout

The local example environment uses:

```txt
RAFAEL_WORKDIR=/home/cachebag/.local/share/rafael/worktrees
RAFAEL_RUNS_DIR=/home/cachebag/.local/share/rafael/runs
```

Issue run artifacts are written under:

```txt
$RAFAEL_RUNS_DIR/<owner>__<repo>/issue-<number>/<run-id>/
```

Pull request revision artifacts are written under:

```txt
$RAFAEL_RUNS_DIR/<owner>__<repo>/pr-<number>/<run-id>/
```

Common artifacts:

- `state.json`
- `context.json`
- `plan.md`
- `transcript.jsonl`
- `diff.stat`
- `verification/`

Checkouts are separate from run artifacts and use the same
`<owner>__<repo>/issue-<number>/<run-id>/checkout` or
`<owner>__<repo>/pr-<number>/<run-id>/checkout` shape inside `RAFAEL_WORKDIR`.

## Verification

Verification commands come from repository context plus
`RAFAEL_VERIFY_COMMANDS`. If neither provides commands, the worker falls back
based on detected project type:

- Rust workspace: `cargo fmt --all -- --check`, then
  `cargo test --workspace --all-features`
- Rust package: `cargo fmt --all -- --check`, then `cargo test --all-features`
- Go: `go test ./...`
- Node: `npm test`
- Python: `python -m pytest`

Verification commands are parsed directly, not run through a shell. Shell
control operators and variable expansion are rejected. Child processes also have
`RAFAEL_*` variables removed from their environment.

By default, publishing requires verification to pass. If no verification command
is selected, the run completes locally but will not publish unless
`RAFAEL_ALLOW_UNVERIFIED_PUBLISH=true`.

## Environment

The tracked example is `services/coding/.env.example`.

Required for all modes:

| Variable | Purpose |
| --- | --- |
| `RAFAEL_GITHUB_APP_ID` | GitHub App ID. |
| `RAFAEL_GITHUB_APP_PRIVATE_KEY_PATH` | Path to the GitHub App private key PEM. |
| `RAFAEL_ALLOWED_REPOS` | Comma-separated allowlist such as `cachebag/rafael`. |

Required for `serve`:

| Variable | Purpose |
| --- | --- |
| `RAFAEL_GITHUB_WEBHOOK_SECRET` | GitHub webhook secret used for HMAC verification. |

Required for manual CLI runs unless passed as `--installation-id`:

| Variable | Purpose |
| --- | --- |
| `RAFAEL_GITHUB_APP_INSTALLATION_ID` | GitHub App installation ID. |

Model defaults:

| Variable | Default |
| --- | --- |
| `RAFAEL_MODEL_BASE_URL` | `http://rafael:8080/v1` |
| `RAFAEL_MODEL_NAME` | `qwen3-coder` |

GitHub behavior defaults:

| Variable | Default |
| --- | --- |
| `RAFAEL_GITHUB_APP_SLUG` | `netshared` |
| `RAFAEL_COLLABORATOR_LOGIN` | `<slug>[bot]` |
| `RAFAEL_GITHUB_API_BASE_URL` | `https://api.github.com` |
| `RAFAEL_IMPLEMENT_LABEL` | `netshared:implement` |
| `RAFAEL_COMMAND_MENTION` | `@<slug>` |
| `RAFAEL_TRUSTED_USERS` | empty |
| `RAFAEL_BLOCKING_LABELS` | `no-ai,needs-human,blocked` |
| `RAFAEL_ENABLE_ASSIGNMENT_TRIGGER` | `false` |
| `RAFAEL_QUIET_COMMENTS` | `false` |
| `RAFAEL_GIT_AUTHOR_NAME` | collaborator login |
| `RAFAEL_GIT_AUTHOR_EMAIL` | GitHub noreply address built from app id and collaborator login |

Workspace and limit defaults:

| Variable | Default |
| --- | --- |
| `RAFAEL_BIND` | `127.0.0.1:3030` |
| `RAFAEL_WORKDIR` | `/var/lib/rafael/worktrees` |
| `RAFAEL_RUNS_DIR` | `../runs` relative to `RAFAEL_WORKDIR` |
| `RAFAEL_MAX_RUN_MINUTES` | `45` |
| `RAFAEL_VERIFY_COMMANDS` | empty |
| `RAFAEL_MAX_TOOL_ITERATIONS` | `24` |
| `RAFAEL_MAX_TOOL_RUNTIME_SECONDS` | `900` |
| `RAFAEL_MAX_FILE_READ_BYTES` | `131072` |
| `RAFAEL_MAX_WRITE_BYTES` | `262144` |
| `RAFAEL_MAX_CHANGED_FILES` | `12` |
| `RAFAEL_VERIFY_COMMAND_TIMEOUT_SECONDS` | `600` |
| `RAFAEL_VERIFY_TOTAL_TIMEOUT_SECONDS` | `1200` |
| `RAFAEL_ALLOW_UNVERIFIED_PUBLISH` | `false` |
