> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# V2 conversation synchronization split

Status: partially implemented and tested, not complete. This is a progress record
under `ARCHITECTURE_V2.md`, not a replacement for its all-platform acceptance scope.

## Implemented ownership

- Protocol owns `ReplicaSyncCommand`, `PendingSyncChange`, `RemoteSyncChange`,
  `RemoteSyncPage`, `RemoteSyncPageResult`, and `SyncCursor`.
- Application converts wire facts to Store inputs without cloning payload Strings.
  It commits synchronization mutations, receipts, and projection notifications in
  its existing write unit of work.
- Domain owns the bounded `ConversationMetadataReplica` contract. Kernel decides
  metadata admission. Store supplies canonical state and persists that decision.
- CLI synchronization submits Protocol commands through its Application port.
  This does not mean the entire CLI is independent of Application types yet.

## Currently published conversation fact types

### `conversation`

A full conversation content snapshot or canonical deletion tombstone. Creation
establishes an initial base. A first turn that creates its conversation publishes
that initial base. Branch publication still uses a full snapshot; immutable
content reference publication remains unfinished.

### `conversation_append`

An append publication carries its base content revision, resulting revision, and
only the newly imported messages. It does not serialize pre-existing messages.
The required tagged kind distinguishes finalized imports, new turns, and internal
resumption. Imports require finalized content. A new turn carries its user message
and empty assistant placeholder with shared Run ownership. Resumption carries a
message identity and previous/new Run identities, preserving its existing Parts.
The receiver validates the content base, message count, ordering and ownership;
it never starts the sender's Run. Missing kind tags are rejected, not interpreted
as a legacy format.

This payload still embeds new text. It is not yet an immutable Part-reference
transport; a sufficiently large new import or batch can exceed transport limits.

### `conversation_transition`

Every content-changing committed transition publishes actual new message/Part
declarations, owned Part mutations, and assistant status. Checkpoints, Host effect
requests, and unrelated history are excluded. Content bases form an ordered chain;
missing dependencies or mismatched message/Run owners roll back the received page.
Local-only runtime transitions with no content changes do not publish a content fact.

Declaration capture occurs before the canonical mutation. Initialization seeds for
already existing Parts are validated locally but omitted from the publication:
their cumulative text is not an actual insertion. Without this distinction a
2,048-character stream generated approximately 4.10 MB of replica data. After the
change the same fixture generated approximately 1.09 MB, with near-linear growth
at 512, 1,024 and 2,048 updates. Per-event overhead remains high; this is not a
claim that transport batching or large-content publication is complete.

Domain now owns the transition wire structs and shared canonical Message/Part
mutation types; Store transaction inputs reference those definitions rather than
defining a second content format. Kernel now evaluates transition base/revision,
tombstone and assistant ownership admission from supplied state. Store reads that
state and applies the result in its page transaction. Kernel also decides message
declaration identity/role admission and Part existence, ownership, kind and open
state eligibility. Store still validates actual content digests, allocates ordinals
and performs guarded SQL updates. The complete Application Port/Store adapter
separation remains unfinished; these shared decisions alone do not establish it.

### `conversation_metadata`

Settings/profile/title changes carry bounded metadata and a required content
base, not Message/Part bodies. Metadata cannot stand in for initial content,
delete a conversation, or resurrect a tombstoned identity.

A missing published base rejects the mutation rather than silently manufacturing
an empty transcript or hydrating a full snapshot during a settings update. Existing
unpublished conversations need an explicit initial-publication workflow; that
workflow is not complete. No database deletion or migration was performed here.

## Ordering, transactions, and retry

- Creation, import, and branch content publication occurs in the corresponding
  mutation transaction. Metadata publication does not load transcript bodies.
- Pending metadata and append records retain the content bases they reference.
  Full-snapshot coalescing must not discard those dependencies.
- Deletion removes pending dependent metadata/appends for that identity before
  publishing the tombstone. It does not remove another conversation's records.
- Queue coalescing has one owner: enqueue-time persistence. Database open no longer
  re-parses pending payloads to perform a second coalescing pass; it only prunes
  settled synchronization and acknowledged effect records.
- Missing dependencies reject the entire received page without advancing its
  cursor. Duplicate operations are no-ops after restart.
- Delayed content preserves newer metadata. Live replicas cannot resurrect a
  tombstoned conversation simply by presenting a larger revision.
- Received facts do not echo back into the local synchronization outbox.

## Encoded transport budget

Pending sync reads now cap both row count (512) and encoded JSON bytes (12 MiB),
leaving envelope space inside the 16 MiB IPC frame. Serialization is measured
through a counting writer rather than allocating a second encoded payload.
JSON escaping counts toward this budget. Pages preserve queue order and leave
unreturned records pending; callers acknowledge a page before reading the next.

A single oversized head returns a bounded error without skipping or acknowledging
it. This protects transport and data integrity, but does not provide resumable
large-record transfer: that queue remains blocked until chunked publication is
implemented. It is an explicit remaining architecture gap, not successful sync.

## Projection delivery behavior

`ApplyRemoteSyncPage` now emits bounded invalidation events for affected live
conversations and appropriate list changes in the Application transaction.
It does not build full before/after transcript diffs. Event identities include
conversation scope so a page touching several conversations cannot reuse one ID.
A wholly skipped/replayed page emits no refresh events.

Mac's window model already retains the requested historical ordinal on refresh.
The chat subscription loop no longer overrides that by forcing an active
conversation's historical page back to the latest page. That edit passed Swift
syntax parsing only; it has not passed full Mac type checking or UI acceptance.

## Observed verification (2026-09-08)

Additional integration evidence from the subsequent bounded-transport work:

