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
RAFAEL_CHAT_MODEL_NAME=gemma-everyday
RAFAEL_CHAT_MODEL_API_KEY=
RAFAEL_CHAT_MODEL_TIMEOUT_SECONDS=300
RAFAEL_CHAT_MODEL_LIST_TIMEOUT_SECONDS=5
RAFAEL_CHAT_SYSTEM_PROMPT=
RAFAEL_CHAT_WEB_SEARCH_PROVIDER=disabled
RAFAEL_CHAT_SEARXNG_BASE_URL=
RAFAEL_CHAT_BRAVE_SEARCH_API_KEY=
RAFAEL_CHAT_WEB_SEARCH_TIMEOUT_SECONDS=15
RAFAEL_CHAT_WEB_FETCH_TIMEOUT_SECONDS=15
RAFAEL_CHAT_WEB_SEARCH_MAX_RESULTS=5
RAFAEL_CHAT_WEB_FETCH_MAX_BYTES=65536
RAFAEL_CHAT_WEB_TOOL_MAX_INVOCATIONS=4
```

The service stores provider settings in `config.json` and conversations under
`conversations/` inside `RAFAEL_CHAT_DATA_DIR`.

For the local llama-swap endpoint, the service reads
`RAFAEL_CHAT_MODEL_BASE_URL/models` and uses that response as the model dropdown.
If the endpoint is unavailable, saved providers in `config.json` are used as a
fallback.

## Web Tools

Chat can expose two read-only tools to models when a search provider is
configured:

- `web_search`: searches the public web and returns bounded title, URL, snippet,
  source, and date metadata.
- `fetch_url`: fetches one public `http` or `https` URL and returns extracted
  readable text.

Web tools are disabled by default. Enable SearXNG:

```sh
RAFAEL_CHAT_WEB_SEARCH_PROVIDER=searxng
RAFAEL_CHAT_SEARXNG_BASE_URL=http://127.0.0.1:8888/
```

Or enable Brave Search:

```sh
RAFAEL_CHAT_WEB_SEARCH_PROVIDER=brave
RAFAEL_CHAT_BRAVE_SEARCH_API_KEY=...
```

`fetch_url` blocks localhost, private-network, link-local, and bare local host
names, does not follow redirects, and caps fetched bytes with
`RAFAEL_CHAT_WEB_FETCH_MAX_BYTES`.

OpenAI-compatible providers are chat-enabled. Anthropic providers can be saved
now, but need a provider adapter before they can be used for chat.
