# 架构历史实施记录

2026-11-08 从现行架构中分离。下文保留原日期、失败/通过与暂停范围，不构成当前实施状态、操作限制或兼容要求；相对链接按归档位置调整。

> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Committed assistant journal Store implementation plan

Status: implementation plan, not a claim that the journal is wired into production.
Read with `COMMITTED_ASSISTANT_JOURNAL_DESIGN.md` and
`COMMITTED_ASSISTANT_JOURNAL_APPLICATION_ADAPTER.md`. Source persistence and bounded
reads exist. The 2026-10-02 complete publication set implementation passed shared
lib acceptance and full Application/Store/Domain tests. Production producer integration and
remote migration remain separate.

### Source leaf now implemented

The StorePort `AssistantJournalRepository` provides local epoch, typed persistence,
append manifests, affected-head reads, immutable snapshot handles and fixed upper
watermark journal reads. The earlier schema 30 introduced the byte-range index;
current schema 33 adds complete publication receipts and ordered frame membership.
Neither this leaf nor the new set API is an automatic Application projection
producer or a remote protocol migration.

`AssistantSourceCommitStamp` distinguishes admission from transition. Private
per-write-unit evidence is recorded only after a successful `begin_turn`,
`begin_internal_turn` or `commit_transition`. First publication checks that evidence
and current persisted checkpoint/Run/message ownership. Rollback clears evidence;
existing set replay instead verifies its complete typed manifest and persisted
records. Successor assistant messages are proven through `messages.run_id`,
conversation and assistant role,
not restricted to the Run's initial assistant slot.

The current Domain batch has no empty/no-change operation. An unchanged visible
projection must produce no batch; there is no pretend `NoChange` Store result.
An equal metadata update must be filtered by the producer before submission.

Read budgets count persisted header plus canonical batch bytes, or encoded head
payload bytes. They are not a guarantee about JSON transport envelopes: transport
must enforce its own actual encoded byte budget. Explicit snapshot release now
requires the complete owner/sequence/digest handle inside a write unit of work.
It returns `Released` or `AlreadyReleased`, retaining a minimal proof for exact
idempotent retries. All snapshot reads reject released handles; other snapshots
and shared content are unchanged. Version 28-to-29 adds only this release ledger.
Release deactivates the handle; it does not reclaim content. Automatic expiry,
proof-retention limits and global GC remain separate gates.

### Complete publication sets implemented and leaf-verified

`CommitAssistantPublicationSet { stamp, groups }` carries distinct owner groups.
Each `AssistantPublicationGroup` holds the captured starting cursor and ordered
`AssistantJournalFrame { batch, supplied_chunks }` values. One authoritative source
commit identifies one complete set, independently of its content digest. Store
resolves the revision and derives each frame operation ID; a changed digest cannot
mint a new publication slot for the same source.

The persisted receipt binds exact owner order/membership, frame counts, canonical
batch bytes, supplied chunk order/digest/length and resulting cursors. Replay must
also verify the actual retained records and header chain against the sidecar and
receipt. Incoming chunk validity alone is insufficient: supplied chunks belonging
to this set require bounded checks against the stored original bytes. This does
not require scanning every older prepared field or the cumulative assistant.
Historical read ownership may support exact replay after a trusted handoff, but
cannot authorize a first write for the earlier owner.

Per frame, metadata is limited to 128 operations and 128 KiB of actual canonical
binary encoding. Supplied content separately allows at most 128 immutable chunks
and 256 KiB of raw UTF-8; each chunk is at most 64 KiB. The combined transport JSON
has a different budget. Application now packs multiple frames rather than rejecting
a legitimate larger logical transition, retaining touched-head revisions and
immutable chunk boundaries. Borrowed canonical-owner mapping preserves admitted
and successor message identities without copying their cumulative Part bodies.

The set's savepoint covers introduced chunks, heads, versions, records, membership
and the publication receipt. Caller preparations, checkpoint, effects and command
receipt still belong to the outer command UoW. A failed final frame cannot leave
public earlier frames, but an already prepared invisible manifest is removed only
by outer rollback. Preparation/reference errors and set errors must remain fatal
to that command; notify consumers only after the outer commit succeeds.

