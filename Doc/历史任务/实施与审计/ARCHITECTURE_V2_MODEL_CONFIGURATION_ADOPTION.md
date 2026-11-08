> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# V2 model configuration adoption

Status: partial adoption, not architecture completion.
Evidence date: 2026-09-06.

## Ownership

Application owns model profiles and their selection. Clients submit intent and
read the committed projection. Reading configuration must not create profiles,
restore obsolete client snapshots, or silently convert storage errors into an
empty model catalog. Credentials remain a separate security concern; the
transactions below do not make Keychain writes atomic with SQLite.

## Implemented boundaries

- Mac provider configuration no longer mirrors profiles into UserDefaults or
  restores canonical rows from that mirror during reads.
- Mac and iOS selection no longer deselect the previous profile in a separate
  client command. The existing Application/Store transaction selects the target,
  displaces the previous selection, and records synchronization changes.
- `model.replace_profiles` replaces one provider/runtime scope in one savepoint.
  It rejects duplicate identities, invalid scope membership, disabled/deleted
  inputs, and nonempty sets without exactly one selected model. Empty input
  clears that scope. Failure rolls back earlier profile and outbox mutations.
- Mac provider saves use `replace_profiles` rather than a loop of upserts and
  deletions. The replacement scope is `openai-compatible` / `remote`.
- `model.reset_parameters` updates parameters for explicit profile IDs inside a
  savepoint. It reads current canonical rows and preserves their identity,
  source, selection, and lifecycle fields. A missing model rejects the batch.
- Android `ModelParametersStore.clearAll()` uses `reset_parameters` and advances
  its local refresh signal only after the command succeeds.
- Apple shared parameter saves use the parameter-only command, propagate errors,
  and publish `lastUpdated` only after success. Their UI callers retain edits
  instead of closing or publishing success after a rejected command.

These savepoints nest inside an Application write unit of work when present.
Direct Application integration tests also exercise them without an outer unit
of work. This is not evidence for full command receipt/retry correctness.

## Executed evidence

| Test | Boundary exercised | Result |
| --- | --- | --- |
| `core/nola-application/tests/model_mcp_port_e2e.rs` | Replacement rollback after a later invalid row, scope preservation, parameter rollback, cold reopen | 5 tests passed |
| `NolaMacTests/MacAppleChatCanonicalEndToEndTests.swift` | Native Mac port, replacement, selection, clearing, cold reopen | 3 tests passed |
| `NolaTests/ModelSelectionCanonicalEndToEndTests.swift` | Production selection commit method, iOS Application binding, repeated selection, parameter preservation, rejected selection | 2 tests passed on iPhone 17e simulator |
| `android/app/src/androidTest/java/com/nola/data/local/ModelParameterTransactionEndToEndTest.kt` | Production parameter store, Android JNI, rollback and database reopen | 1 test passed on Android emulator |

These tests do not call an LLM and do not prove visual correctness. Mac test
coverage of a new empty database does not by itself prove all possible obsolete
UserDefaults payloads are ignored; removal of the production fallback is a
separate code change. Cross-connection concurrent observation and outbox contents
need dedicated assertions beyond the current post-command state checks.

## Remaining gaps

1. Apple shared parameter-only saves now reject missing profiles. Mac 9-test and
   iOS 7-test native integration suites passed, including missing-profile failure
   without phantom model creation or success publication. Error presentation
   still needs visual verification. The Mac local-model adapter now updates
   existing parameters through the same narrow command. First configuration of
   an explicit catalog model registers its source with `selected=false`; that
   registration now uses `configure_parameters`: Application checks the current
   row inside a savepoint, retains existing identity/source/selection fields,
   and rejects parameter edits against tombstones. Mac's pre-read create/update
   branch has been removed. Rust 6-test and Mac 3-test suites passed.
   Its default parameters no longer read a different global Application instance.
2. iOS catalog and selection queries no longer seed defaults, persist selection,
   or bind a Host. Empty catalogs can offer read-only suggestions; a missing
   durable selection remains missing. Explicit first-run configuration and UI
   handling of this state still need full onboarding verification.
3. Profile upserts still accept full client snapshots and lack a dedicated
   selection command with an explicit concurrency contract. Parameter reset
   intentionally targets current rows; other edits need revision conflict
   handling rather than silent stale-field overwrites.
4. Provider secrets and profile changes are not one transaction. Credential
   failures, rollback policy, and user-visible retry behavior remain unresolved.
