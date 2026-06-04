# systemd units

This directory holds user-level systemd units for Rafael.

The intended machine setup is:

- `llama.cpp` is checked out and built at `~/src/llama.cpp`.
- The server binary exists at `~/src/llama.cpp/build/bin/llama-server`.
- `llama-swap` is installed at `~/.local/bin/llama-swap`.
- CUDA is available under `/opt/cuda`.
- The local OpenAI-compatible endpoint is reachable at `http://rafael:8080/v1`.

## llama-swap.service

`llama-swap.service` is the primary local model service. It owns the public API
port `8080` and launches private per-model `llama-server` processes from
`llama-swap.yaml`.

The checked-in config defines these model IDs:

```txt
gemma-everyday
qwen3-coder
gpt-oss
qwen-coder-fim
gemma-deep
```

All five are members of the `local-models` group:

```yaml
groups:
  local-models:
    swap: true
    exclusive: true
```

That means only one model in the group can be resident at once. A request for a
different model unloads the current model before the next one starts. The config
also sets `globalTTL: 600`, so an idle model unloads after 10 minutes.

`gemma-everyday` uses `ggml-org/gemma-4-E4B-it-GGUF:Q4_K_M`.
`gemma-deep` uses `ggml-org/gemma-4-26B-A4B-it-GGUF:Q4_K_M`.

## Install or Update

Install or build `llama-swap`, then place the binary at:

```bash
mkdir -p ~/.local/bin
# install/copy llama-swap to ~/.local/bin/llama-swap
chmod +x ~/.local/bin/llama-swap
```

Install the units into the user systemd directory:

```bash
mkdir -p ~/.config/systemd/user
ln -sf ~/rafael/infra/systemd/llama-swap.service ~/.config/systemd/user/llama-swap.service
ln -sf ~/rafael/infra/systemd/rafael-chat.service ~/.config/systemd/user/rafael-chat.service
systemctl --user daemon-reload
```

Stop the legacy direct server before enabling the proxy:

```bash
systemctl --user disable --now llama-server.service
systemctl --user enable --now llama-swap.service
systemctl --user restart rafael-chat.service
```

After changing `llama-swap.yaml`, restart the proxy:

```bash
systemctl --user restart llama-swap.service
```

Keep user services running after logout or reboot:

```bash
sudo loginctl enable-linger "$USER"
```

## Operations

Check whether the proxy is running:

```bash
systemctl --user status llama-swap
```

Follow proxy and upstream logs:

```bash
journalctl --user -u llama-swap -f
```

Confirm llama-swap is exposing the model list:

```bash
curl http://127.0.0.1:8080/v1/models
```

Trigger a lazy load:

```bash
curl http://127.0.0.1:8080/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{"model":"gemma-everyday","messages":[{"role":"user","content":"hello"}]}'
```

`llama-swap` also exposes operational endpoints:

```bash
curl http://127.0.0.1:8080/health
curl http://127.0.0.1:8080/logs
curl -Ns http://127.0.0.1:8080/logs/stream
```

## Adding Models

Add one block under `models:` in `llama-swap.yaml`, then add the model ID to
`groups.local-models.members`. Keep `${PORT}` in the command so llama-swap can
assign a private upstream port.

For heavy models on the RTX 4080, keep them in `local-models` and tune
`--n-cpu-moe` with `llama-bench` before raising context or KV cache settings.

## rafael-chat.service

The checked-in unit serves the chat UI and API from this repo:

```ini
RAFAEL_CHAT_BIND=0.0.0.0:3031
RAFAEL_CHAT_DATA_DIR=%h/rafael/data/chat
RAFAEL_CHAT_WEB_DIST=%h/rafael/services/chat/web/dist
RAFAEL_CHAT_MODEL_BASE_URL=http://rafael:8080/v1
RAFAEL_CHAT_MODEL_NAME=gemma-everyday
RAFAEL_CHAT_MODEL_TIMEOUT_SECONDS=300
RAFAEL_CHAT_MODEL_LIST_TIMEOUT_SECONDS=5
RAFAEL_CHAT_AUTH_TOKEN_DAYS=30
RAFAEL_CHAT_WEB_SEARCH_PROVIDER=disabled
RAFAEL_CHAT_WEB_SEARCH_FETCH_RESULTS=3
RAFAEL_CHAT_WEB_SEARCH_FETCH_MAX_BYTES=16384
```

The chat backend calls `RAFAEL_CHAT_MODEL_BASE_URL/models` and uses that response
as the local model dropdown. If the model list is unavailable, it falls back to
saved providers in the authenticated user's chat config.

Chat requires login. Users register with an allowed first name and password;
allowed names are `Akrm`, `Nowar`, and `Sofia`, matched case-insensitively. JWTs
are signed with `RAFAEL_CHAT_DATA_DIR/auth_secret`, default to 30 days, and each
user gets isolated chat state under `RAFAEL_CHAT_DATA_DIR/users/<name>/chat/`.

Chat web tools are disabled until a provider is configured in a drop-in:

```bash
systemctl --user edit rafael-chat
```

The local SearXNG Docker Compose stack lives in
`infra/docker/searxng`. It binds to `127.0.0.1:8888` and enables the JSON API
required by the chat web search tool.

SearXNG:

```ini
[Service]
Environment=RAFAEL_CHAT_WEB_SEARCH_PROVIDER=searxng
Environment=RAFAEL_CHAT_SEARXNG_BASE_URL=http://127.0.0.1:8888/
Environment=RAFAEL_CHAT_WEB_SEARCH_FETCH_RESULTS=3
Environment=RAFAEL_CHAT_WEB_SEARCH_FETCH_MAX_BYTES=16384
```

Brave Search:

```ini
[Service]
Environment=RAFAEL_CHAT_WEB_SEARCH_PROVIDER=brave
Environment=RAFAEL_CHAT_BRAVE_SEARCH_API_KEY=...
```

Build the frontend and release binary before starting the unit:

```bash
cd ~/rafael/services/chat/web
npm run build

cd ~/rafael
cargo build --release -p chat
```

Check whether the unit is running:

```bash
systemctl --user status rafael-chat
```

Follow logs:

```bash
journalctl --user -u rafael-chat -f
```

Confirm the chat API is answering:

```bash
curl http://127.0.0.1:3031/api/state
```

From Tailscale devices, use:

```txt
http://rafael:3031
```

## llama-server.service

`llama-server.service` is kept as a direct single-model fallback. Do not run it
at the same time as `llama-swap.service`; both use port `8080`, and the
llama-swap unit has `Conflicts=llama-server.service`.

Rollback to the old direct server:

```bash
systemctl --user disable --now llama-swap.service
systemctl --user enable --now llama-server.service
systemctl --user restart rafael-chat.service
```