New tests exercise isolated actual Store admission/transition stamps, ensured
successor messages, two-owner/three-frame groups, cold exact replay and missing/
reordered/changed members, tampering, source/CAS rejection, final-frame fault and
same-UoW/cold original-set retries. They passed the six-crate lib run; the actual
schema32 fixture also proves an additive upgrade without rewriting existing
epochs, schema objects or canonical data. No legacy set receipt is fabricated
from an older single-frame record. Full tests passed and are tracked separately
in the overall redesign section 8.5; no transport rollout is inferred.
These fixtures do not prove production changed-owner completeness or command
coordinator integration. The source visitor must independently prove that the
first set contains every accepted visible change; its own count/digest cannot
prove that coverage. Task/Skill/ModelAttempt adapters and IdleCheckpoint/Recovery
stamps remain unfinished. Clients must not switch to this journal while those
production paths can bypass publication.

## Current constraints

- Eligible fresh/empty canonical Text already uses `PartMutation::Stream` and
  private `CanonicalStreamHead` storage through the production submission/Host
  planners. Append writes new chunks/manifests and a bounded head CAS, not SQL
  concatenation. This private original-text path is distinct from public journal
  publication and retains review visibility in public projection.
- Legacy `PartMutation::AppendText` remains for non-promotable existing fields.
  Its helper hashes the full prefix and Store concatenates inline text. Non-Text
  Part persistence, whole checkpoints and cumulative projections retain separate
  costs; the eligible Text cutover does not make the entire execution path linear.
- `published_part_body::resolve_field` now distinguishes private live `Chunked`
  Text from completed/trusted-resumption sealed references. Exact ownership,
  visibility and backing-conflict checks remain required; live Text support is
  not permission to relax the separate sealed-body finalization policy.
- `canonical_text_objects` and `canonical_text_chunks` remain prepared sealed
  body storage for import/history. Private live Text uses separate
  `canonical_stream_heads`, `canonical_stream_manifests` and
  `canonical_stream_chunks`, not an append cast of those sealed objects.
- `part_derivation::fingerprint` scans a complete field for provenance. Keep this
  as a one-time derivation operation, never an append-root calculation.
- `remote_assistant_store` and `conversation_transition_sync` currently insert
  complete replacement Part bodies. A source journal alone does not remove these
  network and replica costs.

## Typed boundary

Use the Domain batch validator and its closed metadata types. Do not introduce a
second `serde_json::Value` operation language. Domain's implemented batch contains
version, assistant identity and operations; affected Part heads contain identity,
Part ID, generation, revision, metadata revision, order key, typed metadata and
body references. Each content reference identifies field, content generation,
UTF-8 byte count, chunk count and immutable append-chain content root.

Store provides the validator with only affected heads and content-reference
verification. It persists validated changed heads, not a cloned assistant-wide
map. Declaration/replacement references must resolve to complete, owner-bound
manifests. Append verifies the expected reference and only the new chunk edges.

StorePort responsibilities and remaining integration obligations:

1. Source write: commit the complete ordered typed set under captured owner heads
   in the existing received-command write unit of work. Store obtains the source
   revision from its actual admission/transition evidence, never from a client.
   Domain does not have an empty/no-change operation: Application must filter
   equal visible state and emit no frame, rather than assert a caller NoChange
   boolean or fabricate an empty batch.
2. Journal read: open a fixed upper watermark, then read bounded contiguous pages
   through that watermark. Return typed `SnapshotRequired` for an expired cursor;
   never silently skip a gap. Sequence and digest are not Trace cursors.
3. Snapshot read: open an immutable snapshot handle; page Part manifests and chunk
   references using that handle. Read raw chunks only with canonical owner plus
   snapshot/journal proof, never with a digest alone.

No public write helper may partially persist without its required outer write
unit of work. Test direct invocation as well as received-command dispatch.

## Schema slice

All identity columns use the full source epoch/conversation/Run/assistant tuple.
Checked conversion rejects unsigned values outside SQLite's signed integer range.

