> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Nola Web Assets

`web/` contains the Web UI assets served by the Rust CLI Web backend. The
runtime backend is `cli/src/web.rs`; there is no Flask Web backend in this
directory anymore.

Start the normal development stack from the repository root:

```bash
./dev.sh
```

Or run only the Rust Web backend:

```bash
cd web/frontend
npm ci
npm run check
npm run build
cd ../../cli
cargo run -- web --host 127.0.0.1 --port 5001 --web-root ../web
```

The Web UI uses Svelte 5 + TypeScript + Vite. All workspace pages (`/chat`,
`/models`, `/model-test`, `/capabilities`, `/results`, `/dashboard`, `/data-sources`,
`/settings`, `/scheduled-actions`) and `/login` serve the compiled Svelte shell.
`frontend/src/lib/pageRoutes.ts` selects explicit, lazy-loaded page components;
URL input never becomes a dynamic module path. These routes do not render or
fall back to the old templates. `frontend/` owns UI source and tests; generated assets in
`app/static/ui/` are not committed. The Rust host still owns authentication,
CSRF verification, APIs, permissions, and durable state.

The login page posts to the existing `/api/web/login` endpoint. Anonymous clients
can fetch `/ui/assets/` via GET/HEAD to load the login UI; that exception does not
grant access to workspace pages or APIs. Build the frontend before launching the
Rust host; missing assets produce an explicit 503 instead of a legacy-page fallback.

Use `cd web/frontend && npm run dev` for UI development (API requests proxy to
the Rust host on port 5001). Production and LAN serving use the Rust host and
same-origin assets; a Node server is not needed in production.

Source migration is not runtime acceptance: an existing `app/static/ui/` build
may predate the source changes. The final verification must rebuild the frontend
and Rust host together, then exercise authentication, navigation, chat streaming,
permission decisions, reconnect/recovery and persisted conversation cache usage
through the served application. Old template tests do not verify Svelte behavior.

## Structure

```text
web/
├── frontend/               # Svelte pages, components, typed API projections and tests
├── app/templates/          # Retired templates; not a workspace runtime entrypoint
├── app/static/ui/          # Generated Vite shell and assets (not committed)
├── app/static/             # Shared assets and legacy assets pending dependency audit
├── native_browser/         # Chrome extension + native messaging host
└── native_app/             # Desktop/computer-use native host
```

## Backend Ownership

Add or change Web API behavior in Rust, not Python:

- Rust Web backend: `cli/src/web.rs`
- CLI command/config entrypoint: `cli/src/config.rs`, `cli/src/main.rs`
- Web UI: `web/frontend/`; generated output: `web/app/static/ui/`

The Rust backend serves the Svelte build and implements the `/api/*`
surface for chat, agent streaming, conversations, context reports, scheduled
actions, run results, settings, native host bridges, and mobile clients.

## Native Hosts

The Chrome and desktop native hosts remain Python scripts because they are
installed as OS/browser native messaging helpers.

```bash
cd web/native_browser
./install_host.sh --server-url http://127.0.0.1:5001

cd ../native_app
./install_host.sh --server-url http://127.0.0.1:5001
```
