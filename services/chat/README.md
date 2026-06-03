# rafael chat

A very simple chat interface for local and OpenAI-compatible model endpoints.

## Commands

```sh
cd services/chat/web
npm install
npm run build
```

```sh
cargo run -p chat -- check-config
cargo run -p chat -- serve
```

For frontend development:

```sh
cargo run -p chat -- serve
cd services/chat/web
npm run dev
```

The Vite dev server proxies `/api` to `http://127.0.0.1:3031`.

## Environment

```sh
RAFAEL_CHAT_BIND=127.0.0.1:3031
RAFAEL_CHAT_DATA_DIR=/home/cachebag/rafael/data/chat
RAFAEL_CHAT_WEB_DIST=/home/cachebag/rafael/services/chat/web/dist
RAFAEL_CHAT_MODEL_BASE_URL=http://rafael:8080/v1
RAFAEL_CHAT_MODEL_NAME=Qwen/Qwen2.5-Coder-14B-Instruct-GGUF:Q4_K_M
RAFAEL_CHAT_MODEL_API_KEY=
RAFAEL_CHAT_MODEL_TIMEOUT_SECONDS=300
RAFAEL_CHAT_SYSTEM_PROMPT=
```

The service stores provider settings in `config.json` and conversations under
`conversations/` inside `RAFAEL_CHAT_DATA_DIR`.

OpenAI-compatible providers are chat-enabled. Anthropic providers can be saved
now, but need a provider adapter before they can be used for chat.
