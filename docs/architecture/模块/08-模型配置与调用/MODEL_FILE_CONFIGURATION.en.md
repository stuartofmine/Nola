# Model configuration files and startup publication

[简体中文](MODEL_FILE_CONFIGURATION.md) · English

Local CLI conversations, init, wiki, Web and server read startup configuration. The effective model catalog declares the connections, their models and the Host-wide default. Application publishes them together. A new conversation resolves the committed default. A resumed conversation restores its bound selection. [Startup hook](../../../../crates/nola-app/src/config.rs) · [Application transaction](../../../../crates/nola-app/src/config.rs)

## File discovery and input

Read user `.nola/config.jsonc` then `.nola/config.json`, followed by project layers from the nearest Git root down to the project directory. Outside a repository, discovery uses the user and project directories. Each project level reads `.nola/config`, `nola` and the supported `.nola/current_config` pair, JSONC before JSON. A more specific catalog replaces the earlier declaration as a whole. A language-only file leaves the catalog source in place. [Runtime configuration](../../../../crates/nola-app/src/config.rs)

A connection declares an ID, display name, endpoint, protocol and models. The default names exact connection and model IDs from the same file. A direct endpoint/model configuration and a catalog cannot supply competing routes. A minimal declaration follows.

```json
{"llm":{"connections":[{"id":"local","name":"Local","endpoint":"http://127.0.0.1:8080/v1","protocol":"chat_completions","models":[{"id":"my-model","name":"My model"}]}],"default_model":{"connection":"local","model":"my-model"}}}
```

## Publication and credentials

1. Validate the catalog source, declared identities, protocol, endpoints and default. Check for a gateway loop, name collisions and credential binding errors. Catalog publication requires no provider inference.
2. For a credentialed connection, declare the endpoint, protocol and `api_key_env` in user configuration. A project can use that binding or reuse its existing ready credential on the same route. Credentials reside in the Host-private vault. Changing the endpoint or protocol requires an appropriate binding.
3. Application checks expected revisions and commits all connections, model enablement and default selection in one transaction. Reopening unchanged declarations preserves the connection and profile revisions. Removing a declaration from the file leaves previously saved records available for explicit management.
4. A rejected model catalog reports the file and an edit command. A fresh chat may use a previously saved, enabled default. Without a usable default it stops before requesting inference. Malformed JSON/JSONC prevents startup configuration from being read. Inspection and syntax checking can be run independently.

## Using and verifying the configuration

Save the file and start `nola`. Use `nola config` or the conversation command `/config` to inspect sources and the saved default. `nola config check` validates without publishing. The older explicit apply command remains available for existing scripts. CLI and Web share settings only when they use the same Host database, including any explicit `NOLA_DB_PATH`.

The isolated CLI regression covers clean startup, rejection, unchanged restarts, credential rotation and Web/server startup using a local HTTP model endpoint. A real provider call exercises a separate boundary. [Example configuration](../../../../crates/nola-app/src/config.rs) · [End-to-end test](../../../../crates/nola-app/src/config.rs)
