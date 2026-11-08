> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Nola Cloudflare Worker + D1 Backend

This directory is the Cloudflare-native backend implementation for Nola.
It runs as a Worker and stores relational data in D1. This is the only backend
implementation; local development uses Wrangler and a local D1 database.

## What this Worker covers

- `GET /healthz`
- OAuth relay: `/oauth/login`, `/oauth/start/{provider}`, `/oauth/callback/{provider}`, `/oauth/relay/exchange`
- Provider auth: `/v1/auth/apple`, `/v1/auth/google`, `/v1/auth/github`, `/v1/auth/me`
- Models/chat proxy: `/v1/models`, `/v1/chat/completions`
- Chat history, device registration, scheduled sources, run-now result storage, results list/detail/ack

Scheduled sources are evaluated by a Worker Cron Trigger every minute. The
scheduler evaluates `once`, `daily`, and `weekly` slots in each source's IANA
timezone, claims each source/slot atomically in D1, and reuses the same fetch,
summary, result, and push pipeline as `run_now`.

## Setup

1. Install dependencies:

   ```bash
   cd backend/worker
   npm install
   ```

2. Create D1:

   ```bash
   npx wrangler d1 create nola_backend
   ```

3. Copy the returned `database_id` into `wrangler.toml`:

   ```toml
   [[d1_databases]]
   binding = "DB"
   database_name = "nola_backend"
   database_id = "<your database id>"
   ```

4. Apply migrations:

   ```bash
   npm run migrate:remote
   ```

5. Configure Worker variables/secrets in Cloudflare:

   - `nola_backend_env` as a JSON variable or secret.
   - Keep OAuth/LLM/JWT secrets inside that JSON.
   - `DATABASE_URL` is not used by the Worker; persistence comes from the `DB` D1 binding.

6. Deploy:

   ```bash
   npm run deploy
   ```

## Cloudflare project settings

`backend/start.sh` is npm-only for the Worker runtime:

```bash
./backend/start.sh dev
./backend/start.sh deploy
./backend/start.sh typecheck
```

If your Cloudflare project's root directory is `backend` (not `backend/worker`),
use npm commands from `backend/package.json`:

```bash
cd backend
npm run deploy
```

This guarantees deployment goes through `worker/wrangler.toml` and avoids any
single Worker startup path.

For Cloudflare D1 production, deploy this directory as a Worker project
(`backend/worker`) with Wrangler.

After deployment, set provider callback URLs to the Worker domain:

- Apple: `https://<worker-domain>/oauth/callback/apple`
- Google: `https://<worker-domain>/oauth/callback/google`
- GitHub: `https://<worker-domain>/oauth/callback/github`

Set Web `OAUTH_RELAY_BASE_URL` to the same Worker origin.
