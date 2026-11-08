> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Canonical text publication integration

Status: implementation in progress. This extends the acceptance work under
`ARCHITECTURE_V2.md`; it does not replace or narrow that architecture.

## Implemented storage primitives

- `nola_domain::CanonicalTextReference` is an immutable, validated descriptor.
- `nola_store::canonical_text_store::CanonicalTextStore` commits complete objects
  atomically, verifies exact-byte retries, and returns bounded UTF-8 ranges.
- `ContentUploadStore::copy_sealed_to_canonical` consumes a consistent source
  snapshot one chunk at a time and retains the upload for retry.
- Query-only opening rejects missing or unrelated content stores. It must not
  create storage as a side effect of a missing-content lookup.
- Canonical text quota admission counts total declared UTF-8 bytes of committed
  objects inside the same immediate transaction as insertion. Distinct writer
  handles cannot each reuse the full quota; replay does not reserve space again,
  and incomplete chunk streams roll their reservation back. This is a logical
  content quota, not a bound on SQLite file, index, or WAL bytes. Metadata scanning
  for admission still needs scaling work before large object-count deployment.

These primitives do not yet publish a reference in a conversation and are not
the complete user-visible import feature.

## Required publication order

1. Application validates the import source, conversation ownership, idle Run
   state, expected message count, and planned Message/Part identities.
2. Application validates the sealed upload owner, declared length, and chunk
   count. It assigns or resolves an immutable canonical content identity.
3. Application copies bounded chunks into canonical content storage and commits
   that complete object. A retry must match both descriptor and bytes.
4. One main Store transaction commits Message/Part references, conversation
   revision, projection invalidation, list projection change, and command receipt.
5. Only after that transaction commits may clients observe the new messages.
   Staging reclamation must not run before publication is durably acknowledged.

A crash between steps 3 and 4 may leave an unreferenced immutable object. It must
not produce a partially visible message. Garbage collection needs a separately
proven reference/pinning policy; deleting content merely because it is old is
not safe. Replaying step 4 must not duplicate messages or overwrite another Part.

## Integration points confirmed in current code

- Import, edit, and branch message inputs now use Protocol-owned
  `TranscriptMessageInput` / `TranscriptPartInput`. CLI/Web import these contracts
  directly from `nola_protocol`, not Application. Only Application validates
  finalized use-case rules and converts them into Store inputs, moving
  body strings instead of cloning them. Nested unknown fields are rejected.
  Canonical chunk references are not yet part of this contract; introducing them
  still requires the coordinated reader/publication rollout below.
- `ReadConversationTextRange` currently dispatches directly to the main Store.
  Runtime must resolve a persisted Part reference, then choose inline or chunked
  reading. Do not authorize reads from a client-supplied content ID alone.
- Full, paged, and windowed conversation projections all need explicit storage
  semantics. A chunked Part must not become empty text or silently become a
  complete-looking preview when converted to a domain projection.
- Deferred previews need total original byte lengths from the canonical
  reference, not `length(content_text)` of a preview.
- Local/remote projection materialization, original-history search, model
  context preparation, and backup export need the same canonical resolution
  rules. No client may fill missing original content from streamed deltas.

## Activation gate

Do not expose the new import command while any canonical reader can silently
drop or reinterpret a reference. Roll out the required DTO/protocol changes
together, with a protocol bump rather than legacy fallback behavior. Do not
delete or recreate user databases to make tests pass.

Required integration tests include: a real Service process importing a single
ten-million-character message; restart and receipt replay; concurrent stale
import rejection; failure after content copy but before main commit; bounded
window/range/search/context reads; correct Mac preview and original reader;
and lossless backup/export. Existing storage unit tests alone do not satisfy
these gates.
