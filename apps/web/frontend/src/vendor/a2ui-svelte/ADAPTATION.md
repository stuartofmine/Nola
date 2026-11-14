# Nola renderer adaptation

Source: ChaliceForAuri/a2ui-svelte at d0345eeb52c4c84871c82c4f9c0e43f078ac3ffc, Apache-2.0 (LICENSE retained). Original files and hashes remain in `third_party/a2ui/svelte` and `sources.lock.json`.

The production timeline mounts these components using Rust-resolved projections. Surface, Node, Slot, context and catalog types replace upstream runtime access: they forward binding edits/action references without parsing wire messages or evaluating bindings/functions. No upstream A2uiClient, reducer, transport or protocol implementation is imported. Core accepts v0.9.1 only.

Local adaptations include IME composition buffering, identity-scoped drafts, revision conflict handling, durable unknown-request recovery, and native dialog top-layer/focus handling. Resource widgets remain capability-gated; this is not full upstream catalog support. Renderer fixture tests do not establish real-model acceptance.

`third_party/a2ui/production-adaptation.json` inventories every production file with SHA-256, its upstream source where present, and the unified patch digest. `production-adaptation.patch` records differences from the pinned upstream files (new files diff against `/dev/null`). Regenerate both with `python3 scripts/update_a2ui_adaptation.py` after intentional renderer changes; `npm test` verifies the complete inventory and patch. This manifest is separate from the immutable reference-source lock.

The Surface maps the complete renderer palette to Nola design tokens, including manual appearance selection; native modal backgrounds use the same tokens. Text links reject URL credentials and control characters. Pending/deleted projections expose localized noninteractive status instead of an empty card. These changes affect rendering and URL admission only, not Core authority or wire interpretation.

Cold recovery retains the original submission's field edits and revision. Failed durable reads block new submissions and expose an explicit retry; a retry never invents a replacement request or discards the original intent. Recovery labels follow the client locale. Model diagrams use the separate Markdown renderer, not this form catalog.
