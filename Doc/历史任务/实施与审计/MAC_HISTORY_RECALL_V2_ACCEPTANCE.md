> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Mac history recall V2 acceptance

Status: incomplete. This checklist implements the bounded-resource and canonical
transcript requirements of `ARCHITECTURE_V2.md`; it does not replace the full V2
acceptance scope.

## Ownership

- Application owns transcript queries and enforces conversation scope and limits.
- Store returns bounded canonical text evidence, not platform display models.
- Mac renders returned evidence. It must not load a full transcript to search it.
- Compaction summaries and long-term memories are not original message evidence.
- Search results retain message ID, Part ID, message ordinal and character offset.

## Current implementation gap

`MacConversationRecallBridge` uses `MacApplicationPort.searchConversationText`.
Its results include canonical source identities and bounded snippets.

`MacCanonicalConversationRepository.searchConversationAsync` now uses the same
bounded Application text query. The Mac-only full-transcript index was removed.
Inspector results preserve message ID, Part ID, ordinal and character offset;
the expanded row still displays only a bounded excerpt, not the full message.
Original-message navigation and range loading remain incomplete.

The query does not return timestamps, so the inspector no longer displays a
date for these hits. Search failures now have an explicit error state and retry
action instead of looking like empty results. The description and error/retry
strings cover all nine languages in the Mac Chat catalog. Native UI visual
acceptance is still outstanding.

## Required implementation

1. Implemented, behavioral validation pending: inspector history searches use
   the same Application query as native tool recall, without a full-load path.
2. Preserve the existing cancellation and selected-conversation checks so results
   from a previous query or conversation cannot replace current results.
3. Keep canonical source identities in the UI result model. Opening an original
   message must use a bounded window or content-range request, not full loading.
4. Distinguish search failure from an empty result. Present a localized retryable
   error without exposing provider secrets or internal payloads.
5. Show a timestamp only if the service supplies one. Otherwise use an accurate
   source/location label. Do not fabricate dates.
6. Keep summaries separately identified if summary search is offered. Do not
   silently retain the old mixed-source search contract.

## Evidence required

- Real model compaction followed by `conversation_recall` of a random original
  fact: assert the tool result's structured evidence, not just final prose or a
  brief tool status string.
- Native bridge search for a marker near the end of ten million characters:
  bounded response, original message/Part IDs and correct character offset.
- Inspector searches across multiple long conversations: no full-transcript RPC,
  no stale results after switching, bounded resident memory after warm-up.
- Native UI screenshots and interaction observations: loading, empty, failure,
  result expansion and navigation to original evidence.
- Cold restart: original transcript remains retrievable after compaction without
  treating summaries as original evidence.
- Failure and cancellation: isolate test databases and preserve diagnostic logs.

## Current validation status

The four-test Mac run on 2026-09-07 completed with two passes, two failures and
zero skips. Its diagnostic directory is `/tmp/nola-mac-real-memory.YyCAhq`;
`result.xcresult` is the authoritative test result. Build succeeded.

- The canonical compaction/source separation and original-only native recall
  tests passed.
- Real-provider compaction/recall failed. All six recorded provider responses
  were HTTP 402, with no returned tool calls. This run does not validate real
  model compaction or recall. Investigate the provider response before retrying;
  do not change credentials or mark the test skipped/passed to hide the failure.
- The ten-million-character test failed during transcript import, before search:
  `request exceeds the command limit`. The legacy Mac snapshot importer groups
  128 messages without a byte budget and duplicates text in the canonical Part
  and lossless Mac payload. A single large message already exceeds the transport
  limit. The separate canonical importer budgets bytes but rejects oversized
  individual messages, so it is not a complete solution either.

## Newly required large-content boundary

Large-message import must use bounded content uploads and canonical references,
with Application-owned commit/finalization. Preserve message and Part identity,
exact original text, expected-message-count conflict handling and replay safety.
An interrupted upload must not expose a completed message with missing content.
Do not increase the IPC frame limit or reduce this fixture to claim acceptance.
Add interrupted-upload, retry, multibyte text and cold-restart cases, and retain
the single-ten-million-character case alongside multi-message long transcripts.

The runner now prints its artifact path at startup, preserves failed-run
artifacts, returns signal-specific exit codes and bounds xcresult summary reads
to 60 seconds. A missing-configuration failure fixture confirmed exit code 2 and
artifact retention. This does not prove that XCTest hangs or all child-process
cleanup paths are resolved.

The subsequent inspector migration and error-state changes both passed
`build-for-testing`. Five isolated native persistence/recall tests passed with
zero failures and zero skips. Evidence is in
`/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-inspector-native-9zol8wsx/result.xcresult`.
The four Rust transcript-import contract tests also passed.

Both Mac backup import paths now share encoded-byte batching and preflight
oversized individual messages before creating a conversation. This does not
implement chunked upload for an oversized single message. The separate two-test
native run for preflight and alternating long-conversation search passed both
tests with zero failures and zero skips. Its result bundle is
`/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-long-import-native-37dk5vvq/result.xcresult`.

