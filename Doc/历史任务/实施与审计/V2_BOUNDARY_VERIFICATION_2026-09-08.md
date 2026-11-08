> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# V2 boundary verification: 2026-09-08

Status: partial evidence, not architecture acceptance. The full scope remains
`ARCHITECTURE_V2.md`, including every platform and runtime boundary.

## Client dependency boundary

The current `core/nola-service-client/Cargo.toml` depends on `nola_protocol`,
Serde, JSON, and Unix socket support. Its source imports protocol envelopes and
does not import Application, Store, or Core implementation types. This supports
the SDK-specific dependency requirement in section 5.

It does not prove the platform requirement. Transcript message/Part DTOs have
now moved to `nola_protocol`, with CLI/Web importing them directly and no old
Application re-export. Application keeps a crate-private validation/conversion
trait. Their JSON fields are unchanged. The overall command enum and other DTOs
still come from `nola_application`; the entire command surface, not only
transcript inputs, needs this separation before the platform requirement passes.

## Canonical history progress and remaining scope

- SQLite conversation reads now have a dedicated read repository. Window
  counts, payloads, deferred metadata, and pagination flags share a read snapshot.
- Read snapshots reuse an existing transaction without committing or rolling
  back the caller's work. Deterministic WAL tests cover a concurrent writer.
- Import/edit/branch inputs validate finalized message structure. Empty imports
  are rejected instead of advancing revision without adding content.
- Import and edit reject Run IDs already owned by another local conversation.
  Unknown archive Run IDs still lack a separate provenance model. Branch Run
  identity and runtime ownership remain part of that unresolved design.
- Chunked canonical text storage and sealed-upload adoption exist, but they
  are not yet connected to canonical Message/Part publication and all readers.
  See `V2_CANONICAL_TEXT_PUBLICATION.md` for the required joint rollout.

These facts do not establish bounded Application projection assembly, context
construction, backup, all-platform UI performance, or whole-system single-writer
compliance.

## Mac build evidence

An explicit target-only build was attempted with `-target NolaMac -sdk macosx`,
without a test destination or physical-device request. Xcode still attempted
`com.apple.mobile.notification_proxy` on a passcode-protected device. The existing
guard terminated only the owned build process group and returned status 125.

Log from this attempt:
`/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-mac-target-build-5ay4oohh/build.log`

This is an interrupted build, not a compiler pass or a test pass. Explicitly
selecting a macOS target is insufficient to prevent Xcode device discovery in
this environment. The guard detects the first probe; it does not prevent it.
Do not repeatedly launch this entry point while claiming locked-device isolation.
Uncompiled Mac changes and the outstanding accessibility interaction test remain
unverified. Rust process tests do not substitute for native UI verification.
