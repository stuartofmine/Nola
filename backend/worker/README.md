> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

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
