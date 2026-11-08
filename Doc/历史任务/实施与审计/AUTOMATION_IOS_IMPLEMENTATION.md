> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# iOS automation execution integration

Status: source implementation and static review only; compilation, process recovery and device tests remain pending the goal-wide verification pass.

## Authority and data flow

- Scheduled sources store a typed Agent definition. Ordinary UserTask records use the separate typed `user_task` definition; metadata is not an execution specification.
- The scheduler enqueues and claims one occurrence, obtains its four stable identities from `ObserveAutomationExecution`, and passes its exact lease and definition to the ordinary device Agent driver.
- Every occurrence carries a required `definition_revision` from the committed definition; the admission lease uses the enqueued receipt's revision. Missing revisions fail decoding. Shared transactional enqueue, claim and admission reject stale definitions instead of allowing an old occurrence to inherit new permissions.
- `IOSCanonicalAgentApplication` routes this admission to `BeginAutomationAssistantTurn`. Actual device Host scope, concrete fetched context references, recommended permission policy and configured prompt are checked before admission. Scheduling does not select tools from prompt keywords.
- Process return text and diagnostics are not a result. Only `ReconcileAutomationExecution` can attach a committed exact Run seal/result to an occurrence. Scheduled history projects that receipt and its six-valued RunResult; the old arbitrary Finish command is removed.

## Recovery

The public device recovery path is now `recoverExactRun`. Permission-only reattachment is a narrower wrapper over that implementation, not a second state machine. It reconstructs the frozen provider, checks the actual current Host policy against the admitted invocation, consults the Application effect-work schedule, reconciles expired physical deliveries, and claims exact owned work before dispatch. Unknown work, missing effects and changed invocation proofs remain errors, not fresh submissions or inferred success.

Automation execution owners are keyed by canonical conversation strings, independently of the selected chat tab. A bound occurrence uses `binding.run_id`, not an assumed equality with its submission ID. Startup and scheduler wake enumerate pending occurrences with keyset pagination; an already sealed occurrence is reconciled, and a bound executable occurrence re-enters the common driver. Open input/permission waits are preserved rather than treated as runnable work. No-binding occurrences are not blindly resubmitted by recovery.

## Remaining acceptance and UI work

- Cold recovery, real Host physical-lease races, and provider reconstruction need final coordinated execution tests. Added source tests cover exact bound Run identity and the permission-only restriction; they are not a claim of process-level validation.
- Native navigation and response delivery for automation-owned permission/question waits still need a user-visible owner-aware entry point. The scheduler preserves these waits; it does not answer or cancel them.
- Pending notification receipts remain durable and unacknowledged until delivered. Cold notification rendering/retry is not completed by the execution recovery pass. Existing success-path notification ACK follows delivery, but is not proof that a person saw an OS notification.
- Current platform `ios-device` Host identity is the existing device driver identity. Cross-device ownership must continue to rely on the actual frozen Host binding, not a display name or copied declaration.

No test or build was run during this implementation slice.
