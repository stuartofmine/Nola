> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Durable Task Runtime Reference Audit

## Finding

Nola already modeled worker ownership, lease expiry, heartbeat timestamps,
retry state, and input/completion watermarks. Those values were checkpointed
but were not public Core operations, so Hosts could not use them as a reliable
lifecycle. A completed artifact could therefore remain attached to a live
process indefinitely.

The observed macOS data-Skill run illustrates the distinction:

- the workspace was created at 23:01:16;
- the Xcode build completed at 23:06:02;
- the embedded CLI release build consumed 3 minutes 34 seconds;
- every runtime artifact was written at 23:06:02;
- the outer command still waited because process completion, rather than Task
  completion, was its convergence signal.

## Reference strengths adopted

Codex treats spawned work as an owned session resource: cancellation cascades,
shutdown has a bounded grace period, and terminal events are flushed before a
task is forgotten. OpenCode exposes explicit busy/retry state, preserves a
stable child-task identity for resume, parses server retry hints, and uses
bounded exponential backoff.

Nola now applies those ideas at the shared Core boundary:

1. Claim, heartbeat, retry scheduling, and lease sweeping are typed public
   submissions with idempotent operation IDs.
2. Claim freezes the input watermark and retry-waiting work cannot be reclaimed
   before its Core-recorded retry time.
3. Heartbeats require the ownership token, monotonic time, and a non-shortening
   lease.
4. Missing Retry-After information uses bounded 2s exponential backoff capped
   at 30s; explicit retry delays remain supported.
5. Expired worker leases are reclaimed into retry-waiting or failed state based
   on the retry budget.
6. A Task with an input snapshot cannot enter a terminal state without an exact
   completion watermark and the active ownership token.

## Remaining Host work

Each platform Host still needs a small scheduler adapter that periodically
submits lease sweeps, claims ready work, heartbeats only while execution is
alive, and reports a completion watermark before releasing its process. UI
projections now retain every Core state and durability field, including retry
reason/time and watermarks, without turning heartbeat traffic into a new chat
message. The remaining platform work is the scheduler adapter itself.
