> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Permission Boundary Contract

## Status

This document is normative for every Nola Host and client. It defines the
permission boundary independently from UI language, platform APIs, tools, and
transport.

## Dependency direction

```text
View -> Presenter/Application client -> Application -> Core policy
                                            |
                              committed resolution/permission effect
                                            v
                                    Driver -> Platform Host -> OS API
```

Core owns the pending request, policy evaluation, permission lifetime,
conversation grant, suspension, resume, and audit event. A Host adapter may
request an operating-system permission or resource picker. A View may only
render the current projection and submit one typed user decision.

No dependency may point in the opposite direction. In particular, Core must
not import platform authorization APIs, and a View must not evaluate tool
risk, resource scope, digests, Host identity, or permission policy.

## Distinct decision types

`PermissionPolicyDecision` is produced by Core policy:

- `allow`
- `ask`
- `deny`

`PermissionUserDecision` is submitted for one pending request:

- `allow_once`
- `allow_conversation`
- `deny`

These types are deliberately not interchangeable. `Ask` is not a user
decision. `AllowConversation` is not a Host-side Boolean cache.

The versioned Host protocol additionally supports presentation actions that
are not grants:

- `provide_instruction`
- `return_to_composer`

They are explicit enum cases. Localized text, `allow=true`, `remember=true`,
`grant_scope`, `session`, and aliases are not valid decision contracts.

## Request identity

A pending request is addressed by the Core-issued `request_id`. Its Turn,
call, capability, concrete resource, arguments, policy evaluation, and
continuation remain in the Core checkpoint. Clients must not reconstruct a
request from a tool bubble, Trace event, activity row, or locally cached plan.

A response that does not match the currently pending request is rejected.
Rejecting it must not consume or replace the real pending request.

## Structured grants

An optional `grant` carries platform evidence needed to execute the concrete
call, for example:

```json
{
  "uri": "content://provider/document/42",
  "access": "read_write"
}
```

The user decision controls lifetime. The grant controls resource evidence.
They are orthogonal. A resource URI, bookmark, command prefix, directory, or
URL must never be overloaded into a decision string.

Platform authorization is a Host fact, not a conversation grant. Granting an
OS permission can satisfy the current operation, but only Core may persist an
`allow_conversation` rule for later calls.

## Suspension and resume

1. Core evaluates the concrete capability and resource.
2. Core proposes the pending request/continuation; Application commits them.
3. Host renders `PermissionProjection` or invokes a platform picker.
4. UI submits a typed response by `request_id`.
5. Core computes the resolution; Application atomically commits closure,
   decision receipt and the exact continuation. Host execution follows commit.

Navigation, reconnect, process restart, and conversation switching must render
the checkpointed request again. A client-side continuation, callback closure,
or Boolean `isWaitingForPermission` is not authoritative.

## Presentation rules

- One visible sheet per pending Core request.
- No prompt generated from tool names or user keywords.
- No duplicate validation in View code.
- Platform setup failures remain attached to the same pending request.
- A failed picker does not silently become a denial.
- UI labels are localized projections of typed enum cases.

## Product permission profiles

Every product surface exposes exactly two canonical values:

- `cautious` (谨慎), the default: ordinary non-sensitive reads, public retrieval,
  new file creation and reversible operations in an approved scope proceed.
  Deletion, overwriting existing content, sensitive data access, outbound
  messages/private data and new external connections require confirmation.
  Using an already configured public retrieval service is not a new connection
  requiring approval for every HTTP request. Unknown operation effects require
  an honest concrete scope, not a false read-only classification.
- `allow_all` (全部允许): no Nola application confirmation, including directory,
  sensitive operations and explicitly delegated child work. OS enforcement,
  missing resources, invalid identity, actual credential revocation and
  cancellation remain real constraints, not another Nola approval layer.

Delete the old three-profile values, aliases and fallback readers. Invalid wire
values are rejected; a genuinely new configuration defaults to `cautious`.
Clients do not implement their own risk classifier or infer authorization from
user text, locale or a tool name. Core evaluates concrete operation/resource
facts. A prior denial or cancelled call is never revived by changing the mode.
Unrelated scheduled work uses its own committed execution profile; child work
inherits the explicit parent policy and scope, not the currently selected UI.

### Filesystem scope and model-requested sensitive reads

The workspace supplies the default working directory and project context; it is
not by itself a filesystem sandbox. In the CLI Host, an ordinary path outside
`allowed_dirs` is neither a sensitive resource nor a reason to request a directory
grant. Ordinary external reads and new ordinary files follow the same operation
policy as workspace files. An approved command may use an external temporary
path without a second `workspace_access` check. Overwrites, deletion, unknown
process effects, explicit deny rules and OS permissions retain their own checks.

The Host resolves the actual path, including symlinks, and identifies known
sensitive resources such as `.ssh`, `.env` and key files. A model may also judge
that an otherwise ordinary path contains private data. The file/process tool
schema exposes optional `sensitive_read_reason`: a nonempty explanation raises
an otherwise `OrdinaryRead` observation to `SensitiveRead`. It cannot lower a
Host risk, grant permission, or replace a user decision. The reason and concrete
input remain bound to the invocation; Core evaluates them under the current
profile and existing rules. `allow_all` still means no application confirmation.

