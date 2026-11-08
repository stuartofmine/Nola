> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

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
