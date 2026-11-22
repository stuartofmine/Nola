# MOD-04 · Storage, journals and indexes

[简体中文](README.md) · English · [Architecture](../../README.en.md)

Store Port defines transaction and snapshot interfaces. The SQLite Store implements them, including stream heads, canonical text objects and scoped reads. Checkpoints carry resume state. Search indexes and projections can be rebuilt from their sources.

## Runtime behavior

Restore a conversation from canonical messages, text chunks and the journal cursor. An edited resend commits a rewind in the same conversation. Derived indexes follow that change. Opening a database checks its identity and format and preserves the original file on rejection.

## Failure and verification

Test interrupted writes, replay from a cursor, stale writes after editing and refusal of an unknown epoch. The same schema on two devices does not give them a shared database or ownership of each other’s runs.

## Implementation

- [nola-store/src/schema_store.rs](../../../../core/nola-store/src/schema_store.rs)
- [nola-store/src/canonical_text_store.rs](../../../../core/nola-store/src/canonical_text_store.rs)
- [nola-store-port/src/assistant_journal.rs](../../../../core/nola-store-port/src/assistant_journal.rs)

## Detailed references

[Canonical Backup Format](CANONICAL_BACKUP_FORMAT.md) · [Committed Assistant Journal Application Adapter](COMMITTED_ASSISTANT_JOURNAL_APPLICATION_ADAPTER.md) · [Committed Assistant Journal Design](COMMITTED_ASSISTANT_JOURNAL_DESIGN.md) · [Committed Assistant Journal Store Plan](COMMITTED_ASSISTANT_JOURNAL_STORE_PLAN.md) · [Conversation Aggregate Contract 20261015](CONVERSATION_AGGREGATE_CONTRACT_20261015.md) · [Core Checkpoint Content Contract](CORE_CHECKPOINT_CONTENT_CONTRACT.md) · [Original Text Reader Contract](ORIGINAL_TEXT_READER_CONTRACT.md) · [Store Result Binding](STORE_RESULT_BINDING.md) · [Local-SQLite-Store](local-sqlite-store.md)
