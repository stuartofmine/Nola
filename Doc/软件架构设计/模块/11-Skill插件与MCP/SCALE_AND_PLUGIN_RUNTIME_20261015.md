> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Scale and plugin runtime

Status: active implementation, not accepted. Existing Mac/research fixes remain
in the worktree; none of this work authorizes altering production conversations.

## Goal and acceptance scope

1. Compare pi, Codex and OpenCode storage and extension implementations.
2. Measure the recent conversation workload at 1x, 10x and 1000x turns/tools.
   Preserve operation mix, body sizes, permission waits, failures, and replay;
   multiplying text bytes alone is not the requested test.
3. Optimize measured bottlenecks without sacrificing atomic publication,
   crash recovery, permission provenance or bounded reads.
4. Add usable plugin installation, listing, activation, disablement and removal,
   composing existing Skills and MCP rather than bypassing Core authorization.
5. Keep investigating research convergence, inaccurate reported search counts,
   and duplicate Skill activation revealed by the preceding live acceptance.

## Reference findings (local source, not claims about latest upstream)

- pi `packages/durable/src/testing/storage-benchmark.ts`: explicit scales,
  separate task/document counts, bounded replay tails and fork-depth workloads.
  `storage/sqlite/database.ts`: transaction-handle-only operations and rejection
  after rollback, not merely an async facade hiding synchronous work.
- Codex `codex-rs/state/src/runtime/threads.rs`: cursor ordering and query-plan
  considerations, including avoiding global timestamp scans on filtered lists.
  `codex-rs/rollout/src/recorder.rs` uses a bounded 256-command writer channel,
  flush acknowledgments and pending-item recovery rather than unbounded enqueue.
- OpenCode `packages/opencode/src/storage/db.ts`: WAL, bounded busy timeout,
  explicit cache budget and transaction boundary. Its NORMAL durability choice
  is not automatically appropriate for Nola's acknowledgment guarantees.
- pi `packages/coding-agent/docs/extensions.md`: executable tools, commands,
  lifecycle hooks, MCP registration, and explicit session shutdown. Extensions
  run with process privileges; this must not be mistaken for sandbox isolation.
- Codex `codex-rs/core-plugins/src/manifest.rs`: a plugin composes named/versioned
  resources, Skills, MCP, hooks and presentation metadata; paths are validated
  relative to the package root before resolution.
- OpenCode `packages/opencode/src/plugin/index.ts`: load/registration lifecycle,
  typed hooks and plugin identity; not just a second tool-description catalog.

## Current Nola gaps

- Existing `canonical_stream_performance_tests.rs` explicitly excludes Core
  checkpoint serialization and focuses on one growing body. Passing it does
  not prove a large conversation remains responsive.
- Journal change pagination uses sequence-filtered DISTINCT part IDs followed
  by per-part version lookups. Measure repeated-page sorting/scanning and
  statement preparation before choosing a different index or cursor contract.
- MCP already has application/store commands, revision checks, HTTP/stdio
  hosts and tool discovery. No unified plugin package ownership/lifecycle was
  found in the inspected entry points. Reuse those boundaries.

## Performance test matrix and provisional budgets

All measurements use isolated disk-backed databases and include hardware,
build profile, corpus dimensions, p50/p95/p99, database/WAL bytes and peak RSS.
Absolute numbers are provisional until baseline measurement; algorithmic
regressions are checked through query plans/VM steps as well as elapsed time.

| Boundary | Required measurement | Acceptance target |
| --- | --- | --- |
| Store journal | Sustained per-batch commit, simultaneous fixed-snapshot reads | p95 commit <50 ms, no missing/duplicate parts |
| Application/Core | Model/tool transitions, growing checkpoint and outbox | p95 persistence <100 ms excluding external effects |
| Query | Recent/old message pages, delta pages, tool output ranges | p95 bounded page <100 ms; no total-history materialization |
| Recovery | Close/reopen, interrupted write, retry same command | Atomic/idempotent result; bounded replay from checkpoint |
| UI | Switch sessions, live updates with deep history | No full-history load; maintain interaction and scroll state |
| Saturation | Slow reader/writer, WAL growth, cancellation | Bounded queue/memory; explicit overload, no dropped acknowledged writes |

