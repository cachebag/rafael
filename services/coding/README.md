# services/coding

This crate provides a personal AI coding collaborator. It includes functionality for handling GitHub App webhook receivers, running phase-one workers for accepted issue triggers, and validating environment configuration.

## Key Features
- **GitHub App Webhook Receiver**: Handles incoming webhooks from GitHub.
- **Issue Triggered Worker**: Processes issues based on triggers.
- **Configuration Validation**: Checks and logs the environment configuration.

## Usage
To run the service manually, use the following commands:

```bash
# Run the GitHub App webhook receiver
cargo run -- serve

# Run the phase-one worker for an issue trigger
cargo run -- issue-triggered --repo <repo-ref> --issue <issue-number> --trigger <trigger-kind> --actor <actor-name> --installation-id <installation-id> --run-id <run-id>

# Validate environment configuration
cargo run -- check-config
```

## Environment Variables
The following environment variables are required to configure `netshared` as a collaborator. You can find these in the `.env.example` file.

| Variable Name | Description |
|---------------|-------------|
| `RAFAEL_MODEL_BASE_URL` | Base URL for the model |
| `RAFAEL_MODEL_NAME` | Name of the model |
| `RAFAEL_GITHUB_APP_ID` | GitHub App ID |
| `RAFAEL_GITHUB_APP_INSTALLATION_ID` | GitHub App Installation ID |
| `RAFAEL_GITHUB_APP_PRIVATE_KEY_PATH` | Path to the GitHub App private key |
| `RAFAEL_GITHUB_WEBHOOK_SECRET` | Secret for GitHub webhooks |
| `RAFAEL_GITHUB_APP_SLUG` | GitHub App slug |
| `RAFAEL_COLLABORATOR_LOGIN` | Collaborator login |
| `RAFAEL_ALLOWED_REPOS` | Allowed repositories |
| `RAFAEL_GITHUB_API_BASE_URL` | Base URL for GitHub API |
| `RAFAEL_GIT_AUTHOR_NAME` | Git author name |
| `RAFAEL_GIT_AUTHOR_EMAIL` | Git author email |
| `RAFAEL_IMPLEMENT_LABEL` | Implement label |
| `RAFAEL_COMMAND_MENTION` | Command mention |
| `RAFAEL_TRUSTED_USERS` | Trusted users |
| `RAFAEL_BLOCKING_LABELS` | Blocking labels |
| `RAFAEL_ENABLE_ASSIGNMENT_TRIGGER` | Enable assignment trigger |
| `RAFAEL_QUIET_COMMENTS` | Quiet comments |
| `RAFAEL_WORKDIR` | Working directory |
| `RAFAEL_RUNS_DIR` | Runs directory |
| `RAFAEL_MAX_RUN_MINUTES` | Maximum run minutes |
| `RAFAEL_BIND` | Bind address |
