> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# V2: chunked transcript publication

Status: incomplete. This document specifies the production gap exposed by the
native Mac acceptance run; it does not claim that large-message import works.

## Evidence

The Mac test build succeeded. The selected native suite ran 17 tests: 16 passed,
one failed, none skipped. `testNativeRecallOfTenMillionCharactersReturnsBoundedTailEvidence`
failed during `restoreBackupSnapshot`, before reaching recall. The client encoded
a whole message and rejected it at the import frame budget. Replacing this test
with a smaller message, splitting one logical message into many messages, raising
the frame budget, or bypassing the client import path would hide the failure.

## Existing boundaries

- `ImportConversationTranscript` accepts inline `TranscriptPartInput` bodies.
- `ContentUploadRepository` stages owner-scoped chunks, tracks offsets, seals,
  and resumes without loading the complete upload.
- `CanonicalTextReference` identifies immutable content and its canonical owner.
- `ContentUploadStore::sealed_content_reference` derives byte length and chunk
  count from one validated sealed snapshot. An upload ID alone grants no access.
- `copy_sealed_to_canonical` copies immutable chunks with a destination transaction.
  Copying is not message publication and must not delete staging content.
- Canonical message reads, search and replica publication still primarily use
  inline text. The independent content store is not yet the main Part read path.

## Required publication contract

Use a bounded import manifest containing finalized message metadata and explicit
Part-to-upload bindings. Bind every upload to conversation, import and Part;
bind the resulting immutable content reference to the declared message as well.
Reject duplicate bindings, undeclared Parts, simultaneous inline and uploaded
body sources, wrong owners, cancelled/unsealed uploads, inconsistent byte counts,
and mutable or changed replay input. IDs must remain stable across retries.
Uploaded text Parts must leave both `content_text` and `content_json` absent of
inline bodies: `content_text` is empty and `content_json` is `None`. Even a small
JSON body is a conflicting second source, not harmless metadata. All message and
Part identities, including unbound inline Parts, use the bounded manifest ID rules.

Application must validate the complete finalized manifest and its conversation
revision/count before publication. A live Run must continue to prevent history
editing. The client must never authorize an import by supplying a filesystem path.

The existing inline append repository now rechecks active Runs and known Run
ownership inside the write transaction, in addition to Application preflight.
Tests commit a Run through a second fixture connection after preflight, then
verify refusal, unchanged history/revision and no new sync outbox record after
reopening. Chunked publication must use the same commit-time preconditions; a
successful check before content preparation is not a reservation.
The append integration suite also injects an SQLite trigger failure at sync
outbox insertion, after message/Part insertion and revision advancement. It
checks rollback of message/Part counts, revision, timestamp and outbox count,
reopens the database, and retries identical input after removing the fixture
failure. This covers the existing inline commit unit, not chunked publication.

1. Validate and prepare each sealed source using bounded metadata and chunk reads.
2. Copy immutable content into canonical content storage. Verify identical bytes
   and metadata on replay. Failed copies must not leave a partial visible object.
3. In one canonical write unit, recheck conversation and message preconditions,
   insert messages and Part references, advance revisions, publish bounded
   projection invalidations and sync facts, and commit the command receipt.
4. Only after that commit may the import return success. A restart between steps
   2 and 3 can leave an unreferenced content object, never a visible partial Part.
5. Reclamation requires proven reference/lifecycle accounting. Do not delete an
   upload or canonical object merely because a client disconnected or copying
   succeeded. Do not automatically recreate a user's database for schema changes.

## Readers must be ready before enabling publication

All canonical readers must understand the Part reference: bounded UI windows,
original-text ranges, literal history search, context construction, export,
branching and replica restore. A raw reference JSON object is not a rendered
message, and a preview must never replace the full original in recall or export.
Token usage must not silently count only the preview.

### Concrete reader integration boundary

The current code has three distinct paths that must resolve the same published
Part body before the import command is enabled:

