# lift

Minimal workout journal for Rafael.

## Run Locally

```bash
cd services/lift/frontend
bun install
bun run dev
```

In another shell:

```bash
cd ~/rafael
DATABASE_URL=sqlite:data/lift/lift.db?mode=rwc \
LIFT_STATIC_DIR=services/lift/frontend/dist \
cargo run -p lift
```

The app stores its state in SQLite through `/api/state`. The frontend is built
with Vite and uses `/lift/` as its production base path.

## Deploy

The deploy workflow builds `services/lift/frontend`, builds the `lift` release
binary, installs `infra/systemd/rafael-lift.service`, and checks
`http://127.0.0.1:3033/health`.

Public access is intended to share the existing Rafael Funnel host:

```bash
tailscale funnel --bg --https=443 --set-path=/lift http://127.0.0.1:3033
```
