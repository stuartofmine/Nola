# Release and data compatibility

[简体中文](VERSION_COMPATIBILITY.md) · English

The reissued Nola 1.0.0 is the first supported release. The baseline for Nola-owned protocols, native ABI, database format and Store epoch is **1**. Earlier development formats have no supported conversion path. An unsupported file must be rejected while preserving its contents. External protocols such as MCP and provider APIs follow their own versions.

Application SemVer and build identify a distribution artifact. Service version negotiates a command contract. A profile revision detects an edit. Store epoch identifies a storage contract. Each check uses its own value. Stable file names and record identifiers survive an application patch.

| Change after 1.0.0 | Required evidence |
| --- | --- |
| Add an optional capability | Existing callers retain field meaning, identity, defaults and permission scope. Use a negotiated extension where the old decoder rejects unknown fields. |
| Change a protocol or C ABI incompatibly | Specify supported versions and negotiation, publish upgrade guidance, and exercise old and new callers. Ship matching headers, native libraries and JNI bindings together. |
| Change published data | Preserve the original, create a recoverable backup, validate integrity and run a rollback-capable migration. Test repeated application, failed migration, unknown future formats and downgrade guidance on released fixtures. |
| Distribute an update | Declare the actual data contract in the signed feed and block an incompatible install until the corresponding migration is implemented and verified. |

The current Store checks the released format on open. Future migrations need their own implementation and fixtures. [Store gate](../../../core/nola-store/src/schema_store.rs) · [Service protocol](../../../core/nola-protocol/src/lib.rs) · [Native ABI](../../../core/nola-mobile-bindings/src/lib.rs) · [Update delivery](SOFTWARE_UPDATE_ARCHITECTURE.en.md)
