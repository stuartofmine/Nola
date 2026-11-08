> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# iOS data-source Application adoption

Status: incomplete. Audit date: 2026-09-06.

This tracks the data-source portion of `ARCHITECTURE_V2.md`; it does not redefine
the full architecture objective as a data-source-only change.

## Confirmed ownership gaps

1. Full-list persistence has been removed. `URLFetcherService.persistSource` accepts
   one changed source; deletion requires an explicit source ID from `removeSource`.
   Remaining work: editor-observed revisions and separate configuration/read commands.
2. `IOSApplicationRuntime.saveDataSource` now accepts an explicit expected revision;
   absence means create-only, not upsert. URLFetcherService carries versions from
   its initial projection and its own successful serialized commits. It does not
   silently rebase on another writer's revision. Existing contracts are still
   revalidated on each save, which remains to be separated from configuration edits.
3. `fetchEvents` still calls configuration persistence when setting refreshing,
   completing, and failing. These are read lifecycle transitions, not configuration
   edits. They must use Application read intents and token-bound completion.
4. `validateSource` performs Host I/O before recording an Application validation
   intent. First-time validation and ordinary refresh need distinct workflows.
5. `fetchEventsNetworkAndParse` gates reads on the default `enabled` boolean.
   Conversation selection must instead use Application resolution, without changing
   the global default.
6. `iosURLSource` now takes validation, default enablement and revision from the
   Application aggregate, not embedded metadata. Canonical read state still needs
   end-to-end adoption in fetching, scheduling and UI presentation.
7. Persistence and initial loads use `try?`, including deletion. Rejections can be
   hidden while the optimistic UI reports success. Surface failures and reload the
   committed projection without masking errors as empty data.
8. Network reads use `URLSession.data(for:)` without a response-body byte limit.
   Introduce streaming limits before allocating the complete body, including absent
   or misleading Content-Length and cancellation cases.

## Change in this increment

- Decode canonical `read_state` with typed statuses, tokens, content references,
  diagnostics and timestamps. Do not invent a local default for a missing state.
- Remove the obsolete `disabled` lifecycle variant. Default selection is separate
  from lifecycle and read availability.
- Add decoder coverage and strengthen the real Application cold-reopen test:
  validated configuration without a committed read must remain `never_read`.

These DTO changes are preparatory, not evidence that the workflows above are fixed.
The test execution result must be reported separately from the existence of tests.

The next increment removes implicit snapshot deletion and logs persistence failures.
It adds a two-handle, two-service SQLite regression: saving a stale list or deleting
one owned entry must preserve another service's newly added source and its revision.
Failure presentation and rollback in the optimistic UI remain unfinished.

The subsequent increment removes the list-save API entirely. All mutation sites
submit only their changed source. A second two-handle test edits A in one service
and B in a stale second service, then compares the complete committed A aggregate
after cold reopen. This isolates unrelated-source writes; it does not establish
same-source stale-editor conflict detection.

The following increment adds Application and two-service same-source conflict
tests. Service-level version tracking prevents cross-instance lost updates, but
individual editor drafts within one service also require their own observed revision.
Conflict errors are logged; automatic rebasing is intentionally absent.

Deletion now also requires the observed canonical revision. The adapter must not
fetch a newer revision to make a stale deletion succeed. Service deletion consumes
the same queue-owned version as updates and removes that version only after success.
New tests cover stale deletion rejection, a successful current-version deletion
across cold reopen, and a stale service attempting to delete another service's edit.

The editor now awaits `saveSource` and publishes confirmation only after successful
Application persistence. Failed saves retain form input and surface the error.
The awaited path carries the draft's canonical revision, not the service's latest
revision. That value comes exclusively from the Application projection and is
excluded from embedded metadata serialization. New regressions cover two drafts in
one service and the provenance of the revision. Existing fire-and-forget callers
still need migration, including failure recovery for deletion and background reads.

Actual UI execution exposed a validation projection bug: embedded source metadata
uses second-precision ISO8601 dates, while the sealed contract includes fractional
seconds. Restoring validation from that metadata invalidated the digest. Projection
now decodes the Application validation contract, preserves its timestamps and digest,
and does not reseal or trust embedded validation. A cold-reopen regression uses a
non-integral timestamp; the editor UI test must still pass before this is accepted.

