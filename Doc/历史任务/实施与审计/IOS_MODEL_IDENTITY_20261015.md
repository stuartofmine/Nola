> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# iOS canonical model identity implementation

The conversation's `modelSelectionID` is the sole model identity. `selectedModelInfo`
is a transient presentation projection, never a settings write source. Configured
profiles resolve by profile ID using canonical profile fields and standard source
connection fields; local profiles resolve by exact local runtime identity. A missing,
disabled, or unsupported profile leaves history readable and execution unavailable.
There is no current-global-model fallback.

Creation writes identity and initial parameters atomically. Explicit model selection
uses the canonical configuration command and its observed conversation revision;
existing per-model overrides survive switching away and back. Metadata/title writes
must neither serialize the projection nor change identity.

WebHost is an execution connection, not a configured LLM. Its binding is separate
from model identity, contains no credential, and resolves credentials from device
secure storage. Its remote model remains controlled by the connected Host. Host
binding changes and clearing a model identity must be one transaction.

Tests cover foreign standard profiles, absent models with readable history, model
selection preserving overrides, no persisted projection, and Host separation.
Implementation and test writing precede the coordinated validation batch.

## Implemented boundary

- `LocalConversationStore` derives the model from `modelSelectionID` and the canonical
  profile; absent profiles return readable conversations with an unavailable reason.
- `ModelSelectionManager.modelInfo` consumes standard profile identity/display/runtime
  fields and connection fields, not an encoded iOS `ModelInfo`. Foreign credential
  references are not converted into anonymous credentials; execution still requires
  a supported secure credential binding.
- Explicit selection uses `patch_conversation_configuration` CAS; existing parameter
  entries are validated and preserved. Title/settings updates discard the obsolete
  display snapshot and cannot select a model.
- Host binding is `{baseURL, tlsCertificateSHA256?}`. `clientHostParameters` uses shared
  generation-parameter encoding, and `clientModelPresentation["execution-host"]`
  owns only display preferences. Neither is an LLM identity. The Host chooses its
  own actual model when no remote profile was explicitly selected.
- Local identity resolution uses registered `mlx` profiles, not default suggestions.
  Runtime model installation/loading remains checked by the existing local runtime.

## Remaining boundaries / validation

No full remote `connectedHost:<host>/<profile>` catalog selector is added. Archive
imports preserve canonical identity, but the legacy transcript archive does not
carry a complete configuration aggregate; missing parameters still require explicit
configuration rather than live-global fallback. Unsent Host drafts currently resolve
the paired connection catalog; freezing the connection URL in the draft is separate
from the canonical conversation binding. No legacy display-snapshot migration occurs.

Added regression cases for standard foreign profiles, preserved overrides on switch
back, metadata unable to mutate model, missing-model readable history, and credential-
free Host binding. Existing local fixture profiles and cold-reopen setup now register
canonical profiles explicitly. The coordinated validation below covers this batch;
previous 55/55 evidence predates it and is not its acceptance result.

## Coordinated validation result

2026-10-15: seven selected iOS test classes passed **74/74**, zero failures,
zero skips, no runtime warnings. This includes canonical model selection tests,
draft/repository/aggregate tests, canonical store cold reopen, ModelService config
installation, and ChatManager concurrency/stream ownership tests.

- Result: `/tmp/nola-ios-identity.PXkdKE/Results.xcresult`
- Log: `/tmp/nola-ios-identity.PXkdKE/test.log`
- Simulator: available iPhone 18 Pro Max, iOS 27.0, one worker.
- DerivedData: `/tmp/nola-ios-final.zmOU0Z/DerivedData` (no Mac Debug output).
- Only build warning: existing NolaCore unsandboxed Copy Headers phase.
- No fixture/production correction was needed after this batch began. Real device,
  remote live-provider execution and remote Host model browsing are not claimed.

See `../../当前任务/MOBILE_MODEL_IDENTITY_AUDIT_20261015.md` and
`../../软件架构设计/模块/04-存储与索引/CONVERSATION_AGGREGATE_CONTRACT_20261015.md` for shared ownership constraints.
