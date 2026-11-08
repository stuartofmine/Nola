> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Initial performance and reliability evaluation

Nola ships one bounded entry point for early engineering evaluation:

```bash
python3 scripts/run_initial_evaluation.py
```

The command has a hard 600-second budget. It runs deterministic Core and Mac
product contracts first, then four real external-model requests concurrently.
Secrets are read from `~/.nola/config.json` and are never written to reports.

## What the gate measures

- Core prompt/SOP ownership and durable goal constraints.
- Goal checkpoint, task lease/resume, trace reduction, and service cold resume.
- Real service-process kill/restart recovery, lease reclaim, cooperative cancellation,
  cancellation-after-crash precedence, and terminal idempotency.
- A six-hour virtual Task with 360 heartbeat/progress transitions, hourly Core
  restoration, completion watermark validation, and exactly-once artifact replay.
- Multilingual and ambiguous semantic tool-selection contracts without keyword
  routing.
- Mac test-build cost separately from product-runtime cost.
- Model-picker readiness-cache latency and idle Mac test-host resident memory.
- Absence of periodic local-runtime status publications when state is unchanged.
- Real-model exact English and Chinese instruction following, arithmetic, and a
  schema-constrained tool call.
- First visible model delta and total request latency.

The deterministic report is written to `logs/fast-evaluation/report.json` and
`report.md`. Real-model evidence is written to `real-model.json` and
`real-model.md`. `initial-evaluation.json` records the combined budget result.

## Current baseline

On 2026-08-26, after adding the out-of-process MLX runtime and real worker
kill/recovery coverage, the warm combined gate completed in 95.211 seconds:

- deterministic reliability: 100%
- deterministic contract accuracy: 100%
- deterministic performance: 100%
- real-model accuracy: 100% across four concurrent cases
- real-model median first delta: 0.720 seconds
- model readiness lookup: approximately 9.85 microseconds per cached lookup
- idle Mac test-host RSS: 570,687,488 bytes
- Mac test-build preparation: 22.604 seconds with warm package/build caches
- Mac runtime test case: 39.220 seconds
- ordinary no-change NolaMac app build: 10.81 seconds with warm dependencies

Build/link RSS and direct runtime RSS are reported separately. Cargo linker
memory must not be described as product runtime memory.

## Process cleanup contract

Every evaluation command runs in a dedicated process session. On timeout the
runner terminates the captured descendant tree, its process group, and any
detached Xcode worker containing that case's DerivedData marker. The cleanup
path has a direct regression check using a spawned child process. A timed-out
evaluation must leave no compiler, test host, or background child running.

## Local-model process boundary

Mac model weights and KV cache now live in the bundled `nola-mlx-runtime`
helper. The Nola UI process owns only configuration, lifecycle commands, status,
and the authenticated loopback client. The ephemeral token is sent over stdin,
not exposed in process arguments. The helper monitors the Nola parent PID and
the supervisor terminates it on explicit stop. Model selection remains a pure
configuration action; only the explicit Start action creates the helper and
loads weights.

The build gate also audits that the Nola executable has no MLX links or symbols,
while the helper contains the MLX implementation. This boundary prevents an
active model's weights and KV cache from inflating UI-process resident memory
and lets a crashed or stopped model runtime release its memory independently.

When the complete Qwen3.5 0.8B fixture is already downloaded, the opt-in
`MacLocalModelRealInferenceEndToEndTests` additionally verifies the bundled
helper's loading/ready event stream, concurrent-start coalescing, authenticated
model listing, real chat generation, and explicit stop. It does not download
weights and therefore remains outside the portable initial 10-minute gate.

The current real-model cases are fast smoke evaluations, not a comprehensive
quality benchmark. Future suites must add durable multi-hour task interruption,
checkpoint recovery, cancellation, and exactly-once artifact verification while
preserving the same 10-minute initial gate.
