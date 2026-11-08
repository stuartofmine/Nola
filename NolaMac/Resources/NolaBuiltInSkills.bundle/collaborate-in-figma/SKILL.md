---
name: collaborate-in-figma
description: Inspect, audit, explain, prototype, or edit Figma files, pages, frames, components, variables, and prototypes. Use when a Figma URL, node, design system, handoff, or UI audit is central; use advisory mode when no authorized Figma connection or authenticated browser is available.
---

# Collaborate in Figma

## Nola runtime contract

- Read `NOLA_MANIFEST.json` before execution. Confirm required tools, runtime dependencies, permissions, and validation gates before starting expensive work.
- All Skill files and scripts are bundled locally. Never download a replacement Skill, run a package installer, or silently substitute an unavailable tool.
- Remove temporary helper files before the final validation. After the last successful validation, do not mutate the workspace again; report the validated artifact directly.
- Keep tool results, user-visible progress, and final claims distinct. A draft, preview, local artifact, or advisory report is not evidence of an external write.

## Skill coordination

- Use OCR only for supplied raster references and document review only for attached specifications. A browser or local snapshot is not a substitute for a structured Figma write receipt.
- Load only the smallest set of Skills needed. Pass verified artifacts, identifiers, source anchors, and unresolved constraints across a handoff; do not replay the same discovery or computation.


## Operating modes

- **Inspect**: read hierarchy, styles, variables, components, annotations, and prototype links without changing the file.
- **Audit**: evaluate hierarchy, consistency, accessibility, responsiveness, and design-system usage with node-level findings.
- **Edit**: apply scoped changes through an authorized structured connector, or an approved authenticated browser when no connector exists.
- **Handoff**: produce an implementation-ready specification when direct access is unavailable or the user wants developer guidance.

## Readiness

- Use `tool_search` to discover a structured Figma MCP tool by its concrete published name; never call a generic tool named `mcp`. Otherwise request an exact file or node URL and use the `chrome_native_*` tools with the user's existing authenticated Chrome session.
- Test read access before promising inspection and write access before promising edits. Never ask for credentials in chat.
- Identify file, page, frame, component or instance, viewport, desired outcome, brand system, and whether shared-library publication is in scope.
- Capture a read-only before snapshot: node IDs, component relationships, variables, styles, Auto Layout, constraints, prototype links, and relevant screenshots.

## Progress contract

Use stable stage messages only: connecting to Figma, reading target nodes, auditing design, applying scoped edits, comparing states, and validating handoff. Do not flash the same status when individual node calls change.

## Workflow

1. Map the target hierarchy and distinguish main components, variants, instances, local overrides, variables, styles, and detached content.
2. For audits, record each finding as severity, node ID, evidence, user impact, recommended change, and confidence.
3. For edits, present a compact plan and modify source components or variables when appropriate instead of duplicating styles or detaching instances.
4. Preserve the existing visual language unless redesign was requested. Maintain Auto Layout, constraints, naming, token semantics, and responsive states.
5. Check empty, loading, error, disabled, hover, focus, selected, and overflow states when relevant.
6. In advisory mode, provide exact nodes, values, token mappings, responsive rules, and acceptance criteria; never imply the file changed.

## Snapshot audit contract

When a live structured connector is unavailable, use
`scripts/audit_snapshot.py --input <snapshot.json> --output <handoff.json>`.
The snapshot must contain stable node IDs, node types, dimensions, layout
properties, component relationships, states, and relevant variables. The audit
computes findings from that snapshot and always emits `mode: advisory` and
`applied: false`; a local handoff is not evidence that Figma changed.

Reopen the result with `scripts/audit_snapshot.py --validate-only --output
<handoff.json>`. Do not replace this contract with a generic prose handoff that
cannot be traced back to node IDs.

If the supplied snapshot is incomplete and therefore cannot satisfy the JSON
validator, do not silently fall back to chat-only prose. Create a Markdown
advisory report in the active workspace containing:

- the exact source URL or snapshot path and every stable file, page, frame, and node ID that was actually observed;
- an `Evidence available` section that separates observed properties from inference;
- node-level findings with severity, evidence, impact, recommendation, and confidence;
- a `Validation limitations` section listing each missing canonical field and the connector or snapshot data needed to remove that limitation;
- explicit `Advisory: no Figma changes were applied` status.

Reopen the Markdown report with `read_file` as the final validation and do not
mutate it afterward. This degraded artifact is a traceable handoff, not a
successful structured snapshot audit or evidence of an external edit.

## User control

- Confirm before deleting frames, detaching instances, replacing components, changing shared variables, modifying many instances, publishing a library, or changing prototype entry points.
- Keep collaborator names, comments, private assets, and file metadata within the request scope.
- Do not use Computer Use for broad edits when a structured connector is available.

## Validation

- Re-read changed node IDs and compare before and after properties.
- Check Auto Layout, constraints, spacing rhythm, type scale, color variables, contrast, focus visibility, clipping, component states, and representative viewport sizes.
- Verify prototype links, main-component relationships, instance overrides, exports, and that no unrelated shared token changed.
- Capture an after snapshot for every changed frame and reconcile it with the approved plan.

## Failure handling

Stop after access loss, stale node IDs, conflicting concurrent edits, or an unexpected shared-component blast radius. Preserve completed scoped edits, report exact nodes, and do not retry destructive actions blindly.

## Completion

State `Applied` or `Advisory`, list file/page/frame/node IDs, summarize changes or findings, report validation and unresolved limitations, and link the exact Figma target when available.