The initial ignored Store benchmark scales 16 bodyless activity heads per
batch to 16,000, uses one durable transaction per batch, and reports change-page
latencies. It is an isolation probe, **not** the full 1000x acceptance workload.
Follow it with realistic tool bodies, many turns, concurrent readers and
Application/Core transitions. Do not silently replace that requirement with
the easier Store probe.

## Plugin boundaries and lifecycle

- Domain: versioned manifest and plugin identity; declared Skills, MCP servers,
  executable entry points and requested host capabilities. Manifest data never
  itself grants execution authority.
- Application: inspect -> install disabled -> explicit enable -> activate;
  revision-checked configure/disable/remove. Return precise failure and receipt.
- Store: package identity, content digest, revision, ownership of contributed
  resources, enabled intent and activation diagnostics. Credentials are secret
  references, not manifest values or synced plaintext.
- Host: validate canonical package paths (including symlinks), start owned
  processes/connections lazily, enforce timeout/cancellation/output budgets,
  revoke new calls on disable and drain/terminate owned resources deterministically.
- Core: namespaced capability IDs and typed results under the existing policy;
  plugin discovery and activation cannot select tools by prompt keywords.
- Clients: install/inspect/enable/disable/remove through the same Application
  commands; show requested permissions, contributed tools and connection errors.

Start with local packages containing Skills and MCP contributions plus an
executable MCP entry point. This is a composable transport contract, not an
in-process arbitrary JavaScript loader. Network package acquisition requires
digest/provenance validation and must not run installation scripts implicitly.
In-flight calls retain their admitted owner/version; removal cannot orphan or
silently redirect an accepted tool invocation.

## Required plugin tests

Valid package end-to-end tool discovery/call; malformed/unknown manifest fields;
duplicate IDs; traversal/symlink escape; install has no execution side effects;
missing credentials; failed startup; capability permissions; disable during a
call; deterministic cleanup; restart/recovery; uninstall leaves unrelated MCP
servers intact; multiple clients observe the same revision. Real-model use
must demonstrate schema-selected plugin tools, not an injected forced call.

## Progress ledger

- Reference entry points inspected; initial architecture and acceptance scope recorded.
- Added and ran Store-only 1/10/1000 activity-history measurement (Mac arm64,
  unoptimized Rust test build). At 1000x: 16,000 heads, 125 delta pages,
  write-batch p50/p95 2.678/16.986 ms, page p50/p95 3.914/4.509 ms,
  DB 50,909,184 bytes, WAL 61,849,472 bytes. At 1x: write 4.974 ms,
  page 0.964 ms. Small samples at 1x are not meaningful tail percentiles.
- Removed per-row preparation of the identical part-version query in delta
  pagination; it is now prepared once per page. All nine journal-read tests
  passed including the scale probe, fixed snapshots, tombstones, reopen,
  payload budgets and 2048-version query-step invariants. Post-change 1000x
  page p50/p95 was 3.268/4.139 ms. This run shared the process with other tests:
  it is a regression sanity check, not a controlled speedup claim. Peak RSS,
  WAL stabilization and cold-start latency still need dedicated measurement.
- Full workload fixture, performance optimizations, plugin implementation and
  cross-client acceptance remain open. Do not mark the goal complete from this document.

### Plugin contract implementation

`nola_domain::plugin` now defines the strict version-one package manifest,
Skill paths, stdio/streamable-HTTP MCP contributions, explicit packaged versus
host executables, credential references, and namespaced resource identities.
Decode is byte-bounded; validation rejects duplicate IDs, unknown fields,
unsupported schemas, lexical path escape, malformed environment references,
embedded endpoint credentials and unbounded arguments/resources. Five focused
tests pass. HTTP LAN endpoints remain expressible; actual network admission
belongs to the existing Host/Core policy.

