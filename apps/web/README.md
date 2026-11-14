> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Nola Web Assets

`apps/web/` contains the Web UI assets served by the Rust CLI Web backend. The
runtime backend is `apps/cli/src/web.rs`; there is no Flask Web backend in this
directory anymore.

Start the normal development stack from the repository root:

```bash
./dev.sh
```

Or run only the Rust Web backend:

```bash
cd apps/web/frontend
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

Use `cd apps/web/frontend && npm run dev` for UI development (API requests proxy to
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
├── app/static/ui/          # Generated Vite single-entry shell and assets (not committed)
├── app/static/             # Shared runtime assets (icons, Mermaid bundle, SSE frame decoder)
├── app/locales/            # Shared localization catalog
├── tests/                  # Web localization, Playwright helper and real-host integration tests
├── native_browser/         # Chrome extension + native messaging host
└── native_app/             # Desktop/computer-use native host
```

## Backend Ownership

Add or change Web API behavior in Rust, not Python:

- Rust Web backend: `apps/cli/src/web.rs`
- CLI command/config entrypoint: `apps/cli/src/config.rs`, `apps/cli/src/main.rs`
- Web UI: `apps/web/frontend/`; generated output: `apps/web/app/static/ui/`

The Rust backend serves the Svelte build and implements the `/api/*`
surface for chat, agent streaming, conversations, context reports, scheduled
actions, run results, settings, native host bridges, and mobile clients.

## Native Hosts

The Chrome and desktop native hosts remain Python scripts because they are
installed as OS/browser native messaging helpers.

```bash
cd apps/web/native_browser
./install_host.sh --server-url http://127.0.0.1:5001

cd ../native_app
./install_host.sh --server-url http://127.0.0.1:5001
```
