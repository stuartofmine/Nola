# MOD-08 · Model configuration and inference

[简体中文](README.md) · English · [Architecture](../../README.en.md)

Application owns connections, model profiles and default selection. It resolves session overrides against a profile revision. The Host binds the route and retrieves private credentials. The provider adapter encodes the prepared request. Automatic reasoning omits the effort field. Explicit none is a distinct advertised value.

## Runtime behavior

CLI and Web Host startup publish the file-declared connections, models and default together. A rejected catalog preserves the committed catalog. Invalid JSON or JSONC stops startup. Reasoning controls belong to a particular connection, model and protocol. Local model acquisition checks the executing machine, engine and target window before the user confirms a download.

## Failure and verification

Distinguish a catalog read from an inference attempt. Test repeated startup, a bad final entry and an invalid first-run default. On desktop, check engine readiness and available weights separately. A projected 128K window is a capacity estimate. Confirm actual context and output using a running model.

## Implementation

- [nola-application/src/model.rs](../../../../crates/nola-llm/src/lib.rs)
- [nola-application/src/model_profile_resolution.rs](../../../../crates/nola-app/src/config.rs)
- [nola-credential-host/src/lib.rs](../../../../crates/nola-store/src/vault.rs)
- [cli/src/config_files.rs](../../../../crates/nola-app/src/config.rs)
- [cli/src/model_acquisition.rs](../../../../crates/nola-modules/src/local_models_plugins.rs)
- [nola-application/src/model_connection.rs](../../../../crates/nola-app/src/config.rs)

## Redaction before inference

Every Nola provider adapter passes a request copy through Rust Core's `nola_model_privacy::redact_model_request`. CLI, desktop Host and model gateway adapters call Rust directly. Apple and Android direct and on-device adapters use the same C ABI. Replayed history, tool results and auxiliary requests are covered. The local conversation retains its original content.

Recognized email addresses, supported phone formats and mainland Chinese identity number patterns become `[EMAIL]`, `[PHONE]` and `[ID]` for remote/public endpoints; local and private endpoints (loopback, RFC1918/CGNAT/link-local, IPv6 ULA, `.local`, and container host bridges) skip PII redaction by default unless overridden via `NOLA_REDACT_PII=0/1`. Recognized credentials, PEM private keys and sensitive structured fields become `[CREDENTIAL]` across all endpoints; labeled credential matching stays single-line (never spanning newlines) and excludes shell variable expansions (`$VAR`/`${...}`) and non-secret policy/algorithm identifiers so scripts and structured config payloads are not corrupted. Core processes human-readable tool descriptions, examples and serialized tool payloads. It preserves protocol identifiers, tool schemas, authentication headers, binary attachments and signed or encrypted continuation data.

Placeholders identify a category. Core maintains no reverse lookup and grants no tool authority to recover an original value. The rules cannot identify arbitrary personal names, postal addresses, text inside images or the contents of binary files. A paired remote Host receives the original conversation under its existing authorization.

Run `python3 scripts/test_model_request_privacy.py` to inspect first-turn and replayed HTTP requests for three protocols, a real file-tool result and the original local message.

## Repetitive reasoning resample and stream presentation (2026-11-24)

On tool-capable turns, when `ReasoningProgressGuard` aborts degenerate reasoning (`model_repetitive_reasoning`) before any visible `TextDelta` is emitted, the CLI streaming adapter ([streaming.rs](../../../../crates/nola-llm/src/sse.rs)) performs a single transactional resample (`LlmEvent::Retrying`, with `LlmEvent::SamplingStarted` clearing prior reasoning buffers). `ModelTextPresentationGate::push_text_against_reasoning` preserves opening assistant words that happen to share a short prefix with `<think>`.

## Detailed references

[Desktop Local Models](DESKTOP_LOCAL_MODELS.en.md) · [Generation Override Contract](GENERATION_OVERRIDE_CONTRACT.md) · [Model Configuration Rewrite](MODEL_CONFIGURATION_REWRITE.md) · [Model Connection Import](MODEL_CONNECTION_IMPORT.md) · [Model File Configuration](MODEL_FILE_CONFIGURATION.en.md) · [Nola SIWC](NOLA_SIWC.md) · [Provider Instruction Projection](PROVIDER_INSTRUCTION_PROJECTION.md) · [Reasoning Catalog And Wire](REASONING_CATALOG_AND_WIRE.en.md) · [Shared Execution Host Configuration 20261015](SHARED_EXECUTION_HOST_CONFIGURATION_20261015.md)