This is not a sandbox or an installer. Host canonical-path/symlink validation,
immutable package copying/digest checks, durable installation/ownership state,
activation/cleanup and client commands remain to be implemented. No plugin
code is loaded or executed by manifest parsing. The manifest intentionally
contains neither an enabled flag nor a permission-grant field.

### Package snapshot Host and CLI inspection

`nola_plugin_host` captures up to 4096 entries/64 MiB, validates declared
resources, and hashes sorted paths, file bytes and executable bits. Unix
content reads use directory-relative `openat` with `O_NOFOLLOW` for every
component. Symlinks and nonregular content are rejected. Captured bytes can
be staged under a private temporary directory without rereading mutable source
content or executing package code. Staging is not installation, activation,
durable directory publication, or a runtime sandbox. Non-Unix secure inspection
currently reports unsupported rather than silently using an unsafe fallback.

CLI `nola plugin inspect <directory>` exposes this read-only operation and
reports manifest/digest/byte count with `installed: false`, `executed: false`.
It is classified read-only to avoid the normal stale-run cleanup side effect.
All seven CLI help variants list the command. `cargo check -p nola_cli --lib`
passed. Four Host tests cover capture-vs-source mutation, digest determinism,
symlink/missing-resource rejection, executable permission retention and no
execution during inspection/staging. CLI parser regression passed. The CLI
test link emitted the existing large `__eh_frame` warning; no warning-free
build is claimed. No production configuration or conversation was modified.

Next implementation boundary: persistent Plugin Application commands and
resource ownership, followed by disabled install -> explicit enable -> MCP
activation. This work does not yet provide that end-to-end plugin lifecycle.

### Durable installation intent

Added `ApplicationCommand::Plugin` and a narrow `PluginRepository` backed by
`plugin_installations`. Commands cover install, get, bounded keyset list,
revision-checked enable/disable and removal. Installation always starts disabled;
an enabled package must be disabled before replacement. Removal leaves a
tombstone, and Get exposes its revision for checked reinstall. Registry writes
require Application's durable command admission; this is still intent, not
proof of Host activation or runtime permission.

Schema version is now **74** (epoch unchanged). In accordance with the existing
strict no-upgrade policy, version 73 is rejected without modifying its data.
Do not launch this development build against the user's existing database or
silently recreate it. Tests use fresh temporary databases. Actual deployment
and any explicit archive/recreate action remain separate from this goal's code
changes. No production database was edited.

Compile checks for Store/Application passed. Lifecycle receipt tests cover
idempotency, restart, stale revisions, active-package replacement rejection,
removal/reinstall and bounded read queries. Both lifecycle tests passed after
the final rerun. The Store test rejecting old schema versions without changing
stored facts also passed, including explicit coverage of version 73.
### Package publication and management commands

Host now publishes captured bytes by staging, syncing files/directories and
renaming into the content-addressed package directory before registry admission.
Repeated publication verifies the existing destination; a modified destination
is rejected rather than silently repaired. Failed admission can leave an
unreferenced package; automatic garbage collection is not implemented.
All five Host tests passed, including publication idempotency and tampering.

CLI now exposes inspect/install/list/enable/disable/remove. Enable rechecks
package digest and manifest against the durable record. List is bounded to 128
records and returns `next_after`; `plugin list --after <id>` retrieves the next
page, avoiding silently truncated inventories. Help lists these commands in all
seven variants. These commands persist intent; they do not yet activate tools.
The CLI library compile check and all three `plugin_`-filtered tests passed.
The test linker still emits its large `__eh_frame` warning; it is not resolved
by this work. `git diff --check` passed. Production data was not accessed.

