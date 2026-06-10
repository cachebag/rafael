# rafael

<p align="center">
  <a href="https://github.com/cachebag/rafael/actions/workflows/rust-ci.yml">
    <img alt="rust CI" src="https://github.com/cachebag/rafael/actions/workflows/rust-ci.yml/badge.svg?branch=master">
  </a>
  <a href="https://github.com/cachebag/rafael/actions/workflows/payme-web-ci.yml">
    <img alt="payme web CI" src="https://github.com/cachebag/rafael/actions/workflows/payme-web-ci.yml/badge.svg?branch=master">
  </a>
  <a href="https://github.com/cachebag/rafael/actions/workflows/deploy-chat.yml">
    <img alt="deploy chat" src="https://github.com/cachebag/rafael/actions/workflows/deploy-chat.yml/badge.svg?branch=master">
  </a>
  <a href="https://github.com/cachebag/rafael/actions/workflows/deploy-payme.yml">
    <img alt="deploy payme" src="https://github.com/cachebag/rafael/actions/workflows/deploy-payme.yml/badge.svg?branch=master">
  </a>
</p>

rafael is both my workstation and the name for this monorepo with all the various services, crates and configuration files that power my homelab.

there is no formal way to set this up. i've done my best to document everything and make it easy to work with, but that is in the context of ME working on it, not others. also, basically ALL of this aside from a few things here and there are "vibe-coded". so take this as your warning before using anything here. 

[tailscale](https://tailscale.com/) is used as a mesh VPN between my devices.

## Layout

```txt
crates/          shared Rust crates
services/        runnable services and CLIs
infra/docker/    Docker Compose stacks for external dependencies
infra/systemd/   systemd service definitions
```

## Current State

The active pieces right now are:

- `services/coding`: the GitHub App coding worker.
- `services/chat`: the local model chat interface.
- `services/payme`: the personal finance tracker.
- `infra/docker/searxng`: the local SearXNG search backend for chat web search.
- `infra/systemd/llama-swap.service`: the user systemd service for the local
  model lifecycle proxy.
- `infra/systemd/rafael-chat.service`: the user systemd service for the chat UI.
- `infra/systemd/rafael-payme.service`: the user systemd service for Payme.

## Public Services

Tailscale Funnel exposes the local web services publicly:

```txt
https://rafael.taild0efc0.ts.net/       -> rafael chat
https://rafael.taild0efc0.ts.net:8443/  -> payme
```

## Local Model

Local models are served through `llama-swap`, which lazily starts
`llama-server` processes from `infra/systemd/llama-swap.yaml`.

Model IDs:

```txt
gemma-everyday
qwen3-coder
gpt-oss
qwen-coder-fim
gemma-deep
```

Endpoint:

```txt
http://rafael:8080/v1
```

The systemd service definition is located at:

```txt
infra/systemd/llama-swap.service
```

More details:

- [infra/systemd/README.md](infra/systemd/README.md)
- [services/chat/README.md](services/chat/README.md)
- [services/coding/README.md](services/coding/README.md)

## License

MIT
