> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Nola local SQLite Store

Source checked: 2026-11-08. The canonical Store is a product-state schema and ownership boundary, not one universal file shared by every installed app or account. Native wrappers, schedulers and Hosts submit Application commands instead of persisting a parallel product model.

## Database identity and files

| Client/resource | Current source of identity | Meaning |
| --- | --- | --- |
| CLI / Rust Web | `cli/src/platform_paths.rs`, explicit `NOLA_DB_PATH` or resolved platform default | Same database identity is required to share conversations/models; CLI default uses the Store epoch in its filename |
| macOS | `MacAppEnvironment.databaseURL`, supplied to the Service and child environment | The current Mac filename is not assumed to equal a separately launched CLI default |
| iOS | `IOSApplicationRuntime` resolves its sandbox `nola-product.db` | Embedded Application owns the local writer |
| Android | `AndroidApplicationRuntime` resolves no-backup `nola-product.db` | Device sandbox and pending-command journals are not a remote Host database |
| Upload staging | Store content-upload adapters and a bounded auxiliary SQLite resource | Staging is not the canonical conversation/Run authority |
| Code-memory index | Its explicit index/database resource | Derived search index, not a second conversation writer |
| Worker / D1 | `backend/worker` current schema | Separate auth/transport/replica deployment; not local Agent state |

Do not search, copy, merge or recreate other database files to make a selected path appear to work. Model weights, bookmarks, credentials, logs and browser resources have separate owners; their presence does not make them business truth.

## Current schema gate

`core/nola-store/src/schema_store.rs` validates all of application_id, user_version and `store_metadata.schema_epoch`. Existing objects must match this build's exact schema identity. Only an empty, unclaimed database is initialized.

The schema is assembled from `schema.sql` plus feature DDL files selected by `create_current_schema`; `schema.sql` alone is not a complete initialization specification. Schema version, epoch, checkpoint format, Service version and C ABI version are independent constants in their owning sources. No historical ALTER chain or guessed defaults is permitted.

The default bootstrap policy is `RequireCurrent`; incompatibility is an error, not permission to delete or rebuild user data. Explicit user-requested recreate/archive operations remain separate. Historical provider decoding, parameter conversion, recovery markers and normal-startup archive scanning have been removed from bootstrap, Service and Mac supervisor. An explicitly archived incompatible Store remains an archive; its provider configuration is not seeded into the fresh Store.

Explicit restore/import of a supported current backup is a separate Application use case with validation, staging and transactional installation. It does not justify historical-format compatibility or automatic startup recovery from a different schema.

## Content and transactions

- Conversations, messages, Parts, Runs, permissions, model configuration and execution evidence have canonical Store records and exact owners.
- Canonical text objects/chunks, stream heads/manifests, body references and assistant journal publications are SQLite structures with typed readers. Not every long output is a file under `~/.nola/blobs`.
- External resource references and canonical body references are different contracts. Hash integrity is not read authorization; original owner, source/revision and disclosure checks still apply.
- Application accepts a candidate transition only after Store commits the related facts, receipts, replay/checkpoint state and publications. Queries and unrelated configuration changes do not all rewrite a full checkpoint.
- Raw transcript/permission/Task facts cannot be reconstructed from a lossy diagnostic Trace. Derived indexes may be rebuilt only under their explicit maintenance contract.
- Source journal publication and authenticated replica reception preserve source ownership. Receiving data is not executing the source Run.

## Open and recovery

Resolve the intended database identity, acquire the Service/embedded writer ownership, validate the current schema, then expose Application queries/commands. Thin clients do not open SQLite to repair Service failure. A failed open reports its cause; it cannot silently return an empty replacement store.

Core revision, Store checkpoint CAS, configuration revision, source cursor and worker generation are distinct. Interrupted commands reconcile the original receipt; interrupted physical effects reconcile their original execution evidence before any redispatch. Memory caches and process handles are reconstructible helpers, not durable authority.

Implementation entries: [schema gate](../../../../core/nola-store/src/schema_store.rs), [connection](../../../../core/nola-store/src/connection.rs), [bootstrap](../../../../core/nola-application-bootstrap/src/lib.rs), [canonical text](../../../../core/nola-store/src/canonical_text_store.rs), [archive admission](../../../../core/nola-application/src/archive_restore.rs). This document records static source alignment, not a new recovery or performance acceptance run.
