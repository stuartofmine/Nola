> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Mac explicit model configuration commits

## Revision-isolated follow-up design

Conversation loading now must carry settings and canonical revision from the same
metadata read. Editors submit that original revision; reading a newer revision in
the background may diagnose a conflict but never authorizes overwriting it. Model,
parameter and execution-location edits share the configuration CAS. A successful
receipt advances only its original owner's revision. Content or external metadata
writes can invalidate an editor; the response is an explicit reload requirement,
not automatic retry. This deliberately detects ABA even when field values match.

Ordinary metadata cannot write identity, model parameters or execution location.
Its receipt may advance the displayed configuration revision only when its input
revision matches the revision immediately before that write. Otherwise the existing
projection remains stale and configuration editing requires a fresh read. Host
selection is an independent action, but selecting a model plus its required host
commits both fields atomically. Validation of this follow-up is recorded separately
from the preceding batch below.

## Confirmed defect

`MacWorkspaceModel.selectModel` previously changed the presentation model and
scheduled ordinary metadata persistence. `writeMetadata` serialized every field,
including `modelSelectionID`; the canonical metadata command correctly rejects
identity writes. Merely dropping that field would silently stop model selection
from being persisted. The same stale metadata could overwrite `modelOverrides`
initialized by a successful configuration transaction.

## Implemented boundary

- Ordinary metadata excludes model identity, execution location and per-model parameter overrides,
  including absent-field null generation. Existing fields remain stored unchanged.
- Explicit model selection, reset selection, and parameter editing use
  `patch_conversation_configuration` through a typed application-port wrapper.
- Repository operations run on the existing serial metadata queue, preserving
  ordering with ordinary metadata writes and deletion without blocking the UI.
- The operation captures the original conversation owner and displayed identity.
  It rejects mismatched stored identity. Parameter edits additionally compare the
  displayed typed parameter baseline, then emit only changed fields. A removed
  field emits JSON null, while fields unknown to Mac remain untouched.
- The actual transaction uses the original displayed canonical revision. Its committed
  receipt supplies model identity and parameter projection. Only after success
  does Workspace update that original conversation; a callback cannot update a
  newly selected conversation. Failure does not display a successful model change.
- Unsaved drafts retain their existing draft persistence path. This is not a
  redesign of the draft repository or its canonical promotion behavior.

## Regression coverage / verification

Added `testConfigurationCommitSurvivesStaleMetadataAndParameterClearPreservesForeignFields`:
model switch survives a queued old metadata snapshot; clearing temperature removes
it; reasoning fields unknown to the Mac override DTO survive; stale identity edits
are rejected. Existing two Mac metadata fixtures now register real configured
profiles rather than using invalid bare model IDs. `git diff --check` passes.
Integrated Mac regression passed 43/43 on 2026-10-15, zero skips or runtime warnings.
Evidence: `/tmp/nola-mac-identity.LAj42f/Results-final.xcresult` and `tests-final.log`.
Four suites cover configuration, canonical repository, connected-Host selection,
and default-model submission boundaries. Tests used the Test configuration on the
architecture-qualified local Mac; no unsigned Debug app was produced. Existing
headermap and unsandboxed Copy Headers build warnings remain. This is not manual UI
or live-provider acceptance.

The first integrated asynchronous run exposed three tests reading/submitting before
configuration commit. Added an owner-scoped pending gate to composer state, `send`,
and remote intent preparation. A second edit during pending is explicitly rejected
instead of being silently applied against an old baseline. Completion releases only
that owner's gate and resumes `waitForModelConfigurationCommit` with its result.
Tests now await that receipt, and exercise blocked sending with the draft preserved.
The 43-test result above covers that initial asynchronous commit batch, not the
subsequent strict revision and atomic execution-location changes.

## Explicit limitations

Revision/Host follow-up validation (2026-10-15): **44/44 passed**, zero skips and
zero runtime warnings. Evidence: `/tmp/nola-mac-owner-cas.SmuXWv/Results-final.xcresult`
and `tests-final.log`. The first run exposed a persistence-only fixture accidentally
attaching window observation after an async yield; the fixture now explicitly
detaches presentation, retaining all fail-fast Host checks. Production observation
was not disabled. Existing build warnings remain; no real Host/UI acceptance claimed.

Original-revision model editing and atomic model/Host submission replace the first
batch's ABA and split-write limitations. Other ordinary metadata preferences retain
snapshot-write semantics. Shared full-update generation parameters and the existing
Web profile command still have separate concurrency boundaries; this batch does not
claim every settings entry point has been converted to revision-isolated patches.

Follow-up details: `loadArchives`, `load`, and `loadAsync` obtain settings+revision in
one metadata receipt. Successful configuration receipts advance the original owner.
Ordinary metadata receipts advance it only from an exactly matched preceding read
revision. New conversation creation returns its creation revision; existing records
cannot manufacture a new read baseline through `ensureConversation`. The new ABA
test switches A→B→A and rejects the first A snapshot, and checks execution location
changes in those same transactions. Unrelated content updates may intentionally
cause an editor conflict, requiring reload rather than silent rebase. Metadata-only
preferences retain their previously documented snapshot-write behavior.

## Actionable conflict recovery

The repository now distinguishes `ModelConfigurationConflict.staleSnapshot` from
profile validation, deleted conversation, malformed configuration and service errors.
Only a typed stale-snapshot conflict triggers an asynchronous read of that owner's
model/parameter/Host/revision projection. It does not retry the failed edit, reload
messages, replace drafts, or select the owner. Sending for that owner remains blocked
until refresh finishes. The active owner sees a refreshed/retry notice; another
conversation's display is untouched. A newer in-memory revision is never replaced
by a late commit or refresh receipt. Added an owner-switch regression with an external
winning configuration and a failed local edit, proving no replay and no B-draft change.
