# MOD-09 · Context, compaction and recall

[简体中文](README.md) · English · [Architecture](../../README.en.md)

Core chooses an eligible history range and summary budget. The Host makes the maintenance request. Store commits a validated summary with its source references. Each participant’s context has a defined conversation owner. A physical request snapshot is needed to establish eligibility for prefix reuse.

## Runtime behavior

The first summary request targets half the estimated full input. Fixed instructions, tool schemas and the retained tail consume the same budget. Report the achieved reduction when little history can be removed. Original text and summary lineage remain readable. Cache hit data comes from provider usage on the physical request.

## Failure and verification

Compute reduction as 1 − estimated full input after / estimated full input before. Record the trigger threshold separately. Test truncated or cancelled summaries and restart during maintenance. Missing usage stays unknown. An estimate alone cannot establish a cache hit.

## Implementation

- [nola-application/src/context.rs](../../../../core/nola-application/src/context.rs)
- [agent/runtime/compaction_driver.rs](../../../../core/nola-agent-engine/src/agent/runtime/compaction_driver.rs)
- [src/llm_parts/request_snapshot.rs](../../../../apps/cli/src/llm_parts/request_snapshot.rs)

## Detailed references

[Context Accounting V2](CONTEXT_ACCOUNTING_V2.md) · [Context And Accounting](CONTEXT_AND_ACCOUNTING.md) · [Harness Context Projection](HARNESS_CONTEXT_PROJECTION.md) · [Incremental Context Design](INCREMENTAL_CONTEXT_DESIGN.md)