| Table | Key and role |
| --- | --- |
| Local source epoch metadata | Random durable identity, distinct from schema epoch. Never synchronized. Supported restore explicitly rotates it. Arbitrary filesystem rollback is not transparently detectable. |
| Assistant journal heads | Identity primary key; head sequence, digest, retained floor. |
| Assistant journal records | Identity + sequence primary key; operation identity uniqueness, previous/current digest, actual source commit revision, versioned typed batch encoding with strict bounds. |
| Part heads | Identity + Part ID; current typed head and generation. |
| Part versions | Identity + Part ID + journal sequence; immutable changed-head versions, including tombstones. No assistant-wide copy per append. |
| Content chunks | Digest primary key, validated byte length, raw bounded BLOB. Shared byte deduplication does not grant read authority. |
| Manifest nodes | Root primary key; previous root, complete owner/field/content generation, new chunk digest, resulting byte and chunk counts. Append creates only new nodes. |
| Generation chunk index | Owner + field + generation + chunk index; byte offset, chunk digest and resulting root. Immutable entries allow forward pagination without walking the entire root chain. |
| Snapshot handles | Snapshot ID, identity, fixed sequence/digest, manifest identity/root, lifetime. Pin reachable versions and chunks until expiry/release. |
| Publication receipts | Store-derived source key; actual source revision, whole-set digest/counts and original final owner cursors. One slot per exact source commit, not per digest. |
| Publication frame membership | Source key + ordered group/frame positions; exact owner/sequence and record digest. Replay checks the actual referenced records, not only this sidecar. |

Persist typed batch/metadata encodings with a version and decode validation; digest
bytes come from the Domain canonical encoding, not incidental JSON serialization.
Snapshot creation may perform a bounded-page scan to establish its manifest root;
it must not buffer all Parts or body bytes, nor hold a SQLite read transaction
across client requests. A pinned sequence makes version selection repeatable.
Manifest pages stop at the snapshot content reference's exact chunk count/root,
even if the same live generation subsequently appends more chunks.

The target production command must atomically commit canonical changes, prepared
content, the complete journal set, sync references and command receipt in one
outer UoW. Store's set savepoint implements journal atomicity, not that still
missing production coordinator. A post-commit wakeup is only a recoverable
watermark hint; it cannot be the journal's source of truth.

Chunk introduction also requires authorization: a new reference must carry
verified bytes supplied in this operation, or proof that the chunk is already
reachable under the authorized owner/source. A guessed digest must not allow a
remote batch to attach another assistant's private chunk to its own manifest and
then read it through newly minted owner proof. Global deduplication is only a
physical storage optimization, never permission to create a new owner's reference.

Use a real schema upgrade from version 27, preserving existing sealed references.
Do not manufacture an old fixture merely by lowering a current database's version.

## Removing cumulative canonical writes

1. Add typed source storage and immutable reader tests without advertising a new
   remote contract.
2. Private canonical Text already has a distinct live-manifest branch with owner,
   visibility, generation and root validation. Preserve it and its bounded field
   access; an empty inline slot is not absent text. Other original field types
   still need their own explicit incremental contract.
3. Integrate the Application visible-projection planner inside the existing commit
   unit of work. Raw Core Part mutations are not automatically visible journal
   operations: withheld review candidates and reasoning must remain withheld.
4. Eligible new/empty canonical Text has switched to chunk/reference updates
   without a parallel cumulative inline body. Migrate remaining eligible legacy
   paths explicitly and measure non-Text/checkpoint costs separately; a public
   journal is not authority to dual-write or substitute private original fields.
5. Move sync publication to manifest/chunk references. A one-time conversion of an
   existing inline or sealed field may scan it in bounded chunks, but cannot recur
   for every append.

Keep historical import/provenance semantics explicit. Content roots are append
chain roots, not the SHA-256 of the full concatenated field.

## Later remote slice

After source/read contracts pass, add authenticated per-Run source binding,
remote cursor/head CAS, staged snapshot references and atomic activation. Target
application validates sequence, previous digest, owner, generation and content
roots; canonical changes, local revision and sync references share one transaction.
Execution terminal receipt remains independent from content catch-up. Do not ship
a half-compatible remote fallback that reconstructs canonical text from Trace.

## Required transaction and performance fixtures