## Required acceptance evidence

- Two handles/editors cannot delete or overwrite one another through list saves.
- An HTTP server observes a persisted pending intent before returning its body.
- Cancellation and failure settle only their own read/validation tokens.
- Lost completion responses preserve committed content and do not repeat Host I/O.
- Explicit conversation overrides persist and override defaults without modifying
  them; cloud privacy and source authorization remain Application decisions.
- Actual context assembly contains only committed, usable source content.
- Simulator screenshots verify loading, failure, retry and source selection states.
- Bounded-body tests exercise actual streaming HTTP, not only parser fixtures.

Do not migrate old schemas or introduce compatibility DTO fallbacks.

## 2026-09-06 implementation and evidence update

The earlier gaps above are an audit baseline, not a claim that all remain unchanged.
The following paths have now moved to Application-owned read state:

- Interactive, background aggregation, data-detail and live-context source reads submit
  `request_refresh` before Host I/O and `record_read` with the returned token afterward.
  The batch configuration-write API was removed. Validation probing is separately named
  `readSourceForValidation`; it still needs its own durable validation-intent migration.
- Read success stores a bounded JSON content artifact and commits only its reference,
  schema version, record count and observation time. Read failure and cancellation settle
  the token. Ordinary reads no longer revalidate or overwrite source configuration.
- The local RawEvent artifact currently uses JSONEncoder's deferred Date encoding
  (seconds relative to 2001-01-01 UTC) to preserve the original Double precision.
  This is a Host-local format, not a portable cross-platform canonical event schema.
- HTTP bodies are streamed with a 1 MiB decoded-body ceiling. A real TCP fixture without
  Content-Length verified cancellation before the full 32 MiB response was transmitted.
  This is transfer-bound evidence, not a process peak-memory benchmark.
- Explicit conversation source edits persist custom overrides through Application.
  Conversation settings no longer contain a second copy for registered custom sources;
  ordinary snapshot saves do not overwrite those overrides. System-source overrides
  still use the older settings path. Multi-setting edits are not yet one atomic command.
- Custom-source context consumes Application selection and content references, with a
  final selection/revision check after I/O. It no longer falls back to source history.
  Model-facing projections are bounded to 16 events per source with bounded text fields.
- Authorization must be carried from the selected canonical source. An actual Core
  payload test exposed that omitting it suppressed otherwise valid content. The adapter
  now carries the canonical state, including `not_required` for unprotected connectors.
- Core omission notices are not evidence that source content was used: omitted IDs are
  reported as dropped, while the notice itself still contributes prompt-token overhead.

Evidence is covered by the named integration tests in `URLFetcherServiceTests.swift`:
`testRealHTTPSourceOverflowDisconnectsBeforeFullDownload`,
`testRealSourceRefreshStoresContentWithoutConfigurationWrite`,
`testFailedSourceRefreshSettlesTokenAndCanRetry`,
`testCancellingRealHTTPRefreshSettlesApplicationRead`,
`testConcurrentSourceReadsSettleIndependentlyWithoutBatchWrites`,
`testConcurrentReadsOfSameRevisionDoNotCreateTwoOwners`,
`testConversationContextEditPersistsOnlyCanonicalCustomOverrides`,
`testOrdinaryConversationSaveDoesNotReplayStaleSourceOverrides`,
`testCanonicalContextReadsConversationEnabledSourceAndRestoresCache`,
`testConversationSourceSettingControlsActualCorePayload`, and
`testDisablingSourceDuringRealHTTPReadPreventsContextInjection`.
These tests use isolated SQLite stores and local files/TCP, not a live LLM.
Shared source-protection coverage is in `core/tests/protected_data_context.rs`.

Remaining work includes first-validation intent, system-source adoption, atomic
settings commands, portable content schema, content garbage collection and crash
recovery, missing-cache repair, full outbound privacy/revocation enforcement, and
broader platform/UI/performance regression coverage. The full V2 goal is not complete.
