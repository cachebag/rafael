# rafael

rafael is both my workstation and the name for this monorepo with all the various 
services, crates and configuration files that power my homelab.

[tailscale](https://tailscale.com/) is used as a mesh VPN between my devices.

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

# License 
MIT 