5. Provider identity must be stable and nonlocalized. iOS currently derives it
   from `ModelInfo.source`, which may be a localized display label. Do not apply
   scope replacement there before separating identity from display text.
6. CLI/Web local model operations have not adopted the new replacement/reset
   commands. Audit their actual use cases before introducing unused wrappers or
   incorrectly replacing a mixed-provider catalog.
7. Add fault injection at sync-outbox writes, nested command rollback, duplicate
   command delivery, concurrent edits, and UI acknowledgment failures.
8. Android API metadata now projects from canonical model profiles through
   `AndroidModelConfiguration`, and saves use atomic scope replacement. Two
   emulator integration tests passed. Local and official model registration
   still need adoption before removing implicit creation from individual
   parameter saves. Android UI saves acknowledge only successful writes and
   surface errors, but failure presentation and model-switch races need tests.

## Separate Mac streaming work

The duplicate-node crash was addressed through parser-state restoration,
terminal render snapshot replacement, and nonfatal animation timestamp merging.
155 parser/workspace/native-render tests passed. The real product UI test now
asserts node identity uniqueness after each completed turn and refuses a
non-temporary Application database before writing configuration.

Live multi-turn screenshots, intermediate render identity checks, and end-of-run
animation continuity remain unverified. Completed-turn assertions and static
view snapshots must not be described as proof that live flicker is eliminated.

## Model identity and sync atomicity acceptance (2026-09-06)

The common model upsert writer now rejects changing the provider/runtime of an
existing stable model ID, including deleted records. This closes the tombstone
scope takeover exposed by the new replacement regression, without changing
client list visibility or adding migration behavior.

`model_sync_atomicity_e2e` injects real SQLite trigger failures in sync_outbox.
It checks that replacement, parameter reset and parameter configuration roll
back model records and the complete observed sync queue, including after cold
reopen. Additional cases reject deleted-ID takeover and direct upsert scope
changes for both live and deleted profiles. Together with existing model port
and cross-connection visibility tests, 13 Rust tests passed:
`/tmp/nola-model-scope-fixed-20260906.log`.

Client regression after rebuilding the changed Core:

- Mac: 20 tests passed across MacAppleChatCanonicalEndToEndTests,
  MacLocalModelConfigurationCanonicalEndToEndTests and
  MacCanonicalStoreEndToEndTests. Bundle:
  `/tmp/nola-mac-model-scope-20260906.xcresult`.
- iOS simulator: 10 tests passed across ModelSelectionCanonicalEndToEndTests
  and IOSApplicationRuntimeTests. Bundle:
  `/tmp/nola-ios-model-scope-20260906.xcresult`.

These results do not establish Android/CLI/Web adoption, concurrent editing
conflict behavior, credential/profile atomicity or real-provider model loading.
Those requirements remain open rather than inheriting the Apple test result.

Android follow-up: ModelConfigurationTransactionEndToEndTest and
ModelParameterTransactionEndToEndTest passed on the isolated nola_avd Android 15
emulator through the production JNI path (2 tests). The Gradle run completed
successfully and the owned emulator was cleaned up. Log:
`/tmp/nola-android-model-scope-20260906.log`. This covers existing replacement
and parameter transaction behavior, not all Android model registration flows or
concurrent client edits. CLI/Web adoption remains open.

## Web API model catalog adoption (2026-09-23)

The Svelte API model editor now keeps a client-side draft for endpoints, models,
order, enablement, reasoning and request parameters. The user commits the whole
draft through `POST /api/model-profiles/catalog/replace`, with the complete
saved membership/revision map. The Web adapter validates the complete document
before calling Application's `model.replace_profiles` transaction; Store checks
the revisions again inside the savepoint. Invalid drafts and stale membership
do not partially update profiles or sync outbox records. The committed response
returns the complete projection so the browser does not patch one stale row.

Credentials remain a separate host-vault operation, not part of the catalog
JSON. While a catalog draft is dirty, Web disables credential editing; after
the catalog commits, the user can bind a credential to the saved endpoint.
Deleted endpoint credentials are purged after commit, and cleanup failure is
reported separately. This is still not atomic across SQLite and the vault.

The native Mac and Web catalog behavior now share the same replacement
transaction and computed default-chat route. Their visual layouts and the
credential workflow still require live cross-client verification. This section
does not claim iOS/Android model-center parity or completion of legacy Web HTML
migration.
