> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Committed assistant journal Store contract

Source checked: 2026-11-08. The Store leaf, Application producer and source/replica contracts have current implementations. The old “Store implemented, production integration pending” opening and its schema-33 snapshot no longer define current status.

## Boundary

`AssistantJournalRepository` is a Store Port used by Application. It owns persistence/read constraints, not a second conversation reducer or provider parser. Application supplies exact source identity, committed ownership and the operation set inside the existing transaction. Thin clients consume Application snapshots/cursors and cannot create arbitrary journal publications.

## Current persistence and reads

- Current schema is composed by [schema_store.rs](../../../../core/nola-store/src/schema_store.rs) from per-feature DDL, including journal, publication, range, snapshot-lifecycle and remote journal structures. Do not initialize from a copied historical schema or an ALTER chain.
- Journal sequence/cursor, immutable read snapshot, source publication membership and canonical text/stream body references have different purposes. Readers must use a bounded upper watermark and the exact source/owner; a text hash alone is not authority.
- Part metadata changes do not require replacing cumulative body text. Content reads use canonical field readers and byte/range indexes, preserving complete Unicode boundaries and reference checks.
- Publication acceptance and its business transition share the owning transaction. CAS/deduplication do not authorize redispatching an external effect, and receiver staging is not an execution checkpoint.
- Exact committed records may be acknowledged idempotently after reception. Missing or inconsistent content is a recoverable reception error, not permission to fabricate a head, message or terminal state.

## Source of truth and recovery

Store schema identity, Core checkpoint format, Application configuration revision, source sequence and receiver cursor are independent. Use the constants and types in their actual source owners rather than the obsolete numeric values in implementation chronology.

Application resolves lifecycle and recovery; Store checks ownership, completeness, immutability and transactional integrity. A query is not a mutation receipt; a successful transport read is not a Run terminal seal. Backups/current-version archive restore keep source provenance and do not create local execution authority.

## Required checks

Exercise source publication plus ordinary/metadata/terminal changes, bounded snapshots/ranges, duplicate exact records, owner/revision/manifest conflicts, rollback, cold reopen and complete content verification. Measure total read/write/serialization volume separately from elapsed time; metadata bounds do not prove end-to-end linear complexity.

Current integration is described in [Application producer](COMMITTED_ASSISTANT_JOURNAL_APPLICATION_ADAPTER.md), [journal design](COMMITTED_ASSISTANT_JOURNAL_DESIGN.md) and [original readers](ORIGINAL_TEXT_READER_CONTRACT.md). Static source alignment is not a new full acceptance run.

Historical DDL slices, intermediate migration plans and their original test evidence are preserved in [the Store ledger](../../../历史任务/实施与审计/ASSISTANT_JOURNAL_STORE_EVIDENCE_20261108.md); they do not require preserving an old format or implementation.
