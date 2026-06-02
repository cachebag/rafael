# systemd units

This directory holds user-level systemd units for Rafael. At the moment that is
just `llama-server.service`, which runs the local `llama.cpp` OpenAI-compatible
server used by the rest of the repo.

The intended machine setup is:

- `llama.cpp` is checked out and built at `~/src/llama.cpp`.
- The server binary exists at `~/src/llama.cpp/build/bin/llama-server`.
- CUDA is available under `/opt/cuda`.
- The model endpoint is reachable at `http://rafael:8080/v1`.

## llama-server.service

The checked-in unit defaults to:

```ini
LLAMA_MODEL=Qwen/Qwen2.5-Coder-14B-Instruct-GGUF:Q4_K_M
LLAMA_HOST=0.0.0.0
LLAMA_PORT=8080
LLAMA_CTX=16384
LLAMA_GPU_LAYERS=999
LLAMA_INSTALL_DIR=%h/src/llama.cpp
```

`services/coding` uses the same default model settings:

```txt
RAFAEL_MODEL_BASE_URL=http://rafael:8080/v1
RAFAEL_MODEL_NAME=Qwen/Qwen2.5-Coder-14B-Instruct-GGUF:Q4_K_M
```

## Install or Update

Install the unit into the user systemd directory:

```bash
mkdir -p ~/.config/systemd/user
cp infra/systemd/llama-server.service ~/.config/systemd/user/llama-server.service
systemctl --user daemon-reload
systemctl --user enable --now llama-server
```

After changing the checked-in unit, copy it again and restart:

```bash
cp infra/systemd/llama-server.service ~/.config/systemd/user/llama-server.service
systemctl --user daemon-reload
systemctl --user restart llama-server
```

## Local Overrides

Keep machine-specific tweaks out of the tracked unit by using a systemd drop-in:

```bash
systemctl --user edit llama-server
```

Example override:

```ini
[Service]
Environment=LLAMA_MODEL=bartowski/Llama-3.2-3B-Instruct-GGUF:Q4_K_M
Environment=LLAMA_HOST=127.0.0.1
Environment=LLAMA_PORT=8080
Environment=LLAMA_CTX=8192
Environment=LLAMA_GPU_LAYERS=999
Environment=LLAMA_INSTALL_DIR=%h/src/llama.cpp
```

Reload and restart after editing:

```bash
systemctl --user daemon-reload
systemctl --user restart llama-server
```

## Operations

Check whether the unit is running:

```bash
systemctl --user status llama-server
```

Follow logs:

```bash
journalctl --user -u llama-server -f
```

Confirm the OpenAI-compatible API is answering:

```bash
curl http://127.0.0.1:8080/v1/models
```

If the service fails immediately, check that `~/src/llama.cpp` exists, that the
server binary has been built, and that the CUDA paths in the unit match the
machine.
