# MOD-11 · Skills, plugins and MCP

[简体中文](README.md) · English · [Architecture](../../README.en.md)

Application stores versioned extension declarations. The Skill Host resolves and installs content. The MCP Host connects and discovers tools. Pi and DeepSeek Harness use their own runtime and installation state. The active session can call a tool only after catalog publication and admission.

## Runtime behavior

Choose the resource and the installation Host first. GitHub Skills pin a repository commit. MCP connections use a transport supported by their Host. Desktop plugin page entries come from the installed package declaration. Pi and DSH render runtime-specific interfaces. A phone and a paired Host maintain separate capabilities.

## Failure and verification

Test a repository containing multiple Skills, refusal of installation, disconnect during discovery and a stale tool after an upgrade. Mobile local Skills and HTTP MCP have different requirements from desktop stdio and third-party processes.

## Implementation

- [nola-application/src/skill.rs](../../../../core/nola-application/src/skill.rs)
- [nola-application/src/mcp.rs](../../../../core/nola-application/src/mcp.rs)
- [nola-mcp-host/src/lib.rs](../../../../core/nola-mcp-host/src/lib.rs)
- [nola-skill-host/src/lib.rs](../../../../core/nola-skill-host/src/lib.rs)
- [cli/src/harness_plugin.rs](../../../../apps/cli/src/harness_plugin.rs)

## Detailed references

[Harness Plugins](HARNESS_PLUGINS.en.md) · [Scale And Plugin Runtime 20261015](SCALE_AND_PLUGIN_RUNTIME_20261015.md) · [Skill-Runtime-Architecture](skill-runtime-architecture.md)