- `python3 scripts/test_worker_replica_e2e.py --with-cli` starts an isolated local
  Worker and D1 database with generated test credentials. It runs the actual CLI
  replica Host against that endpoint and a second embedded Application instance.
  Seven large setting records and a conversation containing seven Unicode
  messages, its base, and metadata traverse multiple HTTP batches. Original text
  is compared through 32 KiB range reads, alongside bounded projections, settings,
  retry, no-echo, account isolation, and corrupt-payload rejection.
- This is not a test against production Cloudflare, nor a Mac/iOS/Android UI test.
  Embedded Application instances do not prove daemon restart recovery; separate
  Service-process tests cover a narrower local IPC restart scenario.
- HTTP upload/pull budgets are 4 MiB; the local replica IPC budget is 12 MiB.
  CLI push selects an encoded-size-bounded queue prefix and validates only that
  prefix's acknowledgement identities and revisions. Worker pull first fetches
  lengths, then a bounded prefix of immutable payloads. Neither implementation
  can yet transfer a single oversized record in chunks.
- `ConversationRecallCommand`, `ConversationTextRange`, and `ConversationRecallHit`
  now live in Protocol. SDK original-text reads validate owner, offsets, byte
  lengths and forward progress. Application applies the same range contract for
  generic RPC and embedded callers; Mac's native port also already checks these
  response invariants. Migrating these DTOs does not activate chunk references.

Measurements are individual local runs, not universal performance guarantees.

- Before metadata split: a settings-only change on a ten-million-character history
  took about 398 ms and published 10,000,456 bytes. After split: observed runs were
  about 24-36 ms, with a 209-byte metadata record.
- A first implementation moved full-snapshot cost into imports and failed the
  import memory gate. Incremental append publication fixed that regression: a
  later run appended to ten-million-character history in about 70 ms and passed
  the existing memory bound. The threshold was not raised.
- Four ten-million-character conversations initially yielded about 94 MB Service
  cold-start RSS. Removing redundant startup coalescing reduced the observed
  baseline to 13,631,488 bytes, versus 11,141,120 bytes for an empty Service.
- That run completed 68 cross-conversation recalls: p95 about 38 ms, sampled peak
  36,601,856 bytes, post-warm growth 245,760 bytes. The test now also constrains
  cold-start growth relative to an empty database, not just post-warm growth.

Evidence sources:

- `core/nola-store/tests/conversation_metadata_sync_e2e.rs`: independent sender
  and receiver databases, offline metadata/appends, missing-base rollback,
  delayed content, restart, replay, identity mismatch, and tombstones.
- `core/nola-store/src/sync_outbox_dependency_tests.rs`: queue order, dependency
  retention across restart, acknowledgement release, deletion, and owner scope.
- `core/nola-service/tests/transcript_recall_process_e2e.rs`: seven real-process
  history/upload/settings/cold-start tests; the suite passed after append split.
  The stronger cold-start case passed after removing startup coalescing.
- `core/nola-service/tests/replica_sync_process_e2e.rs`: two isolated real Service
  processes, public IPC commands, two conversations in one page, bounded refresh
  sequences, missing-base rejection, restart replay, no echo, durable confirmation.
- Protocol: 14 tests passed after DTO/command extraction. Application: its complete
  unit/integration suite passed after projection notification integration.
- CLI: both binaries passed `cargo check`; its existing replica synchronization
  test passed after moving commands to Protocol. This is not production cloud QA.
- Architecture boundary suite: six tests passed after removing the DEBUG Mac
  snapshot View's direct global Application Port access.

## Remaining work and acceptance gaps

1. Replace remaining full snapshots and large append bodies with immutable
   Message/Part references and bounded content transfer. The publication and GC
   work in `V2_CANONICAL_TEXT_PUBLICATION.md` remains necessary.
2. Make queue dependency lookup independent of large payload JSON during enqueue
   coalescing too. Removing startup re-parsing does not solve every enqueue cost.
3. Complete resumable transfer for a single oversized record. Encoded-byte batch
   limits are implemented; three real Service IPC tests passed, including ordered
   batching of two 7 MiB messages and repeated bounded errors for an escaped
   oversized head. Neither test proves single-record chunked transfer.
4. Define initial publication for previously unpublished canonical histories
   without deleting user data, guessing a baseline, or reintroducing settings-time
   full-history serialization.
5. Test the actual cloud replica endpoint and network retry/acknowledgement failure
   paths. Local two-process IPC tests do not cover production HTTP or cloud storage.
6. Verify Mac, CLI/Web, iOS, and Android notification consumption, pagination,
   localization, and rendered UI. Rust tests and Swift parsing are insufficient.
7. Complete the broader V2 client/Runtime/Host, permission, context, memory,
   artifact, cancellation, and persistence boundaries. This synchronization slice
   is not evidence that the overall architecture goal is achieved.

## Subsequent transition evidence

- Store and Application full suites passed after replacing begin/resume/transition
  full snapshots. The latest declaration-filter optimization passed the targeted
  cross-store and turn transaction suites, not a fresh full all-platform run.
- `turn_append_sync_e2e.rs` covers independent stores, restart, initial bases,
  exact content restoration, bad Run ownership rollback and idempotent redelivery.
- The real Service long-stream test now transfers a 2,048-update queue through two
  Service processes in bounded pages, restarts the receiver, compares canonical
  Message/Part rows, checks duplicate delivery and no echo, and constrains queue
  bytes and growth. It uses fixtures, not a real LLM or rendered client UI.
- The ten-million-character begin-turn process test observed approximately 42 ms,
  856 publication bytes and 34 MB peak RSS after incremental admission publication.
  These are individual local measurements, not universal latency guarantees.
