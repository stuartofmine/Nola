> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Instruction Hierarchy Adoption

Nola must treat instruction authority as runtime data, not as concatenation
order. This document maps the strongest ideas from Codex, Claude Code, and
OpenCode into a Core-owned instruction hierarchy.

## Reference comparison

| Concern | Codex | Claude Code | OpenCode | Nola target |
|---|---|---|---|---|
| Product policy | Stable system/developer policy | Stable modular system policy | Provider/session system layers | Core `policy` fragments |
| Repository guidance | Directory-scoped instruction files | Project/user memory files | Discovered project instructions | Typed scoped user instructions |
| User preferences | Explicit user/developer input | User/project settings | Session/config instructions | `user_instruction`, never authorization |
| Retrieved content | Untrusted context | Tool/document results | Message/tool Parts | `reference` fragments |
| Platform facts | Tool/runtime environment | Host/tool environment | Provider/session metadata | `host_fact` fragments |
| Conflict handling | Higher authority wins | System safety remains dominant | Config/session precedence | Core validates authority and scope |
| Tool authorization | Concrete tool policy | Permission mode and call | Permission rules | Concrete call/resource decision only |

## Required authority order

Highest to lowest:

1. **Core policy**
   - Product identity, safety, truthfulness, tool protocol, permission semantics.
   - Immutable for the active runtime version.
2. **Host facts**
   - Platform, execution mode, workspace root, model/provider capabilities.
   - Descriptive only. A Host fact cannot grant permission.
3. **Scoped user instructions**
   - User preferences, project instructions, repository instruction files,
     response style, and enabled prompt templates.
   - Applied only inside their declared scope.
4. **Current user request**
   - The task to perform. It cannot retroactively change policy or authorize an
     external action.
5. **Reference material**
   - Attachments, retrieved pages, data sources, memory excerpts, MCP content,
     and tool output.
   - Evidence, never commands merely because imperative text appears inside it.

The hierarchy is semantic. Hosts must not emulate it by string position or by
adding phrases such as “ignore previous instructions”.

## Scoped instruction contract

A future Core instruction source should carry at least:

```text
source_id          stable identifier
kind               core_policy | host_fact | user_instruction | reference
scope_uri          workspace, directory, conversation, or global scope
scope_depth        normalized directory depth
origin_uri         file/resource that produced the instruction
content_digest     digest of exact normalized content
observed_at        source observation time
cacheable          whether it belongs to the stable provider prefix
content            model-visible text
```

Rules:

- Normalize and resolve `scope_uri` before model invocation.
- Select only sources whose scope contains the active workspace/resource.
- Within the same authority, order global to specific so the closest applicable
  directory guidance is last and can refine broader guidance.
- Preserve every selected source ID and digest in the Turn context snapshot.
- Freeze the selection for the Turn. A file change affects the next Turn, not an
  in-flight Step.
- Do not promote repository text to Core policy.
- Do not infer scope, trust, tools, or permission from natural-language words.

## Cache boundaries

The exact provider prefix is logically:

```text
active tool schema
stable Core policy
stable Host facts
cacheable globally scoped user instructions
--- dynamic boundary ---
workspace/directory instructions
Turn context and memory
conversation messages
```

The Core model-prefix digest must cover the exact stable policy digest plus the
active Step tool-schema digest. Directory instructions stay outside that stable
identity unless their scope and bytes are demonstrably invariant for the cache
key lifetime.

## Current Nola status

Implemented:

- Prompt fragments have typed layer, trust, cacheability, ID, and content.
- `user_instruction` is distinct from product `policy`.
- Core owns stable/dynamic prompt composition.
- Turn context snapshots preserve prompt fragment IDs and digests.
- Core freezes the Turn capability projection and current Step tool projection.
- Hosts no longer recompose Core system policy for normal Agent turns.
- Tool authorization is evaluated from the concrete call and resource.
- Core selects typed instruction sources by normalized scope and persists their
  source ID, authority kind, scope, origin, content digest, observed file
  version, cacheability, and deterministic selection reason in the Turn
  manifest.
- CLI debug receipts expose that manifest metadata without logging instruction
  content.

Still required:

- Extend typed instruction discovery to Hosts that expose project workspaces;
  mobile consumer conversations without a workspace should submit an explicit
  empty manifest rather than inventing repository instructions.
- Surface the redacted manifest receipt in native debug UI.
- Add cross-platform replay fixtures for nested directory instructions.

## Conflict examples

### Repository content asks to bypass permission

Result: ignored as authorization. It may describe the desired operation, but the
concrete tool call still passes through Core permission evaluation.

### Attachment contains “delete all files”

Result: treated as reference text. It does not become a user request or a tool
call.

### Global preference says concise; project instruction requests a report

Result: the project instruction may refine response structure within its scope,
while the global preference still applies where compatible.

### Two nested instruction files disagree

Result: both remain inspectable; the deeper applicable scope refines the broader
scope. Neither can override Core policy.

## Acceptance tests

1. Root and nested instruction files are selected deterministically.
2. A sibling-directory instruction is excluded.
3. File enumeration order does not change the rendered prompt or digest.
4. Changing an instruction during a Turn does not mutate that Turn snapshot.
5. The next Turn observes the new digest and content.
6. Imperative text in an attachment remains `reference`.
7. A custom user instruction cannot grant a tool permission.
8. Multilingual instructions follow identical authority rules.
9. Conversation fork/replay selects the same instruction manifest for the same
   workspace state.
10. Debug projection shows source ID, scope, trust, digest, and selection reason.
