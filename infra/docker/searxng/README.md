# SearXNG

Local SearXNG instance for Rafael chat web search.

The container binds only to `127.0.0.1:8888` on the host. `rafael-chat` should
talk to it with:

```ini
[Service]
Environment=RAFAEL_CHAT_WEB_SEARCH_PROVIDER=searxng
Environment=RAFAEL_CHAT_SEARXNG_BASE_URL=http://127.0.0.1:8888/
Environment=RAFAEL_CHAT_WEB_SEARCH_FETCH_RESULTS=3
Environment=RAFAEL_CHAT_WEB_SEARCH_FETCH_MAX_BYTES=16384
```

## First-Time Setup

Create local config and cache directories outside the repo:

```bash
mkdir -p ~/searxng/data
if [ ! -f ~/searxng/settings.yml ]; then
  cp ~/rafael/infra/docker/searxng/settings.yml.example ~/searxng/settings.yml
  secret="$(openssl rand -hex 32)"
  sed -i "s/CHANGE_ME_WITH_OPENSSL_RAND_HEX_32/${secret}/" ~/searxng/settings.yml
fi
```

Start the container:

```bash
cd ~/rafael/infra/docker/searxng
docker compose up -d
```

If an earlier ad hoc stack is already running from `~/searxng/docker-compose.yml`,
it can keep running. To move management to the repo copy:

```bash
cd ~/searxng
docker compose down

cd ~/rafael/infra/docker/searxng
docker compose up -d
```

Verify the JSON API:

```bash
curl -fsS "http://127.0.0.1:8888/search?q=rust%20tokio&format=json&categories=general&safesearch=1&pageno=1" \
  | jq "{result_count: (.results | length), first: .results[0] | {title, url}}"
```

## Chat Integration

Create or update the user-service drop-in:

```bash
systemctl --user edit rafael-chat.service
```

Then restart chat:

```bash
systemctl --user daemon-reload
systemctl --user restart rafael-chat.service
```

## Operations

```bash
cd ~/rafael/infra/docker/searxng
docker compose ps
docker compose logs -f
docker compose pull
docker compose up -d
docker compose down
```

The checked-in `settings.yml.example` enables `json` under `search.formats`.
Without that, SearXNG rejects `format=json` requests with `403 Forbidden`.

To use different host paths without editing the compose file:

```bash
SEARXNG_SETTINGS_PATH=/path/to/settings.yml \
SEARXNG_CACHE_DIR=/path/to/data \
docker compose up -d
```
