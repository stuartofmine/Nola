# Nola architecture

[简体中文](README.md) · English

When a user sends a message, the client submits a command to Application. The agent engine proposes a transition, the Store commits it, and a Host performs the model or tool operation. The client reads the resulting journal. On a paired device the source Host still owns the run. These ownership boundaries explain which component to inspect after a failure.

## Architecture guides

| Guide | What it covers |
| --- | --- |
| [System design](总体设计/SOFTWARE_DESIGN.en.md) | From a submitted message to committed state, a model request and tool effects |
| [Product requirements](总体设计/REQUIREMENTS.en.md) | Product scenarios and the REQ-ARCH and REQ-RUN acceptance contracts |
| [Deployment and source map](总体设计/CURRENT_IMPLEMENTATION_MAP.en.md) | Six client surfaces, their processes, native artifacts and module entry points |
| [Rust dependencies](总体设计/DEPENDENCIES.en.md) | Direct production dependencies extracted from workspace manifests |
| [Module and command contracts](MODULE_CONTRACTS.en.md) | Request identities, receipts, revision conflicts and error ownership |
| [Workflows and recovery](总体设计/WORKFLOWS_AND_RECOVERY.en.md) | Edited resend, long tasks, compaction, disconnects and cancellation |
| [Startup configuration](模块/08-模型配置与调用/MODEL_FILE_CONFIGURATION.en.md) | File precedence, atomic publication, credential binding and startup failure |
| [API guide](模块/01-领域与协议/API.md) | Rust Host HTTP, Service commands and embedded entry points |
| [Version and data compatibility](总体设计/VERSION_COMPATIBILITY.en.md) | The reissued 1.0.0 baseline, negotiation, recoverable migration and preservation of user data |
| [Distribution and updates](总体设计/SOFTWARE_UPDATE_ARCHITECTURE.en.md) | Signed feeds, artifact selection and platform installation handoff |

- [Write ownership](总体设计/OWNERSHIP.en.md) · [Core, Application and Host](总体设计/CORE_HOST_ARCHITECTURE.en.md) · [Layering and lifetimes](总体设计/CORE_HOST_LAYERING.en.md)

## Modules

Each module describes its authority, inputs, failure handling and implementation. Module numbers identify responsibilities and can span several crates.

- [MOD-01 · Domain and protocols](模块/01-领域与协议/README.en.md)
- [MOD-02 · Core and the agent state machine](模块/02-Core与状态机/README.en.md)
- [MOD-03 · Application commands and transactions](模块/03-Application与事务/README.en.md)
- [MOD-04 · Storage, journals and indexes](模块/04-存储与索引/README.en.md)
- [MOD-05 · Execution, cancellation and recovery](模块/05-执行与恢复/README.en.md)
- [MOD-06 · Tools and execution evidence](模块/06-工具与证据/README.en.md)
- [MOD-07 · Permissions and platform capabilities](模块/07-权限与平台/README.en.md)
- [MOD-08 · Model configuration and inference](模块/08-模型配置与调用/README.en.md)
- [MOD-09 · Context, compaction and recall](模块/09-上下文与压缩/README.en.md)
- [MOD-10 · Goals, tasks and experts](模块/10-Goal任务与子代理/README.en.md)
- [MOD-11 · Skills, plugins and MCP](模块/11-Skill插件与MCP/README.en.md)
- [MOD-12 · Memory, data sources and automation](模块/12-记忆数据源与自动化/README.en.md)
- [MOD-13 · Clients, conversation projections and A2UI](模块/13-客户端投影/README.en.md)
- [MOD-14 · Remote Hosts and synchronization](模块/14-远程Host与同步/README.en.md)
- [MOD-15 · Measurement, tests and release acceptance](模块/15-观测与验收/README.en.md)

## Verification and further reading

The overview, module guides and core contracts are available in both languages. Specialist specifications and dated audits retain their original languages. Source links identify implemented boundaries. Verification sections describe the evidence needed for a claim. For example, a receipt can prove a commit, while a device interaction test can show that returning to the screen restores focus.

[Design and prototypes](../design/README.md) · [Platform documentation](../platforms/README.md) · [Validation scope](../quality/VALIDATION_SCOPE.md) · [November 8 source audit](总体设计/ARCHITECTURE_SOURCE_AUDIT_20261108.md)
