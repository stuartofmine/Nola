# MOD-08 · Model configuration and inference

[简体中文](README.md) · English · [Architecture](../../README.en.md)

Application owns connections, model profiles and default selection. It resolves session overrides against a profile revision. The Host binds the route and retrieves private credentials. The provider adapter encodes the prepared request. Automatic reasoning omits the effort field. Explicit none is a distinct advertised value.

## Runtime behavior

CLI and Web Host startup publish the file-declared connections, models and default together. A rejected catalog preserves the committed catalog. Invalid JSON or JSONC stops startup. Reasoning controls belong to a particular connection, model and protocol. Local model acquisition checks the executing machine, engine and target window before the user confirms a download.

## Failure and verification

Distinguish a catalog read from an inference attempt. Test repeated startup, a bad final entry and an invalid first-run default. On desktop, check engine readiness and available weights separately. A projected 128K window is a capacity estimate. Confirm actual context and output using a running model.

## Implementation

- [nola-application/src/model.rs](../../../../core/nola-application/src/model.rs)
- [nola-application/src/model_profile_resolution.rs](../../../../core/nola-application/src/model_profile_resolution.rs)
- [nola-credential-host/src/lib.rs](../../../../core/nola-credential-host/src/lib.rs)
- [cli/src/config_files.rs](../../../../apps/cli/src/config_files.rs)
- [cli/src/model_acquisition.rs](../../../../apps/cli/src/model_acquisition.rs)
- [nola-application/src/model_connection.rs](../../../../core/nola-application/src/model_connection.rs)

## Detailed references

[Desktop Local Models](DESKTOP_LOCAL_MODELS.en.md) · [Generation Override Contract](GENERATION_OVERRIDE_CONTRACT.md) · [Model Configuration Rewrite](MODEL_CONFIGURATION_REWRITE.md) · [Model Connection Import](MODEL_CONNECTION_IMPORT.md) · [Model File Configuration](MODEL_FILE_CONFIGURATION.en.md) · [Nola SIWC](NOLA_SIWC.md) · [Provider Instruction Projection](PROVIDER_INSTRUCTION_PROJECTION.md) · [Reasoning Catalog And Wire](REASONING_CATALOG_AND_WIRE.en.md) · [Shared Execution Host Configuration 20261015](SHARED_EXECUTION_HOST_CONFIGURATION_20261015.md)