Remaining plugin work: runtime digest verification, contributed MCP/Skill
ownership, credential resolution, disable/removal cleanup and other client
surfaces. Plugin-derived servers must not be merged into the editable manual
MCP catalog: its reconciliation would otherwise save or delete owned resources
as independent manual entries. Runtime projection must keep ownership separate.
The registry does not yet start an MCP server and must not be described as
completed plugin support.

### Pinned runtime resource preparation

`nola_plugin_host::prepare_runtime` now rejects disabled/removed records before
credential access, verifies digest and manifest, and stages the captured bytes
into an independently owned runtime snapshot. Its handle retains the snapshot
until dropped. Skills have namespaced IDs; MCP configuration uses direct argv,
explicit executable resolution, and instance-qualified connection references so
two preparations of the same installed revision cannot overwrite each other's
connection entry. Credentials are resolved only into Host configuration; the
handle intentionally implements neither Debug nor Serialize, and resolver errors
are redacted. HTTP bearer values reject control/whitespace injection.

Nine Host tests pass, including unchanged runtime bytes after installed content
changes, snapshot cleanup, parallel-instance separation, disabled/removed/tampered
rejection before credential access, failed-preparation cleanup, and stdio
preparation without execution or shell interpolation. `git diff --check` passes.

This API is not yet connected to the live Host. The caller must retain the handle,
revalidate installation identity before admitting calls, and stop/drain owned MCP
connections before dropping it. Existing transport registries have registration
but no explicit owner-scoped teardown; implementing that lifecycle is the next
integration boundary. The private snapshot is not an OS sandbox and cannot
prevent deliberate mutation by code running with the same user's privileges.
Full workload scale tests and real-model plugin acceptance remain outstanding.

### MCP ownership and teardown integration

Added worker-owned `PluginMcpRuntime`: lazily registers prepared resources with
existing transports, discovers typed descriptors (never returns launch secrets),
and reconciles authoritative installation revision/enabled state. Failure or
Drop retires owned connections before dropping the package snapshot. Both
transports now support exact-reference revocation and a drain token. All owned
entries are removed from admission before waiting for any in-flight request;
draining holds no global registry lock. A retired entry cannot lazily reconnect.
The synchronous drain respects existing transport deadlines and must not run on
the UI thread; shorter cancellation/forced-shutdown budgets remain future work.

An isolated real Python MCP process test passes: initialize, list tools, call
with Chinese arguments, disable one instance, verify its PID is killed/reaped,
and continue using the other instance. Drop cleans the remaining instance and
both private snapshots. A deterministic held-request test proves admission is
revoked before drain and unrelated registration remains available. All six
stdio transport tests pass, including cancellation, timeout and mutation-crash
non-retry. The existing large `__eh_frame` linker warning remains.

This lifecycle API is not yet wired into ordinary conversation startup or
cross-client management. Callers must fetch fresh installation state before
admitting plugin calls; a cached record is not authority. Credential vault
binding and real-model schema-selected plugin acceptance remain open.

Additional scale investigation found `load_conversation_tail_snapshot` scans all
visible messages and inspects provider-message bodies to count user turns before
loading its bounded tail. A small returned page therefore does not establish
bounded work. The multi-turn workload must measure this path and its exact-count
semantics before introducing maintained counters or projection changes.

### Multi-turn/tool tail probe and publication-time classification

Added ignored `conversation_tail_thousandfold_tool_history_probe`: disk-backed
1/10/1000 user turns, 16 tool call/result pairs per turn, distinct tool result
bodies of 1/4/16 KiB, 34 messages per turn, one commit per turn, 16-message tails,
three read samples, close/reopen and exact message/user-turn checks. This directly
populates Store facts and settles normal publication transactions; it is NOT a
Core/harness checkpoint or real-model test and does not yet include concurrent
readers or peak RSS. At 1000x the fixture has 34,000 messages and 16,000 calls.

