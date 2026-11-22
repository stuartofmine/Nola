# Product and architecture requirements

[简体中文](REQUIREMENTS.md) · English

Nola is a local-first personal assistant for macOS, iOS/iPadOS, Android, Windows, Web and CLI. A user can work on one device or delegate execution to an authenticated Host. Model credentials, Host pairing and permission to perform a particular operation each have a defined owner.

| Scenario | Acceptance boundary |
| --- | --- |
| First configuration and inference | Start with an empty Store, apply a valid model catalog and send the first message. A rejected declaration preserves the previous usable default. Confirm the actual provider route and effort on a real request. |
| Conversation and experts | Preserve message, attachment and draft ownership. Edit and resend in the same conversation, rewind affected history, and retrieve an expert’s child conversation under its parent. |
| Tools, UI and permission | The model chooses from schema. Core evaluates the concrete request. Render committed parts, evidence and forms. Cover denial and recovery, stale forms and switching conversations during a pending approval. |
| Data sources and automation | Preserve each source’s read recipe, provenance and validation. Create, inspect, run or remove an automation. On restart verify the occurrence, lease, output and notification against the original execution. |
| Remote long tasks | Authenticate the source Host, retain request identity on disconnect and resume observations by cursor. Verify continuing and cancelling work on the executing Host. |
| Software update | Verify signed release metadata, target artifact and data compatibility before platform installation. Preserve original data on a failed compatibility check. |

## Conversation and channel acceptance

Exercise an installed execution adapter and a real model across multiple turns. Persist the conversation’s selected executor and external session identity. Authenticate incoming channel messages, deduplicate delivery and bind an outbound reply to its approved destination. Record which adapter, account and device the test exercised.

Title generation uses the first visible nonempty user input. An explicit rename survives subsequent messages. Keep model generation settings with the selected profile and sparse session overrides. Retain original parts when sealing streams. Present a tool’s human-readable result with scoped access to technical output and artifacts. Verify clicks, rendered messages, replay and the executing Host’s observations.

Treat compaction, an outstanding question, a denied permission and an inference failure as distinct user-visible states. Scope drafts and input callbacks to their conversation. Context recall, provider usage and published sources each retain their provenance. A remote client uses the source Host’s configured capabilities.

## Ownership and protocol requirements

| ID | Requirement |
| --- | --- |
| `REQ-ARCH-001` | Assign one owner to each product fact. Domain defines identities, Core decides, Application coordinates writes, Store persists, Hosts perform I/O and clients own transient presentation. |
| `REQ-ARCH-002` | Route product writes through Application on all six client surfaces. Build messages from committed projections. |
| `REQ-ARCH-003` | Map storage records to product types inside the Application boundary. Keep SQL rows and checkpoints private. |
| `REQ-ARCH-004` | Version process-boundary messages. Require stable request and event identities, monotonic event order and idempotent replay. |
| `REQ-ARCH-005` | Treat submitted identifiers as opaque and case-sensitive from transport through rendering. |
| `REQ-ARCH-006` | Give sessions, runs, effects, permission requests, automation occurrences and background tasks their own typed lifecycles. |
| `REQ-ARCH-007` | Bind AutomationRun to its definition. Application validates enqueue, claim, heartbeat, retry and completion. |
| `REQ-ARCH-008` | Non-executing clients request effects through the Host protocol. Only the designated Application can commit new business facts. |
| `REQ-ARCH-009` | Keep a canonical database writer alive independent of individual windows or short-lived clients. |
| `REQ-ARCH-010` | Store the full Message/Part transcript. Compaction replaces the working set and retains the accepted summary until a later replacement. |
| `REQ-ARCH-011` | Restore through Application. A client-side checkpoint file cannot reconstruct the canonical conversation. |
| `REQ-ARCH-012` | Verify new data sources with an actual read and store the read recipe, content format and validation state. Persist selected snapshots explicitly. |
| `REQ-ARCH-013` | Discover Rust build outputs with cargo metadata, including under a custom target directory. |
| `REQ-ARCH-014` | Support the reissued 1.0.0 baseline, negotiate incompatible changes, test released fixtures and reject unknown commands before writes. |
| `REQ-ARCH-015` | Exercise serialization across Rust and native bindings, unknown fields, replay, conflicts, leases and cold recovery. Use real multi-turn inference for model-selected routing. |
| `REQ-ARCH-016` | Verify visible behavior with actual runtime evidence and inspection of screenshots or recordings on the target surface. |
| `REQ-ARCH-017` | Rebuild canonical parts after branch, rebase or revert from ledger entries and ownership. Test identity, order and cold recovery. |
| `REQ-ARCH-018` | Display conversation context usage from the Core/Application snapshot, including occupied tokens and total window. |
| `REQ-ARCH-019` | Read the end-to-end transcript back from canonical storage. Bound each device, Host and network probe and the overall test duration. |
| `REQ-ARCH-020` | Commit a combined conversation metadata change, its settings and its sync intent in one transaction. A failed update leaves no intermediate revision. |

## Run configuration and streaming requirements

| ID | Requirement |
| --- | --- |
| `REQ-RUN-001` | Each new assistant or maintenance run supplies an immutable invocation snapshot. |
| `REQ-RUN-002` | Keep the redacted snapshot with the locally admitted run. A remote mirror carries the source projection. Resolve credentials privately on the executing Host. |
| `REQ-RUN-003` | Bind Host events to the run’s configuration digest and reject a mismatch before consuming an effect or advancing the cursor. |
| `REQ-RUN-004` | New runs resolve new configuration. An active run recovers its committed snapshot. |
| `REQ-RUN-005` | Reuse the active run actor on the healthy path. Provider deltas must not each require opening the database or restoring a full checkpoint. |
| `REQ-RUN-006` | Batch streamed deltas by semantic boundary, tool or status change, size, bounded latency (50–100 ms design target), or terminal event. |
| `REQ-RUN-007` | Commit parts, their cursor, checkpoint, effect acknowledgement and successor outbox atomically per batch. |
| `REQ-RUN-008` | Reference large data by type, scope, size, origin, time and digest. Read bounded ranges rather than embedding the payload in a checkpoint. |
| `REQ-RUN-009` | Platforms own persisted preferences and supply the typed snapshot. Core uses those values for the admitted call. |

## Host artifact acceptance

Require a canonical URI and display name on every artifact. Percent-encode local file URIs and verify Unicode round trips through native emission, storage, projection and opening the artifact. Reject an invalid resource atomically and preserve the original effect for a corrected response. Exercise direct, delegated, replay and restore paths. Evidence for a visible artifact includes the actual event, committed record, real Host/model run and inspected screenshot.

[System design](SOFTWARE_DESIGN.en.md) · [Deployment](CURRENT_IMPLEMENTATION_MAP.en.md) · [Compatibility](VERSION_COMPATIBILITY.en.md)