For example, a model can request approval before reading a personal document:

```json
{
  "path": "/Users/example/Documents/medical-notes.txt",
  "sensitive_read_reason": "This document contains private medical information."
}
```

In cautious mode, sensitive reads use the existing concrete-operation approval
flow. Approval must permit the admitted operation to execute; the Host must not
then reject it solely because the path is outside the workspace. A changed
resource, explicit denial or OS refusal is still reported accurately.
`workspace_access` remains available for native Hosts requiring OS directory
access, not as a CLI prerequisite for crossing the workspace boundary. This
change does not broaden configured Skill roots or access to another session's
retained tool-output references.

Implementation and validation on 2026-10-20 cover the CLI/shared Rust Host path;
this is not evidence of native OS-picker parity on all platforms. Existing user
data and saved grants are not migrated or rewritten.

### Active policy changes

Explicit configuration edits commit by CAS even while streaming; stream archive
throttling must not suppress them. A committed permission change enters the
original Run through a durable policy-update command. Its acknowledgement is
the UI's evidence of current effect. It re-evaluates pending work that has not
crossed the physical admission boundary without a new model request. Core
`Dispatched` only means an envelope exists, not that execution has started.
Application must atomically withdraw and fence the old delivery, including a
leased worker that has not started, before Core re-admits it under the new
policy. Submitted envelopes are never edited in place. A switch to `allow_all`
can continue the same pending call, but cannot replay physically admitted or
unknown effects, prior denial or cancelled work. `ExecutionStarted` and the
durable admission receipt determine that boundary, not UI state or elapsed
lease time. New model/generation settings remain next-Run settings.
Failed/conflicting edits retain their draft and the prior effective value;
clients must not replace the expected revision with a fresh read to force a write.

## Policy precedence and replicated observations

Platform capability limits, explicit resource restrictions and revocation form
non-overridable bounds. Product defaults and scoped grants are evaluated only
inside those bounds; a later allow rule cannot override a higher-priority deny.
Resource resolution facts must not widen an existing decision. Invalid responses
are rejected before atomically consuming the real wait; same-answer replay is
idempotent and a conflicting answer cannot consume another request.

Remote grants are source-authority observations. Importing a snapshot does not
grant local resources or replace the receiving device's authorization set.

## Forbidden interfaces

- `resolvePermission(allow: Bool, remember: Bool)`
- `decision: String`
- `scope: String` where scope means both resource and lifetime
- UI-owned session/conversation permission caches
- permission inference from a capability name, command prefix, or user text
- Trace-driven permission restoration
- terminal events carrying an alternative permission result

## Acceptance evidence

Contract tests must cover:

- allow once and allow conversation for the same concrete call;
- denial without losing the pending identity;
- platform resource selection and cancellation;
- reconnect and cold-restart resume;
- delegated and automated origins not inheriting live Turn grants;
- simultaneous requests resolved by exact request ID;
- natural follow-ups and multilingual prompts without keyword routing;
- unknown-tool recovery without opening a permission prompt.
- both profiles on read, create, overwrite, delete, sensitive read, outbound
  message and new connection, including plugin/MCP/delegated boundaries;
- new settings defaulting to `cautious` on every surface, invalid values rejected;
- explicit edits during streaming, committed policy ACK, pending-call
  re-evaluation without an extra model request, cancellation/denial not revived;
- zero Nola confirmation under `allow_all`, including directory scope;
- `allow_all` not being inherited by a non-interactive origin;
- invalid profile wire values being rejected before a Turn starts.

Real E2E acceptance must additionally record the Turn/request/call identities,
open every captured screenshot, and watch the complete recording. The observer
must verify that exactly one permission UI appears, the conversation remains
visible behind it, the same tool resumes, and no duplicate tool or assistant
content is rendered.

## Migration state

The Core policy and user decision types are separate, and the versioned Host
response uses a typed decision plus structured grant/instruction fields.
Android, iOS, macOS, and Web Host decision payloads now use the same exact
`allow_once`, `allow_conversation`, and `deny` values. The old Web
`allow + grant_scope` parser has been deleted.

macOS decodes the dedicated `permission_request` Host envelope for its resume
handle; the embedded Trace is diagnostic presentation only. iOS restores a
pending sheet from `PermissionProjection`, not from a reconstructed Runtime
event. Device-tool correlation uses the Core-issued request identity and nonce,
not a client-recomputed argument digest.

Remaining work is deletion-oriented: remove UI Boolean decision APIs, move all
platform sheets to `PermissionCommands`, remove residual local permission
caches, and prove cold-restart resume with the same Core request ID.

### CLI filesystem validation — 2026-10-20

- Resource tests cover ordinary external reads/new files, overwrite risk,
  model-requested sensitive reads, sensitive symlink targets, approved sensitive
  Shell reads, external output redirection and recovery checkpoints.
