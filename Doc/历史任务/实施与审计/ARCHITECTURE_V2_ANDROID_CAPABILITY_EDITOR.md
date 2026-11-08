> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Android capability editor boundary

Status: partially adopted; remaining verification and ownership gaps are listed below.

## Original audit findings (before adoption)

- `ChatViewModel.mutateUploadedSkills` identifies an item by its displayed list index, publishes the edited list before persistence, reloads a profile, and sends the whole list through `ChatRepository.saveMemoryProfile`.
- `UploadedContent` and `decodeAndroidUserSkills` provide display content, not a canonical editor revision or complete tool references. Reading a fresh revision immediately before saving would authorize an old draft against a newer record.
- `updateMcpServers` has a similar optimistic whole-list write, even though `McpServerConfig` now carries its canonical revision and original configuration.
- Chat document import and deletion still call the same profile replacement API. That API also replaces memories, upserts Skills/MCP servers, and deletes omitted identities through separate commands.
- `IntelligenceViewModel` memory edits now use `saveMemoryText`; its capability editors already use checked individual mutations. Do not reintroduce whole-profile writes to share code with the chat editor.

## Required ownership

1. Application owns canonical resource identity, revision, complete configuration, and commit outcome.
2. The client stores a draft together with the snapshot opened by the user. List position is presentation only, never a persistence identity.
3. Display projections must not be serialized as complete canonical Skill records. In particular, filtering MCP/platform tool references for display must not erase those references on save.
4. A save submits only the resource and fields being edited, with the original revision. Do not query the latest revision to make an old draft pass.
5. A failed or uncertain commit preserves the draft and the baseline. Only a confirmed receipt updates committed UI state. Conflict recovery is explicit reload/reconciliation, not automatic overwrite.
6. A collection import/reconciliation, if retained, uses one Application transaction with explicit changes and per-resource preconditions. It must not infer deletion from an incomplete display list.
7. Memory, documents, Skills, and MCP configuration have separate commands. A document operation cannot write memory or change capability availability.

## Implementation order

- Replace chat Skill index-based persistence with a stable-ID editor snapshot and explicit save/cancel. Use the existing checked Skill editor contract; preserve raw instructions including whitespace.
- Route create, toggle, and delete to their typed commands. Toggle/delete must carry the displayed/opened revision. Do not attach a fresh revision silently after a conflicting update.
- Route MCP edits through their complete canonical configuration snapshots. Use Core `Mcp.ApplyChanges` only when the UI intentionally submits several changes together.
- Introduce a versioned document editor contract before removing its profile replacement callers; do not silently stop persisting documents.
- Remove `ChatRepository.saveMemoryProfile` and `MemoryProfileDataStore.saveProfile` after all callers and fixtures adopt their resource-specific commands. No compatibility wrapper that loops over resources.

## Verification gates

- Open a Skill draft, reorder the catalog, then save: only the original ID changes.
- Open a draft, edit the same Skill from another client, then save/toggle/delete: reject the stale revision and preserve the draft and the newer record.
- Edit instructions containing leading/trailing whitespace and non-builtin tool references: preserve both exactly across reopen.
- Edit one Skill while another client adds a Skill or MCP server: neither new resource disappears; memory and documents remain byte/revision unchanged.
- Inject a commit failure: committed UI stays unchanged and retry does not duplicate creation.
- Submit a multi-resource capability batch whose final precondition fails: earlier changes and sync records roll back; cold-start replay returns the same receipt.
- Save/import/delete a document: Skill, MCP, and memory revisions remain unchanged.
- Exercise the production chat editor with Compose instrumentation, not only direct JNI calls. Observe intermediate saving, failure, conflict, and reopened states.

These gates are not all complete. JNI persistence tests are not substitutes for full chat-editor interaction tests or screenshot inspection.

## Adoption progress

- Application now provides a single-query `Skill.ListEditors` projection. Android retains complete canonical records and positive revisions as transient editor metadata.
- Chat Skill toggle/delete use the clicked record snapshot. Deletion confirmation retains that record instead of a list index. Commit failures do not optimistically replace the list.
- Chat Skill creation/content editing now use an explicit draft dialog and checked individual commands. The old whole-list Skill mutation helper is removed. Four Compose tests passed, covering explicit save, whitespace, stale-editor draft retention/cancellation, and existing conversation-switch behavior. Those dialog tests connect to the real JNI store but do not yet cover every full ChatViewModel dependency or list-reorder scenario.
- The chat MCP collection callback had no invocation in the page. It and the unused `updateMcpServers` whole-profile writer are removed rather than adding a second MCP editor. The intelligence screen remains the MCP editing entry point.
- Document add/delete now use a stable-ID collection snapshot and `PutProductSettingChecked` revision precondition. The whole-profile writers in `ChatRepository` and `MemoryProfileDataStore` have been removed, including their fixtures. Document JNI coverage passed for stale writes, duplicate identities, deletion isolation, and cold reopen.
- Unused unchecked Android Skill/MCP write wrappers are removed. New Skills without an explicit identity use a UUID, not a name-derived identity. Eight emulator tests passed for Skill/MCP editing, stale snapshots, same-name identities, and cold reopen.

## Remaining V2 work

- The document setting is an interim collection-CAS boundary, not the final typed document/Artifact domain. iOS and Android still encode this setting differently; no cross-platform compatibility fallback is intended.
- Some capability callers still load a new revision immediately before an operation instead of carrying the displayed revision. Audit all entry points, not only the chat dialog.
- Complete the full ChatViewModel interaction gates, including list reorder and uncertain commit/retry. Current dialog instrumentation uses the real JNI store but bypasses parts of the ViewModel.
- Replace full, independently queried profile collections with bounded revisioned projections. Passing the editor tests does not establish coherent cross-resource snapshots or UI performance.
- Memory text editing must preserve resources and namespaces outside its scope. Core now retains identical canonical memory records without rewriting lifecycle metadata; iOS namespace preservation and receipt tests passed. Android's lossy display-key deduplication is being covered separately with real JNI tests.
