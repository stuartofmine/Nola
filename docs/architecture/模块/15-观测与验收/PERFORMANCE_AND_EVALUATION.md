> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Performance, Long Tasks, and Evaluation

## Design decisions

Core/agent engine decides semantic work; Application accepts it and Store commits durable product facts. Clients render Core projections and Hosts execute concrete effects. A client may retain an operating-system handle while an effect is running, but that handle is not a second Task or Goal record.

The performance budget is spent in this order:

1. Keep UI evaluation pure and bounded. No process discovery, network probe, model-file validation, or database scan belongs in a SwiftUI/Compose render path.
2. Cache stable catalogs and readiness snapshots. Refresh on an explicit invalidation event or a bounded TTL, never by rebuilding a page every second.
3. Keep prompt prefixes stable. Product policy is composed by Core; Hosts contribute capability facts and surface-specific instructions rather than copies of product policy.
4. Promote only genuinely long work. Foreground operations remain foreground. A promoted Task has a durable Core identity, lease, heartbeat, checkpoint, retry policy, cancellation scope, and bounded guidance queue.
5. Keep model context bounded. The current step and recent evidence belong in the Turn; the complete transcript, artifacts, Task state, and Goal acceptance evidence stay in canonical storage.

Xcode may invoke the Rust build phases to evaluate freshness, but the scripts compare the requested artifacts with relevant Rust sources and workspace manifests before calling Cargo. The runtime embedding phase declares concrete inputs and outputs and uses Xcode dependency analysis. A Swift-only edit must not perform another Cargo release build.

Mac local inference follows the same process boundary as other heavy runtimes. `Nola` owns configuration, lifecycle, status, and an authenticated loopback client. The bundled `nola-mlx-runtime` helper exclusively owns MLX modules, model weights, KV cache, generation, and its listener. Startup configuration and the ephemeral bearer token travel over stdin rather than process arguments. The helper watches its parent PID, and the app supervisor terminates the child on stop or app exit. Selecting a model never starts it.

## Reference comparison

The design follows useful boundaries observed in the bundled reference projects without copying their implementation:

- Codex keeps model discovery behind a shared manager with refresh strategies and a cancellable TTL worker. UI reads a snapshot; it does not own refresh work.
- Codex represents compaction as a session task and separates task cancellation from presentation.
- OpenCode keeps session processing, Task execution, and compaction in durable services. Its anchored summary preserves goal, constraints, progress, decisions, next steps, and relevant files.
- Claude Code's background-job prompt uses explicit completion, input-needed, and failure signals. Nola represents these as typed Core state instead of parsing prose markers.

## Long-task contract

A long task is correct only when all of these hold:

- Goal objective, user constraints, acceptance evidence, and remaining scope are durable.
- Task identity survives process and client restart.
- Exactly one worker owns a valid lease at a time.
- Progress is checkpointed at recoverable boundaries before the next step begins.
- Tool results stored in model context are bounded; full results remain addressable as artifacts.
- Retry uses the recorded failure and policy. An unchanged failure is not retried indefinitely.
- A per-run time or token budget pauses an execution slice; it does not silently complete or discard the Goal.
- Completion is an audited state transition backed by acceptance evidence.

CLI currently retains an in-process map for cancellation and guidance delivery. That map must be treated only as an ephemeral handle registry. Current task admission/recovery uses Application and runtime-task ports; `runtime_control.rs` remains process-local interaction/cancellation, not the persistent Task record. Inspect each producer/consumer before claiming full recovery coverage; an old migration paragraph is not proof that these current persistent ports are absent.

## Fast evaluation

Run:

```bash
python3 scripts/run_fast_evaluation.py
```

The default suite has a hard 540-second global deadline. It emits `logs/fast-evaluation/report.json` and `report.md`.

The initial deterministic score covers:

- accuracy: shared Prompt/SOP invariants and semantic capability selection, including multilingual and ambiguous requests;
- reliability: Goal checkpoint replay, Task lease/retry/resume, and real service cold restart;
- performance: elapsed time and direct test-runtime peak resident memory for every case, plus trace reduction cost;
- speed: total wall time against the global budget.

The report separates direct test-runtime RSS from Cargo build/link RSS. Compiler memory is still reported because it matters to developer experience, but it must not be presented as Nola runtime memory.

The report does not claim model quality or platform UI quality. Those require separate bounded real-model and UI suites with provider traces, screenshots, canonical persistence audits, and explicit environment metadata. Compilation is also reported separately because it proves buildability, not runtime correctness.

## Release gates

- Default fast evaluation finishes within 10 minutes on the supported development machine.
- No case times out or exceeds the global budget.
- Reliability and accuracy scores are 100 for deterministic contracts.
- UI model pickers display cached snapshots and perform no periodic file validation or process launch.
- A real long-task scenario must survive Host termination, recover fenced ownership and resume from a committed boundary. Its canonical result is sealed once; external effects require idempotency evidence or explicit unknown-result reconciliation.
- Cancellation must win over automatic retry after a worker crash. A cancelled queued task is never claimable; lease expiry fences a worker but does not alone prove a running effect stopped. Recovery records confirmed or unknown effects before sealing with the stated limits.
- The Mac UI process must not link MLX. The model helper is embedded as a separate executable and is the only process that owns MLX symbols and active model memory.
# macOS memory measurement

Do not use the RSS of an XCTest-hosted Debug app as the product memory KPI.
That process includes the test bundle, XCTest frameworks, Debug Swift metadata,
and shared file mappings. Measure a standalone Release app after a stabilization
window and report `phys_footprint` for Nola, Agent Host, service, and local-model
helper processes separately. Conversation-count regressions must use a fixed
database fixture and verify that only the selected transcript is resident.

Measure an already-built Release product with:

```bash
python3 scripts/measure_mac_release_memory.py \
  --app /path/to/Release/Nola.app \
  --output <evidence-dir>/nola-mac-release-memory.json
```

`<evidence-dir>` 表示本地验收产物根目录；后面的路径相对该目录，产物不随公开仓库分发。