- Use isolated real databases and received-command units of work. Inject failures
  at journal, head, sync and receipt writes; canonical heads, chunks and references
  must all roll back. Direct write API calls without the required unit must reject
  before any mutation.
- Race two database connections on the same expected head. Test identical replay,
  same operation identity with different digest, wrong owner, missing chunks and
  integer overflow. Test both write-time cross-assistant forged chunk references
  and read-time cross-assistant proof rejection, including a physically deduplicated
  chunk that genuinely exists but was never authorized for the requesting owner.
- Open a snapshot, then append, replace and remove Parts while reading subsequent
  pages. Every page must remain at the original sequence; pinned content survives
  GC. Verify expired handles and retention gaps have typed outcomes.
- Exercise current-version restore epoch rotation and reopen, with sealed body
  ownership and content preserved. Reject other schemas; do not test or implement
  old-schema conversion.
- Generate 2, 8 and 32 MiB bodies with fixed 4 KiB appends. Instrument actual Store
  bytes read/written, batch serialization bytes and transmitted bytes, not only
  elapsed time. Assert growth tracks total new bytes plus operation overhead.
  Inspect SQL execution to exclude full-prefix reads and `content_text || delta`
  on the new path. Test both no subscriber and multiple subscribers.
- Include UTF-8 boundaries, highly escaped JSON, maximum chunk/page/frame limits,
  partial snapshot recovery, review visibility, terminal-before-body catch-up and
  slow-reader backpressure. JSON IPC base64 limits differ from raw HTTP chunk size.

Passing leaf storage tests is not completion: Application commit integration,
canonical reader migration, sync references and end-to-end transport each require
their own explicit gate.

## Bounded byte-range foundation

`read_assistant_snapshot_range` authenticates the exact active snapshot, Part and
content reference before seeking. Schema 29-to-30 adds an index over manifest
owner, Part, field, generation and cumulative byte endpoint. Reads locate the
containing chunk directly, verify visited chunks, preserve UTF-8 boundaries and
return at most the requested raw-byte budget (4 through 32768). Seeking near the
end does not read or hash preceding chunks. Chunk integrity verification can read
one complete 64 KiB chunk even for a small output range; this bounded cost is not
the output-byte budget or a claim that JSON envelopes have the same size.

This API reads public journal content only. Canonical original `content_text` and
`content_json`, including withheld private fields, are a different authority and
field contract. ToolInput/ToolOutput semantic references cannot be cast to an
original JSON field. The separate CanonicalFieldReader preserves exact original
owner, visibility and field semantics while sharing only authorized byte storage
machinery. The public journal range API itself does not switch canonical production
reads or remove legacy SQL concatenation. The private Text cutover below is a
different contract; neither slice completes all full-prefix persistence migration.

## CanonicalFieldReader migration slice

The function-local reader owns one SQLite read snapshot and resolves `Absent`,
`Inline`, `Sealed` or private Text `Chunked` from the exact canonical
conversation/message/Part/field. Its constructor retains visibility, sealed
finalization and trusted-resumption checks. The `Chunked` branch validates the
private canonical stream owner and backing; it is not a public journal content
reference. A text-projection constructor still refuses deferred JSON as finalized
text. This borrowed reader is not a portable StorePort/HTTP handle.

Transcript ranges, conversation hydration and referenced search use the canonical
field machinery; Text chunked range/tail reads use fixed private stream heads.
Inline tail/search SQL optimizations remain explicit and must not become duplicate
whole-field reads merely to uniformize the call graph. Export and semantic original
readers retain their original-field contract; no public journal reference is
substituted for a private field.

The implemented binding is `CanonicalStreamHead` with `CanonicalStreamOwner`,
generation, revision, byte/chunk counts, closed state and a private stream root.
Store publishes it atomically through `PartMutation::Stream` and rejects a second
inline/sealed backing for the same field. Its root is not implicitly convertible
from `AssistantDigest` or `JournalContentRef`. Current stream ownership includes
an exact Run and applies to Text; a generic chunked original-JSON or imported/user
field without a Run remains a future contract. Preserve original field grammar,
review policy and sealed-body checks when adding it.
