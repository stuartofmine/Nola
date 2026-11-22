# MOD-01 · Domain and protocols

[简体中文](README.md) · English · [Architecture](../../README.en.md)

Domain defines identities and value invariants. Protocol defines Service envelopes, receipts and projections. Host Protocol carries execution requests and observations. Each consumer decodes its own envelope before admission. The four stored receipt outcomes are accepted, committed, rejected and conflict.

## Runtime behavior

Look up a Service command by its request_id. Attach a tool result to its original effect, attempt and execution generation. A single user action can produce both identities. The Service protocol, C ABI and database epoch each have their own reader and version check.

## Failure and verification

A timeout leaves the command outcome unresolved. Look up the original receipt. Test round trips and rejection of unsupported versions independently for Service, native bindings and persistence.

## Implementation

- [nola-domain/src/lib.rs](../../../../core/nola-domain/src/lib.rs)
- [nola-protocol/src/lib.rs](../../../../core/nola-protocol/src/lib.rs)
- [nola-host-protocol/src/lib.rs](../../../../core/nola-host-protocol/src/lib.rs)

## Detailed references

[API](API.md) · [Current Runtime Interfaces](CURRENT_RUNTIME_INTERFACES.md) · [Interface Catalog](INTERFACE_CATALOG.md) · [Persistent Agents](PERSISTENT_AGENTS.md)
