# AI Collaborator Progress

## Current State

The GitHub App based collaborator is working through the phase-one flow:

1. GitHub sends a webhook to `rafael`.
2. `services/coding` verifies the webhook signature.
3. The `jonah:implement` label or trusted `@netshared retry` comment triggers a run.
4. The worker authenticates as the GitHub App.
5. The worker fetches the issue and repository metadata.
6. The bot comments that the run started.
7. The worker prepares a local git worktree and branch.
8. The worker calls the local model for a plan.
9. The worker writes `state.json` and `plan.md`.
10. The bot comments that phase-one planning completed.

Code editing, verification, commit, push, and PR creation are not implemented yet.

## GitHub App

App:

```text
Name: netshared
App ID: 3937023
Installation ID: 137370498
Installed repository: cachebag/rafael
Bot login: netshared[bot]
```

Trigger config:

```text
Implementation label: jonah:implement
Command mention: @netshared
Trusted user: cachebag
Assignment trigger: disabled
```

Repository permissions:

```text
Contents: read/write
Issues: read/write
Pull requests: read/write
Metadata: read-only
```

Webhook events in use:

```text
Issues
Issue comment
Pull request
```

## Webhook Exposure

GitHub reaches the local service through Tailscale Funnel:

```text
https://rafael.taild0efc0.ts.net/webhooks/github
```

The local service listens on:

```text
127.0.0.1:3030
```

Funnel command used:

```sh
sudo tailscale funnel --bg --https=443 127.0.0.1:3030
```

Check Funnel:

```sh
tailscale funnel status
```

Disable Funnel:

```sh
sudo tailscale funnel --https=443 off
```

## Local Secrets

The GitHub App private key lives outside the repo:

```text
/run/secrets/rafael-github-app.pem
```

Permissions:

```text
-rw------- cachebag cachebag
```

Do not paste the private key into chat, logs, commits, or issue comments. If it is exposed, revoke it in GitHub App settings and generate a new key.

The webhook secret is not recorded here. It must match exactly between GitHub App settings and `RAFAEL_GITHUB_WEBHOOK_SECRET`.

## Runtime Environment

Current working env shape:

```sh
export RAFAEL_GITHUB_APP_ID=3937023
export RAFAEL_GITHUB_APP_INSTALLATION_ID=137370498
export RAFAEL_GITHUB_APP_PRIVATE_KEY_PATH=/run/secrets/rafael-github-app.pem
export RAFAEL_ALLOWED_REPOS=cachebag/rafael
export RAFAEL_GITHUB_APP_SLUG=netshared
export RAFAEL_COLLABORATOR_LOGIN='netshared[bot]'
export RAFAEL_IMPLEMENT_LABEL=jonah:implement
export RAFAEL_COMMAND_MENTION=@netshared
export RAFAEL_TRUSTED_USERS=cachebag
export RAFAEL_BIND=127.0.0.1:3030
export RAFAEL_WORKDIR="$HOME/.local/share/rafael/worktrees"
export RAFAEL_RUNS_DIR="$HOME/.local/share/rafael/runs"
export RAFAEL_GITHUB_WEBHOOK_SECRET='<same value configured in GitHub>'
```

Start the service:

```sh
cargo run -p coding -- serve
```

Validate config:

```sh
cargo run -p coding -- check-config
```

## Verified Smoke Test

Test issue:

```text
cachebag/rafael#2
```

The label `jonah:implement` triggered a successful phase-one run.

Observed successful log shape:

```text
accepted webhook trigger repo=cachebag/rafael issue=2 trigger=label
received GitHub App installation token
prepared phase-one coding run repo=cachebag/rafael issue=2 branch=work/test
```

Run artifacts:

```text
~/.local/share/rafael/runs/cachebag__rafael/issue-2/<run-id>/state.json
~/.local/share/rafael/runs/cachebag__rafael/issue-2/<run-id>/plan.md
```

Prepared worktree:

```text
~/.local/share/rafael/worktrees/cachebag__rafael
```

Expected branch state after phase one:

```text
## work/test...origin/master
```

The branch is local only. Push and PR creation are not enabled yet.

## Issues Encountered

Webhook signature mismatch:

- Cause: the terminal env used placeholder text instead of the exact GitHub webhook secret.
- Fix: set a real random secret in GitHub and export the same exact value locally.

Run directory permission failure:

- Cause: default paths under `/var/lib/rafael` were not writable by the user.
- Fix: use user-owned dev paths under `~/.local/share/rafael`.

`jsonwebtoken` crypto provider panic:

- Cause: crypto provider was ambiguous at runtime.
- Fix: configure `jsonwebtoken` with explicit `rust_crypto` and `use_pem` features.

Git username prompt during clone:

- Cause: git HTTPS auth did not use the installation token in the expected form.
- Fix: pass a non-interactive GitHub App token auth header and set `GIT_TERMINAL_PROMPT=0`.

Bot comment webhook ignored:

- This is expected. The bot posts status comments, GitHub sends `issue_comment` webhooks for them, and the service ignores `netshared[bot]` because it is not a trusted command sender.

## Next Build Slice

Implement the rest of the collaborator loop:

1. Apply edits in the prepared worktree.
2. Run verification commands.
3. Commit with the configured issue commit style.
4. Push the branch with the installation token.
5. Open a PR linked to the issue.
6. Comment final status on the issue.

