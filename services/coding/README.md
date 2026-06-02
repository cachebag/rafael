# services/coding

This crate provides a personal AI coding collaborator. It includes functionality for handling GitHub App webhook receivers, running phase-one workers for accepted issue triggers, and validating environment configuration.

## Key Features
- **GitHub App Webhook Receiver**: Handles incoming webhooks from GitHub.
- **Issue Triggered Worker**: Processes issues based on triggers.
- **Configuration Validation**: Checks and logs the environment configuration.

## Usage
To run the service, use the following commands:

```bash
# Run the GitHub App webhook receiver
cargo run -- serve

# Run the phase-one worker for an issue trigger
cargo run -- issue-triggered --repo <repo-ref> --issue <issue-number> --trigger <trigger-kind> --actor <actor-name> --installation-id <installation-id> --run-id <run-id>

# Validate environment configuration
cargo run -- check-config
```

## Dependencies
- `anyhow`: For error handling.
- `axum`: For web framework.
- `base64`: For base64 encoding/decoding.
- `chrono`: For date and time handling.
- `clap`: For command-line argument parsing.
- `hex`: For hexadecimal encoding/decoding.
- `hmac`: For HMAC message authentication.
- `jsonwebtoken`: For JSON Web Token handling.
- `reqwest`: For HTTP requests.
- `serde`: For serialization/deserialization.
- `sha2`: For SHA-2 hashing.
- `tokio`: For asynchronous runtime.
- `tracing`: For structured logging.
- `tracing-subscriber`: For tracing subscriber.

## Setup
Ensure that all dependencies are correctly installed and that the environment variables are set as required by the configuration.

## Verification
Run the following commands to verify the code:

```bash
cargo fmt --all -- --check
cargo test --workspace --all-features
```
