# Reasoning levels from model catalog to provider request

[简体中文](REASONING_CATALOG_AND_WIRE.md) · English

A reasoning flag, a list of effort values, a token budget, a thinking toggle and a public summary are separate capabilities. Key a catalog observation by connection, model and wire protocol. Discovery is a bounded catalog read. It does not start inference or select a default profile.

1. Read advertised levels from structured supported_reasoning_levels or reasoning_levels. Preserve order and remove duplicates. An explicit empty list and missing metadata convey different information. A model ID alone does not identify supported levels.
2. Store the catalog observation and resolve the selected profile revision plus session overrides. The automatic choice uses the provider default. Validate any explicit effort against this profile. On model change resolve again.
3. Freeze the effective selection for the attempt. A failure is surfaced on its original route. Switching protocol or changing an explicit effort requires a valid new selection.

| Protocol | Explicit selection |
| --- | --- |
| Chat Completions | Encode an advertised value in `reasoning_effort`. |
| Responses | Encode it in `reasoning.effort`. |
| Anthropic | Use `output_config.effort` for an advertised effort. A supported explicit off uses disabled thinking. A separately supported token budget uses enabled thinking and `budget_tokens`. |

Automatic omits the corresponding control. Only an advertised explicit none requests that value or its protocol-specific representation. Tool compatibility and public summary support are checked independently. A supported effort does not promise that a public summary will appear. Mobile local adapters and remote Hosts may implement different protocols. [Profile resolution](../../../../crates/nola-app/src/config.rs) · [Model configuration](README.en.md)