Before optimization (Mac arm64, unoptimized test build), 1000x tail samples were
29.378/29.437/29.395 seconds; reopen plus tail was 29.571 seconds. Per-turn write
p50/p95 was 4.734/7.366 ms, DB 143,400,960 bytes. Data assertions passed but
performance failed decisively. At 1x reads were ~32 ms and at 10x ~290 ms.

Tail selection now walks the ordinal index backwards instead of OFFSET-skipping
the history. A VM-step test compares 40 versus 40,000 messages and proves the
16-message selection stays bounded (absent long runs of hidden messages).
Separately, provider tool-result classification is now a durable derived fact
computed in the existing publication transaction. It shares canonical content
generation invalidation, but is stored separately from disclosure permission.
Reads use facts only for matching published generation and message/part states.
Dirty/invalid/unpublished or streaming cases retain exact original parsing;
there is no stale-cache fallback on corruption. Reopen, body mutation (inline
and sealed), rollback, and changed classification have regression coverage.

Schema version is now **75**: both 73 and 74 are rejected without migration or
data recreation. Continue using isolated databases, never the user's production
database for these test builds.

After optimization at 1000x: tail samples 92.243/91.435/91.225 ms, reopen plus
tail 94.039 ms; write p50/p95 33.383/37.185 ms; DB 144,351,232 bytes. This moves
classification cost to changed content at publication rather than repeating it
over all historical bytes. The scale run shared its process with seven short
semantic regressions (all passed); early 1x/10x timings were contended and are
not used for speedup claims. Exact count aggregation still visits message/part
metadata, so this is not an O(1) count implementation. Core checkpoints,
streaming provider bodies, parallel readers, cancellation and UI remain unproven.
Full `cargo test -p nola_store --lib` subsequently passed: 411 passed, zero
failed, three ignored (scale probes are opt-in). The tool-history probe was run
explicitly above. `git diff --check` also passed. No production database opened.

### Concurrent reader/writer acceptance probe

Added a two-connection, disk-backed reader/writer test using the same 34-message,
16-tool-result turn fixture. A normal 20-turn regression checks snapshot
consistency; an opt-in 1000-turn probe writes continuously while another
connection reads a 16-message tail, with 5 ms between requests. Every observation
must contain complete publication batches, agree on user-turn/message counts,
advance monotonically, and end at the exact last visible message. After all
calls return, WAL truncation must succeed and leave zero WAL bytes. These are
ordinary short-lived readers, not a pinned export or a stalled transaction.

Before the next micro-optimization, the serially invoked 1000-turn concurrent
probe made 915 reads: read p50/p95/p99 24.130/85.203/93.982 ms, write
35.809/37.696/39.233 ms, peak sampled WAL 14,008,032 bytes, successful truncation.
The count query now skips provider classification for non-user messages and
does not allocate historical message IDs/role strings except when an original
field requires validation. Same 1000-turn probe after this change: 1086 reads,
read p50/p95/p99 20.399/63.419/69.686 ms, write 35.718/37.674/39.145 ms,
peak sampled WAL 12,368,272 bytes, successful truncation. Both runs had exact
34,000 messages / 16,000 tool calls / 1000 user turns and no partial snapshots.
Timing comparisons are local observations, not hardware-independent guarantees.

All eight non-ignored tail-classification tests passed after the change; the
1000-turn concurrent probe was run separately to avoid test-suite contention.
The short regression's parallel-suite timing is deliberately not used as a
performance result. No claim yet covers Core checkpoint serialization, paused
readers, background synchronization, actual UI, or provider tool-result streaming.

### Current priority: ordinary-conversation plugin integration

Performance expansion is paused. `CoreAgentHost::open` now loads enabled MCP
plugin installations for durable conversations through Application, in bounded
128-record pages. Each Host retains its own pinned `PluginMcpSession`; it does
not append private plugin launch configuration to the editable manual registry.
Host startup discovers typed plugin descriptors and adds them to the same
capability catalog used by normal model tool discovery and Core authorization.
Discovery currently runs during startup, so its latency is not yet optimized.

