# MOD-06 · Tools and execution evidence

[简体中文](README.md) · English · [Architecture](../../README.en.md)

An admitted call binds the schema, resource, permission and cancellation scope. The result can contain model-visible evidence, a reference to complete output and private presentation controls. Expose only the records allowed for each reader.

## Runtime behavior

The model selects a tool from its exposed schema. The Host reports the resource, result and checks actually observed. Application records their source. Reading a browser page, acting on it and observing the result can fail independently.

## Failure and verification

Test a missing browser page, partial output and a tool that exits successfully without producing the requested artifact. Keep preview tokens out of model input, summaries and recall. A verified change needs an observed result on the requested resource.

## Implementation

- [nola-application/src/tool_source.rs](../../../../core/nola-application/src/tool_source.rs)
- [nola-store/src/tool_source.rs](../../../../core/nola-store/src/tool_source.rs)
- [nola-domain/src/execution_evidence.rs](../../../../core/nola-domain/src/execution_evidence.rs)

## Detailed references

[Browser Automation](BROWSER_AUTOMATION.md) · [CLI SOP Capability Long Task Design](CLI_SOP_CAPABILITY_LONG_TASK_DESIGN.md) · [Code Mode](CODE_MODE.md) · [Harness Result And Admission Contract](HARNESS_RESULT_AND_ADMISSION_CONTRACT.md) · [Tool Transcript Source Architecture 20261017](TOOL_TRANSCRIPT_SOURCE_ARCHITECTURE_20261017.md)
