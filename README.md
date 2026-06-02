# rafael

rafael is both my workstation and the name for this monorepo with all the various services, crates and configuration files that power my homelab.

[tailscale](https://tailscale.com/) is used as a mesh VPN between my devices.

## Layout

```txt
crates/          shared Rust crates
services/        runnable services and CLIs
infra/systemd/   systemd service definitions
```

## Current State

The active pieces right now are:

- `services/coding`: the GitHub App coding worker.
- `infra/systemd/llama-server.service`: the user systemd service for the local
  `llama.cpp` server.

## Local Model

The primary local model is currently served via `llama.cpp`.

Model:

```txt
Qwen/Qwen2.5-Coder-14B-Instruct-GGUF:Q4_K_M
```

Endpoint:

```txt
http://rafael:8080/v1
```

The systemd service definition is located at:

```txt
infra/systemd/llama-server.service
```

More details:

- [infra/systemd/README.md](infra/systemd/README.md)
- [services/coding/README.md](services/coding/README.md)
- 
## License

MIT
