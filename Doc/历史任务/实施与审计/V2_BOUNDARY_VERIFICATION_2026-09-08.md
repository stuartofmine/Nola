> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

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
