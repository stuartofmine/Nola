> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Android data-source adoption status

Date: 2026-09-05

This is an implementation checkpoint for ARCHITECTURE_V2.md, not a completion declaration.

## Ownership contract

- Application owns source identity, configuration revision, validation and read lifecycle, enablement, and conversation overrides.
- The Android Host performs authorized platform/network reads and manages referenced content files.
- The UI renders Application state and submits explicit commands. Saving unrelated profile settings must not rewrite the source catalog.
- Read failure, cancellation, empty results, and unavailable authorization are distinct outcomes, not interchangeable empty content.

## Implemented and exercised

- CanonicalDataSourceClient submits explicit lifecycle commands rather than the removed source upsert contract.
- AndroidCanonicalDataSources checks the editing revision, records validation/read outcomes, and stores content through references.
- MemoryProfileDataStore source mutations no longer reconcile the whole source list through saveProfile.
- A cache-write failure before a read request does not leave a pending read. A committed read whose response is lost retains its referenced content.
- DataSourcesViewModel separates external refresh from local snapshot reload. Mutations trigger local reload, including failures that may follow a partial commit.
- Refresh completion preserves current mutation/validation indicators rather than restoring indicators from an older snapshot.
- Pending reload requests are coalesced. ViewModel disposal cancels active work and prevents stale callbacks from scheduling new work.
- MemoryProfileDataStore receives the application-scoped AndroidApplicationRuntime through dependency injection. A production-source call-site scan found no remaining direct runtime construction outside its declaration.
- Canonical conversation selection is projected from Application resolve_for_conversation results. The chat control submits one source override and publishes selection only after the command and projection read succeed.
- ContextRepository uses Application enablement/usability and checks source revisions rather than treating the conversation settings ID list as canonical-source authority.
- Default enablement no longer changes the shared source lifecycle. A validated, default-disabled source can be explicitly enabled for a conversation without changing the global default.
- Host refresh uses resolved conversation enablement, preserves the source default, and propagates caller cancellation. Its own bounded HTTP timeout remains a read failure.

## Evidence and limits

The real JNI instrumentation class AndroidApplicationRuntimeRealEndToEndTest exercises independent temporary SQLite databases and the packaged Rust library:

- applicationCommandSurvivesNativeHandleAndSqliteColdReopen: lifecycle, content restoration, stale revision rejection, enablement and deletion.
- sourceCacheFailuresDoNotLeavePendingReadsOrDeleteCommittedContent: cache directory failure and response loss injected after the real command commits.
- concurrentSourceEditorsCannotOverwriteCommittedConfiguration: two runtime handles, rejection of a stale editor, no new cache file from the rejected submission, and cold reopen.

These targeted device runs passed individually. They are not live HTTP, real-model, or rendered UI tests. The concurrent-editor case is a deterministic stale-read interleaving, not a simultaneous stress test.

DataSourcesViewModelTest currently has six passing JVM tests with no skips. They drive the production ViewModel through suspendable operation boundaries and cover refresh/mutation interleaving, validation indicators, failure recovery, cancellation, and disposal. They do not prove Compose layout, accessibility, or click behavior.

AndroidConversationDataSourcesRealTest additionally exercises the production ConversationRepository with real JNI: two conversations, explicit disable/enable/inherit, cold reopen, separation from noncanonical settings IDs, and explicit selection of a default-disabled source. Its initial failure exposed the shared lifecycle/default-enable conflation; the unchanged assertion passed after the Application fix.

AndroidSourceHttpContextRealTest runs a real loopback HTTP server, CIO client, production source repository, isolated SQLite database, JNI, and ContextRepository. It checks that an expired default-disabled source reads once after explicit selection, preserves its default, contributes exactly one platform_data component and ContextPack entry, and survives reopen. Conversation exclusion and cloud disclosure restrictions remove both the component and pack entry, even when stale selected IDs are supplied. These checks passed without LLM calls.

## Remaining architectural gaps

### Complete the conversation selection experience

The canonical source command/read/context path is adopted, but the overall catalog and interaction are not complete:

1. The repository supports clearing an override with inherit, but the chat toggle still exposes only explicit enable/disable. Provide an understandable restore-default action and its localization.
2. Document/platform catalog entries without canonical source identities still use their separate settings path. This is not a second writer for canonical sources, but catalog ownership is not fully unified.
3. Exercise actual chat controls under rapid changes, conversation switching, command rejection, and disposal. Repository/JNI tests do not establish these rendered interaction properties.
4. Verify the final model request and real-model behavior. The HTTP integration test currently ends at assembled components and ContextPack, before model-request serialization.

### Host I/O still precedes lifecycle intent

AndroidDataSourceRepository.validateAndUpsert performs the Host read before the adapter creates the validation lifecycle. Successful refresh also passes through save/configuration validation rather than solely through request-refresh and record-read.

The intended order remains: persist the relevant intent, execute the authorized Host operation, then record its outcome using the issued token. Cancellation and failure must settle that same operation without overwriting a newer attempt.

### Further incomplete areas

- Content-file reclamation after successful refresh and deletion needs a concurrency-safe policy; never delete a file merely because a command response was lost.
- Canonical enums/read state are not yet fully typed throughout the Android adapter.
- Profile-wide reconciliation of other aggregates, including Skills and MCP, remains outside the source-specific fix.
- Clear-all behavior must use explicit aggregate commands rather than rely on the removed source-list reconciliation.
- The tested HTTP-to-JNI-to-context path does not yet cover pending-operation crash recovery, concurrent network completions, final provider requests, or rendered interaction flow.

None of the passing tests above establishes completion of these remaining items or of the overall architecture refactor.
