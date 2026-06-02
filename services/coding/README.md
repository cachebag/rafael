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
To properly configure `netshared` as a collaborator, the following environment variables are required:

- `NETSHARED_API_KEY`: Your API key for netshared.
- `NETSHARED_SECRET`: Your secret for netshared.

For an example configuration, refer to the `.env.example` file located in this directory.