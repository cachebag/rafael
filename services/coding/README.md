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

- `RAFAEL_MODEL_BASE_URL`
- `RAFAEL_MODEL_NAME`
- `RAFAEL_GITHUB_APP_ID`
- `RAFAEL_GITHUB_APP_INSTALLATION_ID`
- `RAFAEL_GITHUB_APP_PRIVATE_KEY_PATH`
- `RAFAEL_GITHUB_WEBHOOK_SECRET`
- `RAFAEL_GITHUB_APP_SLUG`
- `RAFAEL_COLLABORATOR_LOGIN`
- `RAFAEL_ALLOWED_REPOS`
- `RAFAEL_GITHUB_API_BASE_URL`
- `RAFAEL_GIT_AUTHOR_NAME`
- `RAFAEL_GIT_AUTHOR_EMAIL`
- `RAFAEL_IMPLEMENT_LABEL`
- `RAFAEL_COMMAND_MENTION`
- `RAFAEL_TRUSTED_USERS`
- `RAFAEL_BLOCKING_LABELS`
- `RAFAEL_ENABLE_ASSIGNMENT_TRIGGER`
- `RAFAEL_QUIET_COMMENTS`
- `RAFAEL_WORKDIR`
- `RAFAEL_RUNS_DIR`
- `RAFAEL_MAX_RUN_MINUTES`
- `RAFAEL_BIND`
