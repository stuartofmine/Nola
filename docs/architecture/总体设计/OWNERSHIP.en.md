# Architecture decisions and write ownership

[简体中文](架构决策与所有权.md) · English

| Fact | Owner | Binding and change rule |
| --- | --- | --- |
| Connection, account reference and profile | Application model catalog on the chosen Host | Check identity and expected revision. Resolve the private credential on the executing Host. |
| Conversation settings | The owning conversation | Apply sparse overrides against its chosen profile. A global edit takes effect only at an appropriate new resolution boundary. |
| Run parameters and tool policy | The admitted run | Capture a redacted invocation snapshot. Resume using the same committed binding. Explicit permission changes use their own acknowledged control path. |
| Message and part | The conversation ledger | Append or revise under a stable message/part identity. Edits rewind through the Application use case. |
| Expert configuration and history | The template owns its configuration. The parent conversation owns the membership relation. The child conversation owns the expert’s history. | Bind a revision and scope each Task result to its original parent. |
| Remote run | The authenticated source Host | The observing device imports source records with their cursor and epoch. A local cache confers no execution lease. |
| OS resource and effect | The executing Host under Core admission | Inspect the actual resource. Bind the result to the original effect and attempt. |
| Draft and interaction state | The editing device or view | Scope text, attachments and delayed callbacks to the originating conversation. |
| Measurement | The physical attempt | Reconcile late reports against the same attempt. Trace is disposable. |

**Concurrency.** An entity revision is a precondition for mutation. A request ID deduplicates an intent. A cursor orders observation. A worker generation fences late execution. A versioned protocol describes a data contract. Substituting one identifier for another can produce a plausible-looking but invalid recovery.

**Configuration.** Resolve Host defaults, session overrides and capability constraints before admitting execution. For CLI model files, validate the effective catalog and publish it atomically at startup. On rejection the previous committed catalog remains available. Existing conversation and run bindings have their own lifetimes.

**Reading versus changing.** An export, search index, UI projection or summary can be rebuilt from its authoritative records. Applying changes from any of them requires an explicit command and its preconditions. Across Stores use source receipts and reconciliation. [System design](SOFTWARE_DESIGN.en.md) · [Module contracts](../MODULE_CONTRACTS.en.md) · [Deployment](CURRENT_IMPLEMENTATION_MAP.en.md)
