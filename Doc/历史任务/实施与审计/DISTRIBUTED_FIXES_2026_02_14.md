> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# 2026-02-14 后端分布式审计与修复摘要

本文件记录 2026-02-14 在 `backend/` 目录下完成的分布式稳定性审计、主要修复与验证结果（供 PM/运维/开发快速参考）。

问题背景
- 多机部署场景下发现若干明显风险与故障：本地文件锁失效导致重复任务执行、调度器（scheduler）在 leader 宕机后 follower 不接管、启动脚本依赖当前 cwd 导致无法找到 `.env`、`.env` 含脏行会被 `source` 当作命令执行失败。

已实施的变更（简要）
- 原子任务领取/去重：更新 `backend/app/db.py`，引入确定性 job_id 与 `claim_due_jobs()` 原子领取逻辑，避免重复执行。
- 分布式锁与 leader takeover：在 `backend/app/push.py` 中改用 Redis 分布式锁（带 TTL 与心跳续约），并实现 follower 周期性抢占/接管（takeover）逻辑；scheduler runner 改为基于原子 claim。
- Region-aware keys：`sync_worker.py`、`purchases.py`、push 相关锁/consumer group/streams 的 key 默认带 `REGION_ID` 后缀，确保 CN/GLOBAL 隔离消费。
- 启动/安装脚本健壮化：重写/增强 `backend/install_deps.sh`、`backend/start.sh`、`backend/reload.sh`：
  - 按脚本所在目录解析 `.env`（绝对路径），避免从任意 cwd 执行出错；
  - 对 `.env` 做容错解析（只读取合法 `KEY=VALUE`，忽略注释与脏行），避免把非赋值行当作 shell 命令执行；
  - Postgres bootstrap 在无 admin 密码或管理员账号差异时尝试多种本地管理员候选（当前登录用户、postgres），并给出明确错误提示。
- 配置与模板：新增 `.env.cn.example` / `.env.global.example` 与生产草案（占位密钥保留），并将 Redis consumer group、lock key 的命名规则改为自动带 region 后缀。
- Smoke 测试增强：扩展 `scripts/smoke_test_backend.py`，覆盖更多端点与失败反向断言；执行单实例与双实例（8001/8002）并发验证与强制 leader 切换测试，均通过验证（takeover 成功）。

验证与当前状态
- 验证通过项：单实例启动与 smoke 测试；双实例并发场景只有一个实例持有 scheduler lock；leader 被 kill 后 follower 在短时间内成功接管。
- 待用户操作：替换 `.env` 中的占位密码（如 `REPLACE_ME`），在生产环境使用 PostgreSQL 并通过 Secret 管理器下发 `OUTBOX_SIGNING_KEY`、`ADMIN_API_KEY` 等敏感值。

建议与下一步
- 在生产启用监控/告警（DLQ 长度、签名校验失败率、scheduler lock 丢失）；
- 在 k8s 中考虑使用 leader election（例如基于 ConfigMap/Lease 或 Redis）来守护 publisher 单实例；
- 在部署前替换所有 `.env` 占位值并在目标服务器运行 `sh install_deps.sh` 完整验证（脚本已支持 `--no-write-env` 等选项）。

相关文件（局部示例）
- backend/app/db.py
- backend/app/push.py
- backend/app/sync_worker.py
- backend/app/purchases.py
- backend/install_deps.sh
- backend/start.sh
- backend/reload.sh
- scripts/smoke_test_backend.py