Before dispatching a plugin MCP tool/resource/prompt, Host rereads installation
authority and retires disabled, removed, or changed instances. The associated
Host descriptors/routes are removed too. Authority-read failure fails closed;
ordinary manual MCP calls do not depend on plugin authority. Manual catalogs
provided without a refreshable registry are retained during plugin refresh.
This is a pre-dispatch revision check, not an atomic cross-process lease: a
disable racing an already admitted request does not undo its external effects.

Implemented integration behavior:
- Plugin Skills share the pinned session owner, retain namespaced IDs, and load
  only their declared `SKILL.md`. Original configured/managed sources remain
  separate; an explicit empty Skill directory list still disables all Skills.
  Listing and invocation reread installation state; removed Skills cannot be
  reactivated through stale model arguments or unqualified display names.
- Host credentials use a separate `plugin_credentials` table in the private
  credential vault. Bindings are `(plugin_id, reference, package_digest)` with
  optimistic credential revisions, never endpoint keys or environment lookups.
  Changes require disable first; enable checks the package, dependencies, and
  credential bindings without launching the package. Package replacement needs
  an explicit credential rebind. Secrets never enter Application receipts.
- Turn acquisition includes enabled installation revisions in its cache key.
  Changes rebuild the Host at the next Turn boundary. Active instances revoke
  admission on the next concrete plugin call; already admitted effects are not
  retroactively cancelled or undone.
- Run identity freezes plugin revisions. Cold recovery may rebuild transport
  instances for the same revisions, but rejects substituting an upgraded,
  disabled, or removed package into an unfinished Run. Such a Run must be
  resolved/cancelled before starting a new Turn with changed plugins.
- Ephemeral sessions deliberately do not consult the user's persistent plugin
  registry. This is an isolation boundary, not fallback to a global catalog.

This scope is the common durable Host integration used by CLI/native Host
conversations; it does not add marketplace distribution or client management UI.
Discovery still runs during Host startup. A damaged package or failed MCP
initialization is an explicit initialization error, not silent capability loss.

Added regressions exercise actual packaged Python MCP subprocess discovery,
authority failure, disable/removal, distinct instance revocation, Host catalog
projection/removal, and preservation of the independent manual catalog.

### Plugin management and acceptance

Commands (credential-set reads UTF-8 from non-interactive stdin, not argv):

```sh
nola plugin inspect ./my-plugin
nola plugin install ./my-plugin
nola plugin credential-set my-plugin service-token < /path/to/private-token-file
nola plugin enable my-plugin
nola plugin list
nola plugin disable my-plugin
nola plugin credential-clear my-plugin service-token
nola plugin remove my-plugin
```

`secret_env` and `bearer_secret_ref` in `nola-plugin.json` contain references,
not plaintext secrets. Only references declared by that exact installed package
may be bound. Disabling does not delete credentials; removing tombstones the
installation and does not destroy package/user files. Replacing package bytes
does not carry old bindings forward automatically.

Opt-in acceptance: `NOLA_RUN_LIVE_PLUGIN_E2E=1 python3 scripts/live_plugin_e2e.py
--binary target/debug/nola`. The harness uses a separate `NOLA_HOME`, database,
package store, credential vault, and workspace; the production config is read
only for the selected LAN model endpoint/key. Its temporary config is removed
afterwards, while test traces remain. The fixture's MCP credential is synthetic.

First real-model run (2026-10-15) on the configured 192.168.31.18 service passed
Skill invocation -> tool discovery -> credentialed MCP call -> receipt answer,
a cold-process Chinese follow-up with a new tracking ID, and disabled-plugin
non-execution. Evidence: temporary directory `nola-plugin-e2e-i07tdo3r`.
The disabled turn settled as `incomplete`, correctly without an MCP call; it
made several requirement-reporting retries, so this does not claim optimal
refusal latency or a successful shipment lookup while disabled. The strengthened
harness additionally checks actual Skill tool events, successful terminal
outcomes for enabled calls, and an English natural follow-up containing “current”.

