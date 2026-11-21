# MOD-12 · Memory, data sources and automation

[简体中文](README.md) · English · [Architecture](../../README.en.md)

Application owns memory records, source snapshots and automation definitions. A connector observes a resource. A scheduler submits an occurrence through the execution boundary used by other work. Persist the execution specification so a later edit to the definition cannot redirect work already admitted.

## Runtime behavior

Recall filters on the record owner and current conversation binding. A data-source snapshot records origin and freshness. An automation occurrence binds its definition revision and execution instance. Preparation failure, admitted execution and an uncertain result have different recovery paths.

## Failure and verification

Test owner isolation in recall, an expired or denied data source, duplicate triggers and interruption between claim and admission. Inspect the original invocation before issuing another external action.

## Implementation

- [nola-application/src/memory.rs](../../../../core/nola-application/src/memory.rs)
- [nola-application/src/data_source.rs](../../../../core/nola-application/src/data_source.rs)
- [nola-application/src/automation_execution.rs](../../../../core/nola-application/src/automation_execution.rs)

## Detailed references

[Automation Cross Platform Contract](AUTOMATION_CROSS_PLATFORM_CONTRACT.md) · [Background Data](BACKGROUND_DATA.md) · [Code Memory Native Architecture](CODE_MEMORY_NATIVE_ARCHITECTURE.md) · [Memory Execution Policy Contract](MEMORY_EXECUTION_POLICY_CONTRACT.md) · [Memory Recall Architecture](MEMORY_RECALL_ARCHITECTURE.md) · [Memory Scope V2](MEMORY_SCOPE_V2.md) · [Data-Source-Centered-Architecture](data-source-centered-architecture.md)