Store original-text search now excludes reasoning, tool results, status Parts
and unfinished text Parts. Its five focused tests passed. The Mac binary used
for the two-test import run predates this last Store-only change, so that run
cannot be evidence for the new Part-kind filter. These checks do not close the
large content import gap or replace screenshot-based UI acceptance.

## Large-content implementation contract

The present `MessagePartInput` requires inline `content_text`; `content_digest`
does not provide a readable content reference. The existing `artifacts` and
`attachments` tables contain metadata, not resumable upload state. Do not treat
these names as evidence that a Blob transport already exists.

The shared implementation must introduce these operations through Application:

1. Begin an upload bound to a conversation, import operation and intended Part,
   with a declared UTF-8 byte length, per-upload quota and expiration.
2. Append a bounded UTF-8 chunk at an explicit byte offset. Exact retries return
   the existing acknowledgement; overlapping different bytes, gaps, foreign
   conversation references and writes after sealing are rejected.
3. Query acknowledged progress after disconnect or process restart. The client
   resumes from committed bytes, not from an optimistic local counter.
4. Seal only when committed bytes equal the declared length. Validation must be
   incremental; do not join every chunk into an in-memory String or JSON value.
5. Import finalized message metadata with sealed content references. In the
   same transaction, check ownership, expected message count and referenced
   content completeness, then publish the canonical message and projection.
6. Read bounded UTF-8 content ranges for rendering, export and source navigation.
   Search must support references without materializing complete messages.
7. Cancel/expire unreferenced uploads with bounded cleanup. Never collect content
   still referenced by canonical history, an active import or a durable task.

Service admission limits apply to actual encoded chunk commands, not just raw
text size; JSON escaping and multibyte text must be covered. Failed finalize
must leave canonical history unchanged and preserve resumable upload progress.
Concurrent finalizers must be idempotent or return a typed conflict, never create
duplicate messages. Restore must preserve the original Part identity instead of
splitting a large message into artificial user-visible messages.

Schema changes require an explicit data-preserving operational plan for existing
development databases. A hard-cut protocol is not permission to delete user
history or make the current application fail to open it. The end-to-end gate
remains the unmodified single-ten-million-character native test, plus process
restart, interrupted finalization, mixed-language text, ownership isolation and
bounded Service/client memory measurements.

### Upload implementation progress, 2026-09-08

- Shared protocol types now describe ownership, declared UTF-8 byte lengths,
  32 KiB chunks, progress and seal requests. Four protocol tests passed,
  including worst-case JSON escaping and ten-million-byte bounded commands.
- Service-owned SQLite staging lives beside, not inside, the existing canonical
  database. It stores chunks independently and checks contiguous byte offsets,
  exact replays, owner identity and completeness before sealing. Nine Store
  tests passed, including cold reopen, quota races, cancellation and
  ten-million-byte chunked reads.
- Application Runtime now owns begin/append/progress/seal/read/cancel operations and checks
  conversation existence without loading history. Progress is explicitly a
  Service query, bypassing durable write receipts. Querying an absent upload
  does not create a staging database.
- Five real Service child-process tests passed: restart/resume, exact replay, fresh
  progress, unchanged receipt/projection/revision counts during progress reads,
  and zero canonical messages while content is only staged. Cancellation survives
  a child-process restart, rejects foreign ownership and late writes, removes
  staged chunks and leaves pre-existing canonical messages/revisions unchanged.
- Aggregate reservations are limited to 1 GiB and 128 upload identities. Cancel
  releases body chunks and reserved bytes, but retains an owner-bound tombstone
  against resurrection. Tombstones still consume the identity quota; expiration
  and safe tombstone reclamation are not implemented. Sealed uploads cannot be
  cancelled. Released SQLite pages are reusable, not necessarily returned to the
  filesystem immediately.
- Application exposes sealed chunk reads at acknowledged UTF-8 byte boundaries;
  these are not arbitrary text ranges or canonical message reads. The isolated
  native ten-million-byte upload/read test passed at
  `/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-native-upload-e2e-kov84apt/result.xcresult`.
  This tests native IPC wrappers, not product UI or canonical publication.
- The Application Service wire version is 9, including versioned socket names
  and shared fixtures. Protocol tests (12), IPC client tests (16), and Service
  process tests (5) passed. CLI/Service all-target compilation also passed.
- The ten-million-byte IPC fixture exposed a Darwin response-read race: setting
  a socket timeout after peer close could fail despite queued response bytes.
  The client now uses nonblocking receive plus deadline-bound poll. Socket-pair
  and real child-process regressions passed; no frame limit was increased.

Still incomplete: canonical import with sealed references, arbitrary canonical
text-range reads, product UI integration, expiration, garbage collection and
fault injection at canonical finalization.
Staging uses a separate transaction domain; no claim is made that its writes and
the canonical command receipt commit atomically. Exact chunk replay/progress
recovery covers interrupted acknowledgement, but final publication still needs
an explicit atomic reference-adoption design. Do not mark the original native
single-message test as fixed based on these staging tests.