- The final `tools::` run passed 591 tests with one ignored. Three separately
  observed failures were excluded: `concise_foundational_tool_descriptions_stay_within_authored_budgets`,
  `concise_foundational_tool_descriptions_keep_safety_boundaries`, and
  `pure_availability_preserves_default_and_invalid_manifest_semantics`. The full
  group is not reported as green.
- `scripts/test_real_cli_external_resources.py` passed three real-model turns
  using an isolated home/database and fixture files outside the workspace,
  without `--yes` or `allow_all`: Chinese initial read, natural follow-up, and
  English “Search/current” request. Four tool results, zero approval prompts,
  actual file markers recovered. This does not establish every sensitive-data
  approval interaction or every platform's behavior.
- `cargo build -p nola_cli --bin nola --quiet` succeeded. Restart an existing CLI
  process to load the rebuilt binary; no permission-mode change is required.


### Literal Shell inspection — 2026-10-20 follow-up

The latest CLI session exposed repeated confirmation for ordinary source inspection: the Host recognized only a narrow set of single commands, so `grep` and compound reads became `Unknown`. This is distinct from `grep` exit 1 (no match) or exit 2 (command/input error), which must not be rewritten as permission denial or success.

The shared Rust CLI/Web Host now constructs a bounded literal read plan. It validates each program's argv, resolves file operands (including missing leaves and symlink targets), and checks recursive trees without applying source-index directory exclusions. The executable paths are pinned, each subprocess receives a clean environment, and execution uses the validated reconstructed plan. Pipes and conditional/sequential separators retain their semantics. Only exact stderr redirection to `/dev/null` is supported by this contract.

Supported cases include nonrecursive/recursive `grep` with selected read-only options, bounded display commands, directory listing, and numeric print ranges of `sed -n`. This is not an arbitrary-shell read-only classifier: shell expansion, unrecognized programs/options, arbitrary sed scripts and file-writing redirects remain `Unknown`. Sensitive operands remain `SensitiveRead`; an inspect label cannot lower risk. Recursive inspection is bounded at 10,000 entries and rejects incomplete traversal rather than declaring the tree ordinary.

Regression fixtures retain 26 sanitized commands from the incident, including one unsupported wildcard-expansion case which must still require confirmation. Additional cases cover chained mutation, command substitution, pattern-file reads, external paths, sensitive symlinks, recursive sensitive files, and original-versus-pinned output/exit-code equivalence. Implementation changes do not constitute native Apple/Android or real-model acceptance; current results are recorded in the task ledger.


### Process execution grants versus execution snapshots — 2026-10-20

A valid `ProcessInvocation` is eligible for an explicitly selected conversation grant. Removing all such offers was an architectural error: an authorization scope is not a promise that script dependencies, environment variables or payload bytes never change. A process grant authorizes the declared invocation within its original conversation, Host, capability/operation, command, cwd and parameters; it does not authorize arbitrary Bash commands or infer an executable-family prefix.

Every subsequent invocation obtains fresh Host resolution before matching the grant. Observed process risk participates in the scope; sensitive reads additionally retain their Host evidence boundary, so new risk or changed sensitive evidence requires a new decision. Explicit constraints, revocation, one-shot effect identity and unknown-side-effect recovery remain independent. This is an execution authorization, not a claim that arbitrary shell side effects have been statically classified.

The Core and direct Rust Host share eligibility and observation semantics. CLI option 2 displays the actual command/cwd and remains selectable for valid process scopes. An ineligible challenge displays the disabled option with a reason and omits its shortcut. No stored user grants are rewritten. Runtime and client coverage is tracked separately in the task ledger.

### Configuration ownership snapshot and commit — 2026-10-21

`read_permission_configuration` reads configuration, active Run, exact recovery evidence, source admission and policy acknowledgement under one repository snapshot. It returns the configuration revision, credential-free settings, owner (`inactive`, `local`, or `remote`), active policy and application-derived state (`inactive`, `pending`, `active`). Unresolved Run IDs and unconfirmed source observations cannot silently become an inactive owner. Remote identity must come from validated source admission, not a transport spinner or client-composed receipt.

An observer may supply `minimum_policy_revision` from its successful save. Core evaluates acknowledgement against that committed boundary and the current desired profile; unrelated later settings revisions must not make a previously requested acknowledgement permanently pending. Polling cadence, cancellation and stale-page suppression remain client responsibilities.

Source and mirror configuration revisions are separate clocks. A mirror snapshot validates remote ownership but does not interpret a source policy revision against its local configuration or claim a source acknowledgement. Remote acknowledgement is read from the exact source Host using the source commit revision.

`set_conversation_permission` validates the expected Run/source owner inside the same write transaction as the configuration CAS and policy notification. An owner change rejects the write. A revision conflict remains a conflict rather than an automatic retry using newer settings. Independent settings may be included as a validated `additional_configuration` patch in the same commit; it cannot supply a second permission profile. The active policy remains pending until the Run acknowledges it; a source save, mirror save or HTTP success does not fabricate that acknowledgement.

The current implementation and regression status are tracked in [the cross-platform convergence task](../../../历史任务/2026-10-28_A2UI与跨端执行阶段归档/跨端业务复用与真实链路验收_20261020.md). This contract update is not a claim that runtime acceptance has passed.