### Integration acceptance completed, 2026-10-15

The strengthened fixture declares its MCP dependency in `NOLA_MANIFEST.json`.
It exposed a real refresh defect: Skill dependency resolution refreshed the
discovery catalog, while the executable Host catalog and active Turn projection
retained different revisions. Updating only the session catalog was insufficient
because effect dispatch reads the active Turn projection.

The refresh response now carries both catalogs. Core validates the Host identity
and monotonic fence, then updates the session and active Turn projection in the
same transition, preserving selected model tools rather than selecting everything
newly discovered. Host accepts an already queued model request's old revision
only while its model descriptor and availability epoch are unchanged. Old tool
requests are not grandfathered. Local plugin revocation removes routes without
invalidating unrelated in-flight requests; adopting an earlier committed snapshot
cannot resurrect those removed capabilities. Cold restoration validates executable
descriptors and pending model fences before replacing driver state.

Final real-model evidence: `nola-plugin-e2e-70tmlev_` under the machine's temporary
directory. The configured `192.168.31.18:8081/v1` model completed:

- Explicit Skill dependency -> credentialed MCP request -> original receipt:
  13.4 seconds, `action_completed`.
- Fresh CLI process, same conversation, new tracking ID: 30.9 seconds,
  `action_completed`.
- English natural follow-up: 12.0 seconds, `action_completed`.
- Disabled plugin: stale invocation rejected, no additional MCP server call;
  settled `incomplete` after 44.0 seconds, not a successful lookup.

Audit records confirm tracking IDs NOLA-42, NOLA-43 and NOLA-45 reached the
fixture service. NOLA-43 was queried twice; the cold follow-up also made four
history recalls. These are remaining harness-efficiency issues, not claimed
latency improvements. Temporary model credentials were removed after the run;
production configuration and conversation storage were not changed.

Focused regression acceptance: 191 passed (Core Skill contracts 14, active Turn
projection 1, Core Host contracts 149, CLI refresh contracts 5, CLI plugin tests
7, credential vault 4, package Host 9, Application lifecycle 2). CLI build passed.
The existing macOS linker `__eh_frame` compact-unwind size warning remains.
This validates the shared Host and CLI path, not native UI interaction, mobile
plugin-management screens, or replay of interrupted external side effects.

### Follow-up: recovery contracts and fixture correction

Inspection of the full trajectory showed that the repeated cold-follow-up
lookups above were prompted by the fixture returning the same receipt for
different tracking IDs. This is not evidence of lost conversation state. The
fixture now returns a distinct receipt per ID and verifies both the actual
requested ID in the server audit and its receipt in the final task answer,
rather than merely finding a receipt anywhere in the trace.

Unexposed-tool recovery now supplies an exact `tool_search` query. Discovery
distinguishes an exact identifier missing from the current catalog from a
keyword miss or filtered-out tool. Exact misses preserve optional suggestions,
but no longer prescribe automatic substitute/retry queries. Suggestions do not
authorize a replacement action, and absence alone does not prove disablement or
the absence of every equivalent capability. No prompt keyword routing or relaxed
result-evidence checks were added.

Real-model rerun `nola-plugin-e2e-7tfomzw3` passed all four stages. First invocation
took 19.7s, cold follow-up 10.1s, English follow-up 10.5s; each queried its own
tracking ID exactly once, with no history recall. The disabled turn took 31.3s,
performed one exact discovery and one semantic discovery, made no actual MCP
call, and ended `answered` instead of retrying requirements declarations. These
are individual observations, not a latency benchmark or a guarantee that all
unnecessary discovery has been eliminated. Regression checks passed: 29 tool
search tests, one exact-missing recovery test and one Core recovery-envelope
test. CLI build and `git diff --check` passed; the linker warning noted above
remains.