- `nola-application/src/conversation_recall.rs` consumes `TranscriptReadRepository`
  and validates range ownership, offsets, UTF-8 length and EOF in the response.
  Preserve this command and port rather than adding a second upload-only recall API.
- `nola-store/src/bounded_message.rs::load_parts` currently reads bounded prefixes
  directly from `message_parts.content_text`; its JSON hydration decision also
  depends on the inline column length. A referenced body needs the same bounded
  prefix semantics without hydrating an entire canonical object.
- The same `load_parts` pass now derives deferred metadata from the lengths
  already selected for preview loading; the separate `deferred_text_parts`
  query pass has been removed. Referenced bodies still need published identity
  and original length rather than the empty inline column's length.

`conversation_read_repository.rs` carries the preview loader's Parts and deferred
metadata together to the main Store. These operations must observe one coherent Part
identity; independently selecting an upload by client-provided owner strings is
not a valid substitute for resolving a committed reference. The existing
`transcript_range.rs` and `transcript_search.rs` remain the canonical range/search
entry points and must cover both storage representations.

Deferred text metadata is now owned by Store Port rather than the SQLite crate.
Construction in the preview adapter and wire deserialization both reject empty
identities, zero original length, and previews equal to or longer than the body;
wire decoding also rejects unknown fields. This validates metadata shape only:
matching the metadata against the actual Part and immutable body remains a reader
responsibility, and chunked publication is still not enabled.

Preparation into `CanonicalTextStore` alone changes none of these production
readers. Do not introduce a visible reference field or enable a publication RPC
until preview, deferred metadata, range reads, search and replica publication have
matching support. In particular, returning an empty inline Part and relying on
the UI to discover an upload would violate the canonical message contract.

Range reads must reject a missing tail chunk rather than return a successful
partial prefix. A short range is valid only at actual EOF or when the next UTF-8
scalar does not fit the remaining byte budget. Chunked literal search now uses
bounded overlapping windows and inherits this range integrity check, but it is
not yet wired into canonical Part resolution or the Mac import path.

Replica transfer needs bounded content chunks and acknowledgement before a
remote reference becomes visible. Sending only the manifest produces dangling
Parts; embedding the entire body in sync metadata restores the original problem.

## Mac restore lifecycle

Both native backup entry points preflight all message frames before clearing
existing history. The ordinary snapshot path also checks metadata encoding.
The oversized-message regression keeps a pre-existing conversation and places
a valid backup conversation before the oversized one, asserting that neither
partial replacement nor loss of the existing original occurs. This only guards
deterministic preflight failures: restore is not yet an atomic multi-conversation
Application transaction, and failures after clearing remain a separate gap.

Plan bounded metadata, create/resolve the conversation, upload text in UTF-8-safe
chunks, seal, then publish the manifest. Resume by durable import/message IDs and
server offsets. The current `result.created` early return is not adequate for
resuming an interrupted multi-batch restore: an existing conversation is neither
proof of completion nor permission to overwrite its history. Resume must verify
the previously committed import prefix and reject unrelated existing content.

## Acceptance gates

- Keep the failing native ten-million-character test and its original one-message
  identity; run it through actual Mac IPC with the production restore path.
- Measure client and Service peak RSS, frame sizes, elapsed time and rendered UI.
- Exercise Unicode and JSON escaping, multiple large Parts, metadata-only Parts,
  and mixed small/large messages without allocating a full encoded backup batch.
- Restart after chunk append, seal, content copy and publication. Verify idempotent
  receipts, exact original bytes, no duplicate messages and no partial visibility.
- Reject altered replay bytes, foreign conversation/import/Part ownership and
  stale conversation preconditions; preserve all previously committed messages.
- Verify original recall, bounded viewport display, context usage, export and
  cross-database sync after restarting both endpoints.

Until these gates pass, native large-message import and the broader V2 goal
remain incomplete. Storage primitives and isolated tests are not substitutes
for successful end-to-end publication and rendering.
