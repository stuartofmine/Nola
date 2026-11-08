> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Web unknown-effect recovery (A08)

## Ownership and API

`POST /api/chat/agent/v2/recovery` accepts only typed `inspect`, `receipt`,
and `resolve` operations. It inherits the Web server's existing remote session
and CSRF checks; it is not a public/mobile route or a generic Application RPC.
The existing loopback-local trust policy is unchanged.

All three operations validate the exact source Host, original admitted browser
client, conversation, Run and assistant-message identity against Application's
local admission and source-submission receipt. This binding remains required
after terminal sealing. Neither inspecting another client's Run nor retrieving
its decision receipt is permitted merely because its Run ID is known.

Inspection returns typed unknown-attempt fences to the authenticated browser.
They are opaque transport values, not UI labels, model context or editable
parameters. Browser choices are explicit risk retry or preserving unknown as
blocked. Browser-supplied execution-success receipts are forbidden. Application
remains responsible for exact attempt revision, lease, configuration and pending
effect validation. The Web layer cannot manufacture success or release fences.

## Durable decision and receipt protocol

1. The user inspects current unknown work and explicitly chooses an action.
   Risk retry warns about duplicate side effects and requires confirmation.
2. The browser creates one complete decision (including request ID, resolution
   ID, timestamp, reason and inspected fences), serializes it once, and commits
   it using a strict-durability IndexedDB transaction before any network write.
3. The key is the exact Host/client/conversation/Run/assistant tuple. Atomic
   `add` prevents another tab from overwriting an unresolved decision. Storage
   failure blocks sending; there is no in-memory fallback.
4. Every delivery starts with a receipt query using that original decision.
   Only an absent receipt permits submitting the same decision. A lost ACK,
   accepted receipt, malformed reply or transport failure retains the original
   intent. It never allocates another request or silently replaces its fences.
5. The server's Application request identity is SHA-256 of a versioned domain,
   exact owner and browser request ID. Payload is deliberately excluded: changing
   a reason, timestamp or fence must cause an identity conflict, not a new write.
6. A plain CommandReceipt lookup has no payload digest. When a receipt exists,
   the server passes the same ID and caller's original command through the
   received-command admission path. Its Existing branch atomically verifies
   payload and replays without executing the command again. Service results must
   also equal the canonical stored receipt; an ephemeral rejection cannot be
   treated as durable completion. An absent receipt-only query submits nothing.
7. Only a verified definitive receipt retires the exact persisted decision.
   Same-ID/different-payload failure retains it. Browser reload discovers stored
   decisions for the selected conversation, including sealed historical Runs.

## Continuation and presentation

`retry_authorized` uses the existing exact-Run resume mechanism, never a new
user submission or replacement Run. `unknown_preserved_blocked` explicitly reads
the canonical source observation and requires its terminal seal. No UI success
or local terminal event substitutes for that seal.

The recovery panel is contextual, not a startup dialog. Each instance is keyed
by selection generation and Run; late replies cannot resume a newly selected
conversation. Historical saved decisions can reconcile even without an active
worker. They do not automatically start execution on page load.

The component follows existing Svelte styling and localization for all seven
Web locales. Native buttons, labelled controls, wrapping actions, distinct risk
confirmation and inline status implement the accessibility/feedback principles
reviewed through the apple-design skill. No visual, contrast or browser-runtime
verification is claimed at this source-only stage.

## Verification boundaries

Added source tests cover narrow API decoding, source owner/client isolation,
Application-backed durable conflict receipts after reopening storage, and
same-ID changed-time/reason/fence refusal. TypeScript source tests cover frozen
payload delivery, receipt-first retry, lost ACK, accepted receipts, malformed
owner/results and competing intent creation.

Per the current implementation phase, none of these tests, builds or browser
checks have run. Final acceptance must include real service-backed successful
risk retry and blocked seal, IndexedDB reload/two-tab behavior, network loss,
session/CSRF denial and switching conversations during recovery. The current
Application-backed Web fixture exercises a missing-attempt conflict, not a
successful unknown-attempt resolution; shared Application success tests do not
replace that outstanding vertical acceptance.
