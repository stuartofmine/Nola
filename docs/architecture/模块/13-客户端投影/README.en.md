# MOD-13 · Clients, conversation projections and A2UI

[简体中文](README.md) · English · [Architecture](../../README.en.md)

The client owns focus, drafts, scroll position and disclosure state. Application owns committed messages and run status. A transient overlay is bound to its run and attempt. The terminal seal finishes streaming while preserving part identity and the reader’s scroll anchor.

## Runtime behavior

Clients consume committed increments keyed by stable message and part IDs. Editing and resending rewinds the affected suffix in the same conversation. A2UI components submit input against a surface revision. Rendering a form and performing an external action requested through it have separate boundaries.

## Failure and verification

Test edits with stale revisions, switching conversations during streaming, submitting an obsolete form and reconnecting at a journal cursor. Verify the rendered screen on the target platform. A successful reducer or prototype test cannot establish focus, hit targets or keyboard behavior.

## Implementation

- [nola-mobile-bindings/src/lib.rs](../../../../core/nola-mobile-bindings/src/lib.rs)
- [nola-mobile-bindings/src/projection.rs](../../../../core/nola-mobile-bindings/src/projection.rs)
- [apple/Protocol/RemoteSubmissionIdentity.swift](../../../../shared/apple/Protocol/RemoteSubmissionIdentity.swift)
- [nola-application/src/edited_resend.rs](../../../../core/nola-application/src/edited_resend.rs)

## Detailed references

[A2UI Runtime Contract](A2UI_RUNTIME_CONTRACT.en.md) · [Android Web CLI UI 20261019](ANDROID_WEB_CLI_UI_20261019.md) · [Canonical Streaming Text Contract](CANONICAL_STREAMING_TEXT_CONTRACT.md) · [Client Boundaries](CLIENT_BOUNDARIES.md) · [Client Interaction And Configuration](CLIENT_INTERACTION_AND_CONFIGURATION.md) · [Client Projection Boundary](CLIENT_PROJECTION_BOUNDARY.md) · [Mobile Shared Core](MOBILE_SHARED_CORE.md) · [Mobile-Agent-Architecture](mobile-agent-architecture.md)
